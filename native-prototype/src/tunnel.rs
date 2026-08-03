use std::collections::{BTreeMap, VecDeque};
use std::fmt;
use std::io::{self, Read, Write};
use std::net::{IpAddr, Ipv4Addr, Shutdown, SocketAddr, TcpListener, TcpStream};
use std::sync::mpsc;
use std::thread::{self, JoinHandle};
use std::time::Duration;

use crate::ssh::ConnectionParams;

pub type TunnelId = u64;

const LISTEN_ADDRESS: IpAddr = IpAddr::V4(Ipv4Addr::LOCALHOST);
const MAX_CLIENTS: usize = 64;
const MAX_PENDING_BYTES: usize = 256 * 1024;
const IO_CHUNK_BYTES: usize = 16 * 1024;
const SSH_WOULD_BLOCK: i32 = -37;

#[derive(Clone, PartialEq, Eq)]
pub struct TunnelSpec {
    pub connection: ConnectionParams,
    pub local_port: u16,
    pub remote_host: String,
    pub remote_port: u16,
}

impl TunnelSpec {
    pub fn validate(&self) -> Result<(), String> {
        if self.connection.host.trim().is_empty() {
            return Err("请选择 SSH 主机".to_string());
        }
        if self.connection.port == 0 {
            return Err("SSH 端口必须在 1-65535 之间".to_string());
        }
        if self.local_port == 0 {
            return Err("本地端口必须在 1-65535 之间".to_string());
        }
        if self.remote_host.trim().is_empty() {
            return Err("远端主机不能为空".to_string());
        }
        if self.remote_host.contains('\0') {
            return Err("远端主机包含无效字符".to_string());
        }
        if self.remote_port == 0 {
            return Err("远端端口必须在 1-65535 之间".to_string());
        }
        Ok(())
    }

    pub fn local_addr(&self) -> SocketAddr {
        SocketAddr::new(LISTEN_ADDRESS, self.local_port)
    }
}

impl fmt::Debug for TunnelSpec {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TunnelSpec")
            .field("connection", &self.connection)
            .field("local_addr", &self.local_addr())
            .field("remote_host", &self.remote_host)
            .field("remote_port", &self.remote_port)
            .finish()
    }
}

#[derive(Clone, PartialEq, Eq)]
pub enum TunnelStatus {
    Connecting,
    Active,
    Closing,
    Stopped,
    Failed(String),
}

impl TunnelStatus {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Connecting => "连接中",
            Self::Active => "运行中",
            Self::Closing => "关闭中",
            Self::Stopped => "已停止",
            Self::Failed(_) => "失败",
        }
    }

    pub fn error(&self) -> Option<&str> {
        match self {
            Self::Failed(error) => Some(error),
            _ => None,
        }
    }

    pub fn is_terminal(&self) -> bool {
        matches!(self, Self::Stopped | Self::Failed(_))
    }
}

impl fmt::Debug for TunnelStatus {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Connecting => formatter.write_str("Connecting"),
            Self::Active => formatter.write_str("Active"),
            Self::Closing => formatter.write_str("Closing"),
            Self::Stopped => formatter.write_str("Stopped"),
            Self::Failed(_) => formatter
                .debug_tuple("Failed")
                .field(&"<redacted>")
                .finish(),
        }
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct TunnelInfo {
    pub id: TunnelId,
    pub generation: u64,
    pub spec: TunnelSpec,
    pub status: TunnelStatus,
}

