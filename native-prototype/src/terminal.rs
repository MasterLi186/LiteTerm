use std::io::{Read, Write};
use std::sync::{mpsc, Arc, Mutex, OnceLock};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use alacritty_terminal::event::{Event, EventListener};
use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::index::{Column, Line, Point};
use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::term::Config as TermConfig;
use alacritty_terminal::term::{Term, TermMode};
use alacritty_terminal::vte::ansi::StdSyncHandler;
use portable_pty::{native_pty_system, CommandBuilder, MasterPty, PtySize};

use crate::bash_integration::{
    is_bash_path, LocalBashRuntime, MarkerDecoder, MarkerKind, RemoteBashRuntime,
    MAX_SNAPSHOT_INPUT_BYTES,
};
use crate::smart_completion::CompletionSessionKey;

type Processor = alacritty_terminal::vte::ansi::Processor<StdSyncHandler>;

pub(crate) fn default_shell_path() -> String {
    #[cfg(windows)]
    {
        std::env::var("COMSPEC").unwrap_or_else(|_| {
            std::env::var("SystemRoot")
                .map(|root| format!(r"{root}\System32\cmd.exe"))
                .unwrap_or_else(|_| r"C:\Windows\System32\cmd.exe".to_string())
        })
    }
    #[cfg(not(windows))]
    {
        std::env::var("SHELL").unwrap_or_else(|_| "/bin/bash".to_string())
    }
}

struct TermDimensions {
    cols: usize,
    rows: usize,
}

impl Dimensions for TermDimensions {
    fn total_lines(&self) -> usize {
        self.rows + 10000
    }
    fn screen_lines(&self) -> usize {
        self.rows
    }
    fn columns(&self) -> usize {
        self.cols
    }
}

#[derive(Clone)]
pub struct Listener {
    pty_write_tx: mpsc::Sender<String>,
}

impl EventListener for Listener {
    fn send_event(&self, event: Event) {
        if let Event::PtyWrite(text) = event {
            let _ = self.pty_write_tx.send(text);
        }
    }
}

fn spawn_writer_worker_with_protocol(
    mut writer: Box<dyn Write + Send>,
    protocol_gate: Arc<crate::zmodem::runtime::ProtocolGate>,
) -> (
    crate::zmodem::runtime::TransportWriter,
    crate::zmodem::runtime::ProtocolWriter,
) {
    let (write_tx, write_rx) =
        crate::zmodem::runtime::transport_write_channel(Arc::clone(&protocol_gate));
    let protocol_writer =
        crate::zmodem::runtime::ProtocolWriter::from_transport_writer(write_tx.clone());
    std::thread::spawn(move || {
        while let Ok(message) = write_rx.recv() {
            match message {
                crate::zmodem::runtime::TransportWrite::Normal { bytes, epoch } => {
                    // This is the FIFO exclusivity barrier: all Normal requests
                    // queued before activation are consumed but rechecked and
                    // discarded before the first Protocol request can run.
                    if protocol_gate.is_active() || epoch != protocol_gate.epoch() {
                        continue;
                    }
                    if writer
                        .write_all(&bytes)
                        .and_then(|_| writer.flush())
                        .is_err()
                    {
                        break;
                    }
                }
                crate::zmodem::runtime::TransportWrite::Protocol(request) => {
                    if !request.begin() {
                        request.complete(Err(std::io::Error::new(
                            std::io::ErrorKind::TimedOut,
                            "协议写请求在开始前已取消或超时",
                        )));
                        continue;
                    }
                    let mut offset = 0;
                    let mut result = Ok(());
                    while offset < request.bytes().len() {
                        if !request.may_continue() {
                            result = Err(std::io::Error::new(
                                std::io::ErrorKind::TimedOut,
                                "协议写请求已取消或超时",
                            ));
                            break;
                        }
                        match writer.write(&request.bytes()[offset..]) {
                            Ok(0) => {
                                result = Err(std::io::Error::new(
                                    std::io::ErrorKind::WriteZero,
                                    "终端 writer 返回零长度写入",
                                ));
                                break;
                            }
                            Ok(written) => offset += written,
                            Err(error) => {
                                result = Err(error);
                                break;
                            }
                        }
                    }
                    if result.is_ok() && request.may_continue() {
                        result = writer.flush();
                    }
                    let failed = result.is_err()
                        && result
                            .as_ref()
                            .is_err_and(|error| error.kind() != std::io::ErrorKind::TimedOut);
                    request.complete(result);
                    if failed {
                        break;
                    }
                }
            }
        }
    });
    (write_tx, protocol_writer)
}

