use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::{mpsc, Arc, Mutex, OnceLock};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use alacritty_terminal::event::{Event, EventListener};
use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::index::{Column, Line, Point, Side};
use alacritty_terminal::selection::{Selection, SelectionRange, SelectionType};
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

const SEMANTIC_SELECTION_DELIMITERS: &str = ",;│`|:&\"' ()[]{}<>\t（）【】「」『』《》〈〉，。；：";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TerminalSelectionKind {
    Simple,
    Block,
    Semantic,
    Lines,
}

impl From<TerminalSelectionKind> for SelectionType {
    fn from(kind: TerminalSelectionKind) -> Self {
        match kind {
            TerminalSelectionKind::Simple => Self::Simple,
            TerminalSelectionKind::Block => Self::Block,
            TerminalSelectionKind::Semantic => Self::Semantic,
            TerminalSelectionKind::Lines => Self::Lines,
        }
    }
}

pub(crate) fn default_shell_path() -> String {
    #[cfg(windows)]
    {
        local_shell_paths()
            .into_iter()
            .next()
            .unwrap_or_else(|| {
                std::env::var_os("SystemRoot")
                    .map(PathBuf::from)
                    .map(|root| root.join("System32/cmd.exe"))
                    .unwrap_or_else(|| PathBuf::from(r"C:\Windows\System32\cmd.exe"))
            })
            .to_string_lossy()
            .into_owned()
    }
    #[cfg(not(windows))]
    {
        std::env::var("SHELL").unwrap_or_else(|_| "/bin/bash".to_string())
    }
}

/// Return usable local shells in preference order. Windows installations may
/// have multiple interactive shells, so Git Bash is preferred, followed by
/// PowerShell 7, Windows PowerShell and finally cmd.exe.
pub(crate) fn local_shell_paths() -> Vec<PathBuf> {
    #[cfg(windows)]
    {
        let mut shells = Vec::new();
        let mut add = |path: PathBuf| {
            if !path.is_file() {
                return;
            }
            let key = path.to_string_lossy().to_ascii_lowercase();
            if shells
                .iter()
                .all(|existing: &PathBuf| existing.to_string_lossy().to_ascii_lowercase() != key)
            {
                shells.push(path);
            }
        };

        let mut git_bash = Vec::new();
        for variable in ["ProgramW6432", "ProgramFiles", "ProgramFiles(x86)"] {
            if let Some(root) = std::env::var_os(variable).map(PathBuf::from) {
                git_bash.push(root.join("Git/bin/bash.exe"));
                git_bash.push(root.join("Git/usr/bin/bash.exe"));
            }
        }
        if let Some(local_app_data) = std::env::var_os("LOCALAPPDATA").map(PathBuf::from) {
            git_bash.push(local_app_data.join("Programs/Git/bin/bash.exe"));
        }
        if let Some(git) = find_windows_executable("git.exe") {
            if let Some(git_root) = git.parent().and_then(|path| path.parent()) {
                git_bash.push(git_root.join("bin/bash.exe"));
                git_bash.push(git_root.join("usr/bin/bash.exe"));
            }
        }
        if let Some(path) = find_windows_executable("bash.exe") {
            if path.components().any(|component| {
                component
                    .as_os_str()
                    .to_string_lossy()
                    .eq_ignore_ascii_case("git")
            }) {
                git_bash.push(path);
            }
        }
        for path in git_bash {
            add(path);
        }

        if let Some(path) = find_windows_executable("pwsh.exe") {
            add(path);
        }
        if let Some(root) = std::env::var_os("ProgramFiles").map(PathBuf::from) {
            add(root.join("PowerShell/7/pwsh.exe"));
        }
        if let Some(root) = std::env::var_os("SystemRoot").map(PathBuf::from) {
            add(root.join("System32/WindowsPowerShell/v1.0/powershell.exe"));
        }
        if let Some(path) = find_windows_executable("powershell.exe") {
            add(path);
        }

        if let Some(path) = std::env::var_os("COMSPEC").map(PathBuf::from) {
            add(path);
        }
        if let Some(root) = std::env::var_os("SystemRoot").map(PathBuf::from) {
            add(root.join("System32/cmd.exe"));
        }

        return shells;
    }

    #[cfg(not(windows))]
    {
        vec![PathBuf::from(
            std::env::var("SHELL").unwrap_or_else(|_| "/bin/bash".to_string()),
        )]
    }
}