impl fmt::Debug for TunnelInfo {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TunnelInfo")
            .field("id", &self.id)
            .field("generation", &self.generation)
            .field("spec", &self.spec)
            .field("status", &self.status)
            .finish()
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct TunnelEvent {
    pub id: TunnelId,
    pub generation: u64,
    pub status: TunnelStatus,
}

impl fmt::Debug for TunnelEvent {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TunnelEvent")
            .field("id", &self.id)
            .field("generation", &self.generation)
            .field("status", &self.status)
            .finish()
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct TunnelExit {
    pub id: TunnelId,
    pub generation: u64,
    pub status: TunnelStatus,
}

impl fmt::Debug for TunnelExit {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TunnelExit")
            .field("id", &self.id)
            .field("generation", &self.generation)
            .field("status", &self.status)
            .finish()
    }
}

#[derive(Debug)]
enum WorkerCommand {
    Shutdown,
}

struct TunnelRecord {
    info: TunnelInfo,
    command_tx: mpsc::Sender<WorkerCommand>,
    worker: Option<JoinHandle<()>>,
    exited: bool,
}

impl fmt::Debug for TunnelRecord {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TunnelRecord")
            .field("info", &self.info)
            .field("worker_running", &self.worker.is_some())
            .field("exited", &self.exited)
            .finish()
    }
}

pub struct TunnelRegistry {
    next_id: TunnelId,
    next_generation: u64,
    records: BTreeMap<TunnelId, TunnelRecord>,
    event_tx: mpsc::Sender<TunnelEvent>,
    event_rx: mpsc::Receiver<TunnelEvent>,
    exit_tx: mpsc::Sender<TunnelExit>,
    exit_rx: mpsc::Receiver<TunnelExit>,
}

impl fmt::Debug for TunnelRegistry {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TunnelRegistry")
            .field("next_id", &self.next_id)
            .field("records", &self.records)
            .finish_non_exhaustive()
    }
}

impl Default for TunnelRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl TunnelRegistry {
    pub fn new() -> Self {
        let (event_tx, event_rx) = mpsc::channel();
        let (exit_tx, exit_rx) = mpsc::channel();
        Self {
            next_id: 1,
            next_generation: 1,
            records: BTreeMap::new(),
            event_tx,
            event_rx,
            exit_tx,
            exit_rx,
        }
    }

    pub fn start(&mut self, spec: TunnelSpec) -> Result<TunnelId, String> {
        spec.validate()?;
        let id = self.allocate_id();
        let generation = self.allocate_generation();
        let (command_tx, command_rx) = mpsc::channel();
        let event_tx = self.event_tx.clone();
        let exit_tx = self.exit_tx.clone();
        let worker_spec = spec.clone();
        let worker = thread::Builder::new()
            .name(format!("ssh-tunnel-{id}"))
            .spawn(move || {
                tunnel_worker(id, generation, worker_spec, command_rx, event_tx, exit_tx);
            })
            .map_err(|error| format!("启动隧道后台任务失败: {error}"))?;

        self.records.insert(
            id,
            TunnelRecord {
                info: TunnelInfo {
                    id,
                    generation,
                    spec,
                    status: TunnelStatus::Connecting,
                },
                command_tx,
                worker: Some(worker),
                exited: false,
            },
        );
        Ok(id)
    }

    pub fn close(&mut self, id: TunnelId) -> bool {
        let Some(record) = self.records.get_mut(&id) else {
            return false;
        };
        if record.info.status.is_terminal() {
            return false;
        }
        if record.info.status != TunnelStatus::Closing {
            record.info.status = TunnelStatus::Closing;
            let _ = record.command_tx.send(WorkerCommand::Shutdown);
        }
        true
    }

    pub fn close_all(&mut self) {
        let ids = self.records.keys().copied().collect::<Vec<_>>();
        for id in ids {
            self.close(id);
        }
    }

    pub fn info(&self, id: TunnelId) -> Option<&TunnelInfo> {
        self.records.get(&id).map(|record| &record.info)
    }

    pub fn infos(&self) -> Vec<TunnelInfo> {
        self.records
            .values()
            .map(|record| record.info.clone())
            .collect()
    }

    pub fn poll(&mut self) -> (Vec<TunnelEvent>, Vec<TunnelExit>) {
        let mut accepted_events = Vec::new();
        while let Ok(event) = self.event_rx.try_recv() {
            if self.apply_event(&event) {
                accepted_events.push(event);
            }
        }

        let mut accepted_exits = Vec::new();
        while let Ok(exit) = self.exit_rx.try_recv() {
            let Some(record) = self.records.get_mut(&exit.id) else {
                continue;
            };
            if record.info.generation != exit.generation {
                continue;
            }
            record.exited = true;
            if !record.info.status.is_terminal()
                && status_transition_allowed(&record.info.status, &exit.status)
            {
                record.info.status = exit.status.clone();
            }
            accepted_exits.push(exit);
        }

        self.reap_finished_workers();
        (accepted_events, accepted_exits)
    }