fn spawn_writer_worker(writer: Box<dyn Write + Send>) -> crate::zmodem::runtime::TransportWriter {
    let (write_tx, _protocol_writer) = spawn_writer_worker_with_protocol(
        writer,
        Arc::new(crate::zmodem::runtime::ProtocolGate::new()),
    );
    write_tx
}

#[cfg(test)]
pub(crate) struct TestTransportCapture {
    receiver: mpsc::Receiver<crate::zmodem::runtime::TransportWrite>,
}

#[cfg(test)]
impl TestTransportCapture {
    pub(crate) fn try_recv(&self) -> Result<Vec<u8>, mpsc::TryRecvError> {
        loop {
            match self.receiver.try_recv()? {
                crate::zmodem::runtime::TransportWrite::Normal { bytes, .. } => return Ok(bytes),
                crate::zmodem::runtime::TransportWrite::Protocol(request) => {
                    request.complete(Err(std::io::Error::new(
                        std::io::ErrorKind::Unsupported,
                        "测试 capture 不处理协议写",
                    )));
                }
            }
        }
    }

    pub(crate) fn recv_timeout(
        &self,
        timeout: Duration,
    ) -> Result<Vec<u8>, mpsc::RecvTimeoutError> {
        let deadline = Instant::now() + timeout;
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            match self.receiver.recv_timeout(remaining)? {
                crate::zmodem::runtime::TransportWrite::Normal { bytes, .. } => return Ok(bytes),
                crate::zmodem::runtime::TransportWrite::Protocol(request) => {
                    request.complete(Err(std::io::Error::new(
                        std::io::ErrorKind::Unsupported,
                        "测试 capture 不处理协议写",
                    )));
                }
            }
        }
    }
}

#[cfg(test)]
fn test_transport_capture() -> (
    crate::zmodem::runtime::TransportWriter,
    TestTransportCapture,
) {
    let gate = Arc::new(crate::zmodem::runtime::ProtocolGate::new());
    let (writer, receiver) = crate::zmodem::runtime::transport_write_channel(gate);
    (writer, TestTransportCapture { receiver })
}

#[cfg(test)]
struct IsolatedTestBashEnvironment {
    home: std::path::PathBuf,
    histfile: std::path::PathBuf,
    inputrc: std::path::PathBuf,
    bash_env: std::path::PathBuf,
}

#[cfg(test)]
fn isolated_test_bash_environment(runtime: &LocalBashRuntime) -> IsolatedTestBashEnvironment {
    let home = runtime.temp_dir().to_path_buf();
    IsolatedTestBashEnvironment {
        histfile: home.join(".bash_history"),
        inputrc: home.join(".inputrc"),
        bash_env: home.join(".bash_env"),
        home,
    }
}

#[cfg(test)]
fn configure_isolated_test_bash_environment(
    command: &mut CommandBuilder,
    runtime: &LocalBashRuntime,
) {
    let environment = isolated_test_bash_environment(runtime);
    command.env("HOME", environment.home.as_path());
    command.env("HISTFILE", environment.histfile.as_path());
    command.env("INPUTRC", environment.inputrc.as_path());
    command.env("BASH_ENV", environment.bash_env.as_path());
}

type LocalChild = Box<dyn portable_pty::Child + Send + Sync>;

const MAX_SHELL_PATH_BYTES: usize = 4095;

fn validate_shell_path(shell: &str) -> Result<(), String> {
    if shell.is_empty() {
        return Err("shell 路径不能为空".into());
    }
    if shell.as_bytes().contains(&0) {
        return Err("shell 路径不能包含 NUL 字节".into());
    }
    if shell.len() > MAX_SHELL_PATH_BYTES {
        return Err(format!(
            "shell 路径过长（最大 {MAX_SHELL_PATH_BYTES} 字节）"
        ));
    }

    let path = std::path::Path::new(shell);
    if !path.is_absolute() {
        return Err("shell 路径必须是绝对路径".into());
    }
    let metadata = std::fs::metadata(path).map_err(|error| format!("shell 路径无效: {error}"))?;
    if !metadata.is_file() {
        return Err("shell 路径必须指向普通文件".into());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if metadata.permissions().mode() & 0o111 == 0 {
            return Err("shell 文件不可执行".into());
        }
    }
    Ok(())
}

static LOCAL_CHILD_REAPER: OnceLock<Option<mpsc::Sender<LocalChild>>> = OnceLock::new();
static SERIAL_REAPER: OnceLock<Option<mpsc::Sender<JoinHandle<()>>>> = OnceLock::new();