#[cfg(windows)]
fn find_windows_executable(name: &str) -> Option<PathBuf> {
    std::env::var_os("PATH")
        .into_iter()
        .flat_map(|path| std::env::split_paths(&path).collect::<Vec<_>>())
        .map(|directory| directory.join(name))
        .find(|path| path.is_file())
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
    terminal_reply_sink: Arc<Mutex<TerminalReplySink>>,
}

const MAX_PENDING_TERMINAL_REPLIES: usize = 32;
const MAX_PENDING_TERMINAL_REPLY_BYTES: usize = 4 * 1024;

#[derive(Default)]
enum AltScreenSequenceState {
    #[default]
    Ground,
    Escape,
    Csi(Vec<u8>),
    String,
    StringEscape,
}

impl AltScreenSequenceState {
    fn is_alt_screen_enable(params: &[u8]) -> bool {
        params
            .strip_prefix(b"?")
            .unwrap_or(params)
            .split(|byte| *byte == b';')
            .any(|parameter| matches!(parameter, b"47" | b"1047" | b"1049"))
    }

    fn advance(self, byte: u8, alt_screen_seen: &mut bool) -> Self {
        match self {
            Self::Ground => match byte {
                0x1b => Self::Escape,
                0x9b => Self::Csi(Vec::new()),
                0x9d | 0x90 | 0x98 | 0x9e | 0x9f => Self::String,
                _ => Self::Ground,
            },
            Self::Escape => match byte {
                b'[' => Self::Csi(Vec::new()),
                b']' | b'P' | b'X' | b'^' | b'_' => Self::String,
                b'\x1b' => Self::Escape,
                0x9b => Self::Csi(Vec::new()),
                _ => Self::Ground,
            },
            Self::Csi(mut params) => {
                if (0x40..=0x7e).contains(&byte) {
                    if byte == b'h' && Self::is_alt_screen_enable(&params) {
                        *alt_screen_seen = true;
                    }
                    Self::Ground
                } else if byte == 0x1b {
                    Self::Escape
                } else {
                    if params.len() < 64 {
                        params.push(byte);
                    }
                    Self::Csi(params)
                }
            }
            Self::String => match byte {
                b'\x07' | 0x9c => Self::Ground,
                0x1b => Self::StringEscape,
                _ => Self::String,
            },
            Self::StringEscape => match byte {
                b'\\' | 0x9c => Self::Ground,
                0x1b => Self::StringEscape,
                _ => Self::String,
            },
        }
    }
}

struct TerminalReplySink {
    writer: Option<crate::zmodem::runtime::TerminalReplyWriter>,
    pending: Vec<String>,
    pending_bytes: usize,
    deferred: Vec<String>,
    deferred_bytes: usize,
    defer_disallowed: bool,
    deferred_alt_screen: bool,
    alt_screen_sequence_state: AltScreenSequenceState,
    allowed: bool,
}

impl Default for TerminalReplySink {
    fn default() -> Self {
        Self {
            writer: None,
            pending: Vec::new(),
            pending_bytes: 0,
            deferred: Vec::new(),
            deferred_bytes: 0,
            defer_disallowed: false,
            deferred_alt_screen: false,
            alt_screen_sequence_state: AltScreenSequenceState::default(),
            allowed: true,
        }
    }
}