    pub fn remove_finished(&mut self, id: TunnelId) -> bool {
        let removable = self
            .records
            .get(&id)
            .is_some_and(|record| record.exited && record.worker.is_none());
        if removable {
            self.records.remove(&id);
        }
        removable
    }

    pub fn all_workers_finished(&self) -> bool {
        self.records.values().all(|record| record.worker.is_none())
    }

    fn allocate_id(&mut self) -> TunnelId {
        loop {
            let id = self.next_id.max(1);
            self.next_id = id.wrapping_add(1).max(1);
            if !self.records.contains_key(&id) {
                return id;
            }
        }
    }

    fn allocate_generation(&mut self) -> u64 {
        let generation = self.next_generation.max(1);
        self.next_generation = generation.wrapping_add(1).max(1);
        generation
    }

    fn apply_event(&mut self, event: &TunnelEvent) -> bool {
        let Some(record) = self.records.get_mut(&event.id) else {
            return false;
        };
        if record.info.generation != event.generation {
            return false;
        }
        if record.info.status == event.status {
            return true;
        }
        if !status_transition_allowed(&record.info.status, &event.status) {
            return false;
        }
        record.info.status = event.status.clone();
        true
    }

    fn reap_finished_workers(&mut self) {
        for record in self.records.values_mut() {
            let is_finished = record.worker.as_ref().is_some_and(JoinHandle::is_finished);
            if !is_finished {
                continue;
            }
            if let Some(worker) = record.worker.take() {
                let _ = worker.join();
            }
        }
    }
}

impl Drop for TunnelRegistry {
    fn drop(&mut self) {
        for record in self.records.values_mut() {
            if !record.info.status.is_terminal() {
                let _ = record.command_tx.send(WorkerCommand::Shutdown);
            }
        }
        let workers = self
            .records
            .values_mut()
            .filter_map(|record| record.worker.take())
            .collect::<Vec<_>>();
        if !workers.is_empty() {
            let _ = thread::Builder::new()
                .name("liteterm-tunnel-reaper".into())
                .spawn(move || {
                    for worker in workers {
                        let _ = worker.join();
                    }
                });
        }
    }
}

fn status_transition_allowed(from: &TunnelStatus, to: &TunnelStatus) -> bool {
    match from {
        TunnelStatus::Connecting => matches!(
            to,
            TunnelStatus::Active | TunnelStatus::Closing | TunnelStatus::Failed(_)
        ),
        TunnelStatus::Active => {
            matches!(to, TunnelStatus::Closing | TunnelStatus::Failed(_))
        }
        TunnelStatus::Closing => matches!(to, TunnelStatus::Stopped | TunnelStatus::Failed(_)),
        TunnelStatus::Stopped | TunnelStatus::Failed(_) => false,
    }
}

fn tunnel_worker(
    id: TunnelId,
    generation: u64,
    spec: TunnelSpec,
    command_rx: mpsc::Receiver<WorkerCommand>,
    event_tx: mpsc::Sender<TunnelEvent>,
    exit_tx: mpsc::Sender<TunnelExit>,
) {
    let status = match run_tunnel_worker(id, generation, &spec, &command_rx, &event_tx) {
        Ok(()) => TunnelStatus::Stopped,
        Err(error) => TunnelStatus::Failed(error),
    };
    let event = TunnelEvent {
        id,
        generation,
        status: status.clone(),
    };
    let _ = event_tx.send(event);
    let _ = exit_tx.send(TunnelExit {
        id,
        generation,
        status,
    });
}