fn local_child_reaper_sender() -> Option<&'static mpsc::Sender<LocalChild>> {
    LOCAL_CHILD_REAPER
        .get_or_init(|| {
            let (sender, receiver) = mpsc::channel::<LocalChild>();
            match std::thread::Builder::new()
                .name("local-shell-reaper".into())
                .spawn(move || {
                    while let Ok(mut child) = receiver.recv() {
                        let pid = child.process_id();
                        if let Err(error) = child.kill() {
                            log::warn!(
                                "本地 shell 回收线程重试终止失败 (pid={pid:?}, error={error})"
                            );
                        }
                        if let Err(error) = child.wait() {
                            log::warn!(
                                "本地 shell 回收线程最终 wait 失败 (pid={pid:?}, error={error})"
                            );
                        }
                    }
                }) {
                Ok(_) => Some(sender),
                Err(error) => {
                    log::warn!("无法启动本地 shell 回收线程: {error}");
                    None
                }
            }
        })
        .as_ref()
}

fn enqueue_local_child_or_wait(mut child: LocalChild, sender: Option<&mpsc::Sender<LocalChild>>) {
    let pid = child.process_id();
    if let Err(error) = child.kill() {
        log::warn!("终止本地 shell 失败，仍将执行最终 wait (pid={pid:?}, error={error})");
    }

    let mut child = match sender {
        Some(sender) => match sender.send(child) {
            Ok(()) => return,
            Err(error) => {
                log::warn!("本地 shell 回收队列已断开，同步 wait (pid={pid:?})");
                error.0
            }
        },
        None => child,
    };
    if let Err(error) = child.wait() {
        log::warn!("同步 wait 本地 shell 失败 (pid={pid:?}, error={error})");
    }
}

fn terminate_local_child(mut child: LocalChild) {
    match child.try_wait() {
        Ok(Some(_)) => return,
        Ok(None) => {}
        Err(error) => {
            log::warn!("本地 shell 初次状态查询失败，继续异步终止 (error={error})");
        }
    }
    enqueue_local_child_or_wait(child, local_child_reaper_sender());
}

fn serial_reaper_sender() -> Option<&'static mpsc::Sender<JoinHandle<()>>> {
    SERIAL_REAPER
        .get_or_init(|| {
            let (sender, receiver) = mpsc::channel::<JoinHandle<()>>();
            std::thread::Builder::new()
                .name("serial-reaper".into())
                .spawn(move || {
                    while let Ok(worker) = receiver.recv() {
                        let _ = worker.join();
                    }
                })
                .map(|_| sender)
                .map_err(|error| log::warn!("无法启动串口回收线程: {error}"))
                .ok()
        })
        .as_ref()
}

fn reap_serial_worker(worker: JoinHandle<()>) {
    let worker = if let Some(sender) = serial_reaper_sender() {
        match sender.send(worker) {
            Ok(()) => return,
            Err(error) => error.0,
        }
    } else {
        worker
    };
    std::thread::spawn(move || {
        let _ = worker.join();
    });
}

pub fn shutdown_serial_handle(handle: crate::serial::SerialHandle) {
    let mut parts = handle.into_parts();
    let _ = parts.shutdown_tx.send(());
    drop(parts.reader);
    drop(parts.write_tx);
    drop(parts.io_done_rx);
    if let Some(worker) = parts.join.take() {
        reap_serial_worker(worker);
    }
}