impl TerminalReplySink {
    fn push_pending(&mut self, text: String) {
        if self.pending.len() >= MAX_PENDING_TERMINAL_REPLIES
            || self.pending_bytes.saturating_add(text.len()) > MAX_PENDING_TERMINAL_REPLY_BYTES
        {
            return;
        }
        self.pending_bytes += text.len();
        self.pending.push(text);
    }

    fn discard_pending(&mut self) {
        self.pending.clear();
        self.pending_bytes = 0;
    }

    fn discard_deferred(&mut self) {
        self.deferred.clear();
        self.deferred_bytes = 0;
        self.defer_disallowed = false;
        self.deferred_alt_screen = false;
        self.alt_screen_sequence_state = AltScreenSequenceState::default();
    }

    fn begin_deferred_batch(&mut self) {
        self.defer_disallowed = true;
        self.deferred.clear();
        self.deferred_bytes = 0;
        self.deferred_alt_screen = false;
    }

    fn observe_input(&mut self, data: &[u8]) {
        for &byte in data {
            let state = std::mem::take(&mut self.alt_screen_sequence_state);
            self.alt_screen_sequence_state = state.advance(byte, &mut self.deferred_alt_screen);
        }
    }

    fn push_deferred(&mut self, text: String) {
        if self.deferred.len() >= MAX_PENDING_TERMINAL_REPLIES
            || self.deferred_bytes.saturating_add(text.len()) > MAX_PENDING_TERMINAL_REPLY_BYTES
        {
            return;
        }
        self.deferred_bytes += text.len();
        self.deferred.push(text);
    }

    fn finish_deferred_batch(
        &mut self,
        allow: bool,
    ) -> Option<(crate::zmodem::runtime::TerminalReplyWriter, Vec<String>)> {
        self.defer_disallowed = false;
        self.deferred_bytes = 0;
        let allow = allow || self.deferred_alt_screen;
        self.deferred_alt_screen = false;
        if !allow {
            self.deferred.clear();
            return None;
        }
        let replies = std::mem::take(&mut self.deferred);
        self.writer.clone().map(|writer| (writer, replies))
    }

    fn take_pending(&mut self) -> Vec<String> {
        self.pending_bytes = 0;
        std::mem::take(&mut self.pending)
    }
}

impl EventListener for Listener {
    fn send_event(&self, event: Event) {
        if let Event::PtyWrite(text) = event {
            let writer = {
                let mut sink = self.terminal_reply_sink.lock().unwrap();
                if !sink.allowed {
                    if sink.defer_disallowed {
                        sink.push_deferred(text);
                        return;
                    }
                    log::debug!("忽略 canonical PTY 输出中的终端协议查询");
                    return;
                }
                let Some(writer) = sink.writer.clone() else {
                    sink.push_pending(text);
                    return;
                };
                writer
            };
            enqueue_terminal_reply(&writer, &text);
        }
    }
}