fn run_tunnel_worker(
    id: TunnelId,
    generation: u64,
    spec: &TunnelSpec,
    command_rx: &mpsc::Receiver<WorkerCommand>,
    event_tx: &mpsc::Sender<TunnelEvent>,
) -> Result<(), String> {
    if shutdown_requested(command_rx) {
        send_status(event_tx, id, generation, TunnelStatus::Closing);
        return Ok(());
    }

    let connection = resolve_tunnel_connection(&spec.connection)?;
    let session = crate::ssh::connect_authenticated(&connection)?;
    if shutdown_requested(command_rx) {
        send_status(event_tx, id, generation, TunnelStatus::Closing);
        return Ok(());
    }

    let listener = TcpListener::bind(spec.local_addr())
        .map_err(|error| format!("监听 {} 失败: {error}", spec.local_addr()))?;
    listener
        .set_nonblocking(true)
        .map_err(|error| format!("设置本地监听为非阻塞失败: {error}"))?;
    session.set_blocking(false);
    send_status(event_tx, id, generation, TunnelStatus::Active);

    let mut clients = Vec::<TunnelClient>::new();
    let mut pending_opens = VecDeque::<PendingOpen>::new();
    loop {
        if shutdown_requested(command_rx) {
            send_status(event_tx, id, generation, TunnelStatus::Closing);
            return Ok(());
        }

        accept_clients(&listener, &mut pending_opens, clients.len())?;
        open_pending_channels(&session, spec, &mut pending_opens, &mut clients);

        let mut index = 0;
        while index < clients.len() {
            match clients[index].pump() {
                Ok(true) => index += 1,
                Ok(false) => {
                    clients.swap_remove(index);
                }
                Err(error) => {
                    log::warn!("SSH tunnel client closed after I/O error: {error}");
                    clients.swap_remove(index);
                }
            }
        }

        match command_rx.recv_timeout(Duration::from_millis(4)) {
            Ok(WorkerCommand::Shutdown) | Err(mpsc::RecvTimeoutError::Disconnected) => {
                send_status(event_tx, id, generation, TunnelStatus::Closing);
                return Ok(());
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
        }
    }
}

fn resolve_tunnel_connection(
    connection: &crate::ssh::ConnectionParams,
) -> Result<crate::ssh::ConnectionParams, String> {
    let mut resolved = connection.clone();
    if resolved.password.is_empty() && matches!(resolved.auth.as_str(), "keyring" | "password") {
        let entry =
            crate::keyring::KeyringEntry::new(&resolved.user, &resolved.host, resolved.port);
        resolved.password = entry
            .retrieve_password()
            .map_err(|error| format!("读取 SSH 凭据失败：{error}"))?
            .ok_or_else(|| "未找到已保存的 SSH 密码，请先连接该主机并保存密码".to_string())?;
        resolved.auth = "password".to_string();
    }
    Ok(resolved)
}

fn send_status(
    event_tx: &mpsc::Sender<TunnelEvent>,
    id: TunnelId,
    generation: u64,
    status: TunnelStatus,
) {
    let _ = event_tx.send(TunnelEvent {
        id,
        generation,
        status,
    });
}

fn shutdown_requested(command_rx: &mpsc::Receiver<WorkerCommand>) -> bool {
    match command_rx.try_recv() {
        Ok(WorkerCommand::Shutdown) | Err(mpsc::TryRecvError::Disconnected) => true,
        Err(mpsc::TryRecvError::Empty) => false,
    }
}

struct PendingOpen {
    local: TcpStream,
    origin: SocketAddr,
}

fn accept_clients(
    listener: &TcpListener,
    pending: &mut VecDeque<PendingOpen>,
    active_count: usize,
) -> Result<(), String> {
    while active_count + pending.len() < MAX_CLIENTS {
        match listener.accept() {
            Ok((local, origin)) => {
                local
                    .set_nonblocking(true)
                    .map_err(|error| format!("设置本地连接为非阻塞失败: {error}"))?;
                let _ = local.set_nodelay(true);
                pending.push_back(PendingOpen { local, origin });
            }
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => break,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(format!("接受本地连接失败: {error}")),
        }
    }
    Ok(())
}

fn open_pending_channels(
    session: &ssh2::Session,
    spec: &TunnelSpec,
    pending: &mut VecDeque<PendingOpen>,
    clients: &mut Vec<TunnelClient>,
) {
    let Some(front) = pending.front() else {
        return;
    };
    let origin_host = front.origin.ip().to_string();
    match session.channel_direct_tcpip(
        spec.remote_host.trim(),
        spec.remote_port,
        Some((&origin_host, front.origin.port())),
    ) {
        Ok(channel) => {
            let pending_open = pending.pop_front().expect("front was checked");
            clients.push(TunnelClient::new(pending_open.local, channel));
        }
        Err(error) if ssh_error_would_block(&error) => {}
        Err(error) => {
            log::warn!("SSH direct-tcpip channel failed: {error}");
            pending.pop_front();
        }
    }
}

fn ssh_error_would_block(error: &ssh2::Error) -> bool {
    matches!(
        error.code(),
        ssh2::ErrorCode::Session(code) if code == SSH_WOULD_BLOCK
    )
}

#[derive(Default)]
struct PendingBuffer {
    bytes: Vec<u8>,
    offset: usize,
}

impl fmt::Debug for PendingBuffer {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PendingBuffer")
            .field("pending_bytes", &self.len())
            .finish()
    }
}