/// SSH resize sender (optional, only for SSH connections)
type ResizeSender = Option<mpsc::SyncSender<(u16, u16)>>;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum IntegrationEvent {
    HistoryPath {
        session: CompletionSessionKey,
        path: String,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LogicalPoint {
    absolute_line: i64,
    column: usize,
}

pub struct SnapshotBase {
    prefix: String,
    anchor: LogicalPoint,
}

pub struct PromptTracking {
    session: CompletionSessionKey,
    decoder: MarkerDecoder,
    active: bool,
    anchor: Option<LogicalPoint>,
    snapshot_base: Option<SnapshotBase>,
    snapshot_requested_at: Option<Instant>,
    outstanding_snapshot_responses: u32,
    stale_snapshot_responses: u32,
}

const SNAPSHOT_RETRY_TIMEOUT: Duration = Duration::from_millis(250);

#[derive(Debug, PartialEq, Eq)]
pub enum CandidateWriteTarget {
    Local(std::path::PathBuf),
    Remote(String),
    Direct,
}

pub struct CandidateWriteRequest {
    pub session: CompletionSessionKey,
    pub target: CandidateWriteTarget,
    pub bytes: Vec<u8>,
}

pub struct TerminalState {
    term: Option<Term<Listener>>,
    writer: Option<crate::zmodem::runtime::TransportWriter>,
    zmodem_protocol_writer: Option<crate::zmodem::runtime::ProtocolWriter>,
    zmodem_input_gate: Arc<crate::zmodem::runtime::ProtocolGate>,
    pty_reader: Option<Box<dyn Read + Send>>,
    pty_master: Option<Box<dyn MasterPty + Send>>,
    local_child: Option<Box<dyn portable_pty::Child + Send + Sync>>,
    ssh_resize_tx: ResizeSender,
    ssh_shutdown_tx: Option<mpsc::Sender<()>>,
    ssh_io_done_rx: Option<mpsc::Receiver<()>>,
    serial_shutdown_tx: Option<mpsc::Sender<()>>,
    serial_io_done_rx: Option<mpsc::Receiver<crate::serial::SerialExit>>,
    serial_join: Option<JoinHandle<()>>,
    pty_write_rx: Option<mpsc::Receiver<String>>,
    cols: u16,
    rows: u16,
    pub scroll_offset: i32,
    pub local_bash_runtime: Option<LocalBashRuntime>,
    pub remote_bash_runtime: Option<RemoteBashRuntime>,
    prompt_tracking: Option<PromptTracking>,
    replay_parser: Option<Processor>,
    render_revision: u64,
}

impl Drop for TerminalState {
    fn drop(&mut self) {
        self.shutdown();
    }
}

#[cfg(test)]
pub(crate) struct CompletionHarness {
    terminal: TerminalState,
    parser: Processor,
}

#[cfg(test)]
impl CompletionHarness {
    pub fn new(cols: u16, rows: u16, session: CompletionSessionKey) -> Self {
        let mut terminal = TerminalState::new();
        terminal.init_term(cols, rows);
        terminal.prompt_tracking = Some(PromptTracking {
            decoder: MarkerDecoder::new(session.clone()),
            session,
            active: false,
            anchor: None,
            snapshot_base: None,
            snapshot_requested_at: None,
            outstanding_snapshot_responses: 0,
            stale_snapshot_responses: 0,
        });
        Self {
            terminal,
            parser: Processor::new(),
        }
    }

    pub fn feed(&mut self, bytes: &[u8]) -> Vec<IntegrationEvent> {
        self.terminal.process_pty_output(&mut self.parser, bytes)
    }

    pub fn input(&self) -> Option<String> {
        self.terminal.current_bash_input()
    }

    pub fn authenticated_prompt_active(&self) -> bool {
        self.terminal.has_authenticated_active_bash_prompt()
    }

    pub fn terminal(&self) -> &TerminalState {
        &self.terminal
    }

    pub fn submit(&mut self) -> Option<String> {
        self.terminal.take_bash_submission()
    }

    pub fn enable_local_snapshot_requests(&mut self) -> (TestTransportCapture, Vec<u8>) {
        let session = self
            .terminal
            .prompt_tracking
            .as_ref()
            .unwrap()
            .session
            .clone();
        let runtime = LocalBashRuntime::create(session).unwrap();
        let sequence = runtime.snapshot_sequence().as_bytes().to_vec();
        let (write_tx, write_rx) = test_transport_capture();
        self.terminal.writer = Some(write_tx);
        self.terminal.local_bash_runtime = Some(runtime);
        (write_rx, sequence)
    }

    pub fn enable_remote_snapshot_requests(&mut self, sequence: &[u8]) -> TestTransportCapture {
        let session = self
            .terminal
            .prompt_tracking
            .as_ref()
            .unwrap()
            .session
            .clone();
        let (write_tx, write_rx) = test_transport_capture();
        self.terminal.writer = Some(write_tx);
        self.terminal.remote_bash_runtime = Some(RemoteBashRuntime {
            session,
            bash_path: "/bin/bash".into(),
            rc_path: "/tmp/liteterm-test.rc".into(),
            candidate_path: "/tmp/liteterm-test.candidate".into(),
            widget_sequence: "\x1b[777;123~".into(),
            snapshot_sequence: String::from_utf8(sequence.to_vec()).unwrap(),
        });
        write_rx
    }

    pub fn resize(&mut self, cols: u16, rows: u16) {
        self.terminal.resize(cols, rows);
    }
}

mod read_loop;
mod state_completion;
mod state_lifecycle;
mod state_view;

#[cfg(test)]
pub use read_loop::read_loop;
pub use read_loop::read_loop_with_zmodem;

#[cfg(test)]
#[path = "terminal/tests.rs"]
mod tests;