fn enqueue_terminal_reply(writer: &crate::zmodem::runtime::TerminalReplyWriter, text: &str) {
    if let Err(error) = writer.try_enqueue(
        text.as_bytes(),
        crate::zmodem::runtime::DEFAULT_TERMINAL_REPLY_WRITE_TIMEOUT,
    ) {
        if error.kind() == std::io::ErrorKind::WouldBlock {
            log::debug!("终端协议应答队列已满，丢弃过载应答");
        } else {
            log::warn!("终端协议应答写回失败: {error}");
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
                crate::zmodem::runtime::TransportWrite::TerminalReply(request) => {
                    if !request.begin() {
                        request.complete(Err(std::io::Error::new(
                            std::io::ErrorKind::TimedOut,
                            "终端应答在开始写入前已取消或超时",
                        )));
                        continue;
                    }
                    let mut offset = 0;
                    let mut result = Ok(());
                    while offset < request.bytes().len() {
                        if !request.may_continue() {
                            result = Err(std::io::Error::new(
                                std::io::ErrorKind::TimedOut,
                                "终端应答写入已取消或超时",
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
                            Ok(written) => {
                                request.mark_progress();
                                offset += written;
                            }
                            Err(error) => {
                                result = Err(error);
                                break;
                            }
                        }
                    }
                    if result.is_ok() {
                        if request.may_continue() {
                            result = writer.flush();
                            if result.is_ok() && !request.may_continue() {
                                result = Err(std::io::Error::new(
                                    std::io::ErrorKind::TimedOut,
                                    "终端应答 flush 后超过硬截止时间",
                                ));
                            }
                        } else {
                            result = Err(std::io::Error::new(
                                std::io::ErrorKind::TimedOut,
                                "终端应答在完成前超过硬截止时间",
                            ));
                        }
                    }
                    let failed = result.is_err()
                        && (request.requires_transport_shutdown()
                            || result
                                .as_ref()
                                .is_err_and(|error| error.kind() != std::io::ErrorKind::TimedOut));
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
    receiver: mpsc::Receiver<Vec<u8>>,
}

#[cfg(test)]
impl TestTransportCapture {
    pub(crate) fn try_recv(&self) -> Result<Vec<u8>, mpsc::TryRecvError> {
        match self.receiver.recv_timeout(Duration::from_millis(50)) {
            Ok(bytes) => Ok(bytes),
            Err(mpsc::RecvTimeoutError::Timeout) => Err(mpsc::TryRecvError::Empty),
            Err(mpsc::RecvTimeoutError::Disconnected) => Err(mpsc::TryRecvError::Disconnected),
        }
    }

    pub(crate) fn recv_timeout(
        &self,
        timeout: Duration,
    ) -> Result<Vec<u8>, mpsc::RecvTimeoutError> {
        self.receiver.recv_timeout(timeout)
    }
}

#[cfg(test)]
fn test_transport_capture() -> (
    crate::zmodem::runtime::TransportWriter,
    TestTransportCapture,
) {
    let gate = Arc::new(crate::zmodem::runtime::ProtocolGate::new());
    let (writer, receiver) = crate::zmodem::runtime::transport_write_channel(gate);
    let (capture_tx, capture_rx) = mpsc::channel();
    std::thread::spawn(move || {
        while let Ok(message) = receiver.recv() {
            match message {
                crate::zmodem::runtime::TransportWrite::Normal { bytes, .. } => {
                    let _ = capture_tx.send(bytes);
                }
                crate::zmodem::runtime::TransportWrite::Protocol(request) => {
                    request.complete(Err(std::io::Error::new(
                        std::io::ErrorKind::Unsupported,
                        "测试 capture 不处理 ZMODEM 协议写",
                    )));
                }
                crate::zmodem::runtime::TransportWrite::TerminalReply(request) => {
                    if !request.begin() {
                        request.complete(Err(std::io::Error::new(
                            std::io::ErrorKind::TimedOut,
                            "测试终端应答写入已取消",
                        )));
                        continue;
                    }
                    let bytes = request.bytes().to_vec();
                    let result = capture_tx.send(bytes).map_err(|_| {
                        std::io::Error::new(
                            std::io::ErrorKind::BrokenPipe,
                            "测试终端应答 capture 已关闭",
                        )
                    });
                    request.complete(result);
                }
            }
        }
    });
    (
        writer,
        TestTransportCapture {
            receiver: capture_rx,
        },
    )
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
const MAX_COMPAT_PRIMARY_REPLY_OUTPUT_BYTES: usize = 64 * 1024;
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
    terminal_reply_sink: Option<Arc<Mutex<TerminalReplySink>>>,
    cols: u16,
    rows: u16,
    pub scroll_offset: i32,
    pub local_bash_runtime: Option<LocalBashRuntime>,
    pub remote_bash_runtime: Option<RemoteBashRuntime>,
    prompt_tracking: Option<PromptTracking>,
    replay_parser: Option<Processor>,
    render_revision: u64,
    output_bytes_since_user_input: usize,
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