impl PendingBuffer {
    fn len(&self) -> usize {
        self.bytes.len().saturating_sub(self.offset)
    }

    fn is_empty(&self) -> bool {
        self.len() == 0
    }

    fn append(&mut self, bytes: &[u8]) {
        if self.offset > 0 && self.offset >= self.bytes.len() / 2 {
            self.bytes.drain(..self.offset);
            self.offset = 0;
        }
        self.bytes.extend_from_slice(bytes);
    }

    fn pending(&self) -> &[u8] {
        &self.bytes[self.offset..]
    }

    fn consume(&mut self, count: usize) {
        self.offset = (self.offset + count).min(self.bytes.len());
        if self.offset == self.bytes.len() {
            self.bytes.clear();
            self.offset = 0;
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum WriteProgress {
    Empty,
    Progress(usize),
    WouldBlock,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ReadProgress {
    Progress(usize),
    Eof,
    WouldBlock,
    Backpressured,
}

fn write_pending(
    writer: &mut impl Write,
    pending: &mut PendingBuffer,
) -> io::Result<WriteProgress> {
    if pending.is_empty() {
        return Ok(WriteProgress::Empty);
    }
    match writer.write(pending.pending()) {
        Ok(0) => Err(io::Error::new(
            io::ErrorKind::WriteZero,
            "pending write returned zero",
        )),
        Ok(written) => {
            pending.consume(written);
            Ok(WriteProgress::Progress(written))
        }
        Err(error)
            if matches!(
                error.kind(),
                io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
            ) =>
        {
            Ok(WriteProgress::WouldBlock)
        }
        Err(error) => Err(error),
    }
}

fn read_into_pending(
    reader: &mut impl Read,
    pending: &mut PendingBuffer,
) -> io::Result<ReadProgress> {
    let available = MAX_PENDING_BYTES.saturating_sub(pending.len());
    if available == 0 {
        return Ok(ReadProgress::Backpressured);
    }

    let mut chunk = [0_u8; IO_CHUNK_BYTES];
    let limit = available.min(chunk.len());
    match reader.read(&mut chunk[..limit]) {
        Ok(0) => Ok(ReadProgress::Eof),
        Ok(read) => {
            pending.append(&chunk[..read]);
            Ok(ReadProgress::Progress(read))
        }
        Err(error)
            if matches!(
                error.kind(),
                io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
            ) =>
        {
            Ok(ReadProgress::WouldBlock)
        }
        Err(error) => Err(error),
    }
}

#[derive(Default)]
struct HalfCloseState {
    local_read_eof: bool,
    remote_read_eof: bool,
    remote_write_eof_sent: bool,
    local_write_shutdown: bool,
}

impl fmt::Debug for HalfCloseState {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("HalfCloseState")
            .field("local_read_eof", &self.local_read_eof)
            .field("remote_read_eof", &self.remote_read_eof)
            .field("remote_write_eof_sent", &self.remote_write_eof_sent)
            .field("local_write_shutdown", &self.local_write_shutdown)
            .finish()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct HalfCloseActions {
    send_remote_eof: bool,
    shutdown_local_write: bool,
    complete: bool,
}

impl HalfCloseState {
    fn actions(&self, to_remote_empty: bool, to_local_empty: bool) -> HalfCloseActions {
        let send_remote_eof = self.local_read_eof && to_remote_empty && !self.remote_write_eof_sent;
        let shutdown_local_write =
            self.remote_read_eof && to_local_empty && !self.local_write_shutdown;
        HalfCloseActions {
            send_remote_eof,
            shutdown_local_write,
            complete: self.local_read_eof
                && self.remote_read_eof
                && to_remote_empty
                && to_local_empty
                && self.remote_write_eof_sent
                && self.local_write_shutdown,
        }
    }
}

struct TunnelClient {
    local: TcpStream,
    channel: ssh2::Channel,
    to_remote: PendingBuffer,
    to_local: PendingBuffer,
    half_close: HalfCloseState,
}

impl TunnelClient {
    fn new(local: TcpStream, channel: ssh2::Channel) -> Self {
        Self {
            local,
            channel,
            to_remote: PendingBuffer::default(),
            to_local: PendingBuffer::default(),
            half_close: HalfCloseState::default(),
        }
    }

    fn pump(&mut self) -> Result<bool, String> {
        write_pending(&mut self.channel, &mut self.to_remote)
            .map_err(|error| format!("写入 SSH channel 失败: {error}"))?;
        write_pending(&mut self.local, &mut self.to_local)
            .map_err(|error| format!("写入本地连接失败: {error}"))?;

        if !self.half_close.local_read_eof {
            match read_into_pending(&mut self.local, &mut self.to_remote) {
                Ok(ReadProgress::Eof) => self.half_close.local_read_eof = true,
                Ok(
                    ReadProgress::Progress(_)
                    | ReadProgress::WouldBlock
                    | ReadProgress::Backpressured,
                ) => {}
                Err(error) => {
                    return Err(format!("读取本地连接失败: {error}"));
                }
            }
        }

        if !self.half_close.remote_read_eof {
            match read_into_pending(&mut self.channel, &mut self.to_local) {
                Ok(ReadProgress::Eof) => self.half_close.remote_read_eof = true,
                Ok(
                    ReadProgress::Progress(_)
                    | ReadProgress::WouldBlock
                    | ReadProgress::Backpressured,
                ) => {}
                Err(error) => {
                    return Err(format!("读取 SSH channel 失败: {error}"));
                }
            }
        }
        if self.channel.eof() {
            self.half_close.remote_read_eof = true;
        }

        self.drive_half_close()?;
        Ok(!self
            .half_close
            .actions(self.to_remote.is_empty(), self.to_local.is_empty())
            .complete)
    }

    fn drive_half_close(&mut self) -> Result<(), String> {
        let actions = self
            .half_close
            .actions(self.to_remote.is_empty(), self.to_local.is_empty());
        if actions.send_remote_eof {
            match self.channel.send_eof() {
                Ok(()) => self.half_close.remote_write_eof_sent = true,
                Err(error) if ssh_error_would_block(&error) => {}
                Err(error) => return Err(format!("发送 SSH EOF 失败: {error}")),
            }
        }
        if actions.shutdown_local_write {
            match self.local.shutdown(Shutdown::Write) {
                Ok(()) => self.half_close.local_write_shutdown = true,
                Err(error) if error.kind() == io::ErrorKind::NotConnected => {
                    self.half_close.local_write_shutdown = true;
                }
                Err(error) => return Err(format!("关闭本地写方向失败: {error}")),
            }
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "tunnel/tests.rs"]
mod tests;
