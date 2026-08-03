use std::collections::HashMap;
use std::{
    fmt,
    io::{self, Read},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, RecvTimeoutError, Sender},
        Arc,
    },
    thread,
    time::{Duration, Instant},
};

use crate::monitor::{
    format_bytes, format_uptime, normalize_process_start_time, DiskItem, MonitorData, MonitorKey,
    NetIfaceInfo, ProcessAncestor, ProcessDetail, ProcessEnvironment, ProcessIdentity, ProcessInfo,
    ProcessMemoryMetric, ProcessStats,
};
use crate::network_detail::{
    read_network_detail_bounded, NetworkDetailSnapshot, NETWORK_DETAIL_COMMAND,
};

pub const REMOTE_SNAPSHOT_COMMAND: &str = "LC_ALL=C; export LC_ALL; printf '%s\\n' STAT; cat /proc/stat; printf '%s\\n' MEM; cat /proc/meminfo; printf '%s\\n' DISK; df -Pk; printf '%s\\n' NETDEFAULT; iface=$(ip -o route show default 2>/dev/null | awk 'NR==1 {print $5}'); if [ -z \"$iface\" ]; then for link in /sys/class/net/*; do name=${link##*/}; [ \"$name\" = lo ] && continue; [ \"$(cat \"$link/carrier\" 2>/dev/null)\" = 1 ] && { iface=$name; break; }; done; fi; printf '%s\\n' \"$iface\"; printf '%s\\n' NET; cat /proc/net/dev; printf '%s\\n' LOAD; cat /proc/loadavg; printf '%s\\n' UPTIME; cat /proc/uptime; printf '%s\\n' PS; ps -eo pid=,user=,stat=,rss=,pcpu=,comm=,lstart=,args= --sort=-pcpu | head -n 100; printf '%s\\n' PSANON; for pid in $(ps -eo pid= --sort=-pcpu | head -n 100); do while read key value rest; do if [ \"$key\" = \"RssAnon:\" ]; then printf '%s %s\\n' \"$pid\" \"$value\"; break; fi; done < \"/proc/$pid/status\" 2>/dev/null; done; printf '%s\\n' PSSTATS; ps h -eo stat= | cut -c1 | sort | uniq -c; printf '%s\\n' CPUINFO; (grep -m1 -E '^(model name|Hardware|Processor)[[:space:]]*:' /proc/cpuinfo || true); printf '%s\\n' END";

pub(crate) const MAX_SNAPSHOT_BYTES: usize = 2 * 1024 * 1024;
pub(crate) const MAX_PROCESS_DETAIL_BYTES: usize = 512 * 1024;
const MAX_DETAIL_FIELD_BYTES: usize = 16 * 1024;
const MAX_STATUS_BYTES: usize = 64 * 1024;
const MAX_PSS_BYTES: usize = 256;
const MAX_ENVIRONMENT_BYTES: usize = 64 * 1024;
const MAX_ENVIRONMENT_ENTRIES: usize = 256;
const MAX_ANCESTORS: usize = 50;
const MAX_ANCESTOR_COMMAND_BYTES: usize = 512;

pub(crate) enum RemoteMonitorCommand {
    Refresh,
    FetchProcessDetail {
        requester: String,
        request_id: u64,
        pid: u32,
    },
    FetchNetworkDetail {
        requester: String,
        request_id: u64,
    },
    Shutdown,
}

pub(crate) enum RemoteMonitorEvent {
    Update {
        key: MonitorKey,
        generation: u64,
        data: Box<MonitorData>,
    },
    Failed {
        key: MonitorKey,
        generation: u64,
        error: String,
    },
    ProcessDetail {
        key: MonitorKey,
        generation: u64,
        requester: String,
        request_id: u64,
        result: Result<Box<ProcessDetail>, String>,
    },
    NetworkDetail {
        key: MonitorKey,
        generation: u64,
        requester: String,
        request_id: u64,
        result: Result<Box<NetworkDetailSnapshot>, String>,
    },
}

impl fmt::Debug for RemoteMonitorEvent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Update {
                key, generation, ..
            } => f
                .debug_struct("RemoteMonitorEvent::Update")
                .field("key", key)
                .field("generation", generation)
                .finish(),
            Self::Failed {
                key, generation, ..
            } => f
                .debug_struct("RemoteMonitorEvent::Failed")
                .field("key", key)
                .field("generation", generation)
                .finish(),
            Self::ProcessDetail {
                key,
                generation,
                request_id,
                result,
                ..
            } => f
                .debug_struct("RemoteMonitorEvent::ProcessDetail")
                .field("key", key)
                .field("generation", generation)
                .field("request_id", request_id)
                .field("result", &if result.is_ok() { "Ok" } else { "Err" })
                .finish(),
            Self::NetworkDetail {
                key,
                generation,
                request_id,
                result,
                ..
            } => f
                .debug_struct("RemoteMonitorEvent::NetworkDetail")
                .field("key", key)
                .field("generation", generation)
                .field("request_id", request_id)
                .field("result", &if result.is_ok() { "Ok" } else { "Err" })
                .finish(),
        }
    }
}

pub(crate) struct RemoteMonitorHandle {
    generation: u64,
    tx: Sender<RemoteMonitorCommand>,
    refresh_pending: Arc<AtomicBool>,
    #[cfg(test)]
    done_rx: Option<Receiver<()>>,
}

impl RemoteMonitorHandle {
    pub(crate) fn generation(&self) -> u64 {
        self.generation
    }

    pub(crate) fn shutdown(&self) {
        let _ = self.tx.send(RemoteMonitorCommand::Shutdown);
    }

    pub(crate) fn refresh(&self) {
        if self
            .refresh_pending
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
            && self.tx.send(RemoteMonitorCommand::Refresh).is_err()
        {
            self.refresh_pending.store(false, Ordering::Release);
        }
    }

    pub(crate) fn fetch_process_detail(&self, requester: String, request_id: u64, pid: u32) {
        let _ = self.tx.send(RemoteMonitorCommand::FetchProcessDetail {
            requester,
            request_id,
            pid,
        });
    }

    pub(crate) fn fetch_network_detail(&self, requester: String, request_id: u64) {
        let _ = self.tx.send(RemoteMonitorCommand::FetchNetworkDetail {
            requester,
            request_id,
        });
    }

    #[cfg(test)]
    fn take_done_receiver_for_test(&mut self) -> Option<Receiver<()>> {
        self.done_rx.take()
    }
}

impl Drop for RemoteMonitorHandle {
    fn drop(&mut self) {
        self.shutdown();
    }
}

trait SnapshotSource {
    fn collect(&mut self) -> Result<String, String>;

    fn fetch_process_detail(&mut self, _pid: u32) -> Result<ProcessDetail, String> {
        Err("当前监控源不支持进程详情".to_string())
    }

    fn fetch_network_detail(&mut self) -> Result<NetworkDetailSnapshot, String> {
        Err("当前监控源不支持网络详情".to_string())
    }
}

struct SshSnapshotSource {
    session: ssh2::Session,
}

impl SshSnapshotSource {
    fn connect(params: &crate::ssh::ConnectionParams) -> Result<Self, String> {
        crate::ssh::connect_authenticated(params).map(|session| Self { session })
    }
}

impl SnapshotSource for SshSnapshotSource {
    fn collect(&mut self) -> Result<String, String> {
        let mut channel = self
            .session
            .channel_session()
            .map_err(|error| format!("创建远端监控通道失败: {error}"))?;
        let result = (|| {
            channel
                .exec(REMOTE_SNAPSHOT_COMMAND)
                .map_err(|error| format!("执行远端监控命令失败: {error}"))?;
            read_snapshot_bounded(&mut channel)
        })();
        let _ = channel.close();
        let _ = channel.wait_close();
        result
    }

    fn fetch_process_detail(&mut self, pid: u32) -> Result<ProcessDetail, String> {
        let command = process_detail_command(pid);
        let mut channel = self
            .session
            .channel_session()
            .map_err(|error| format!("创建远端进程详情通道失败: {error}"))?;
        let result = (|| {
            channel
                .exec(&command)
                .map_err(|error| format!("执行远端进程详情命令失败: {error}"))?;
            let output = read_process_detail_bounded(&mut channel)?;
            parse_process_detail(pid, &output)
        })();
        let _ = channel.close();
        let _ = channel.wait_close();
        result
    }

    fn fetch_network_detail(&mut self) -> Result<NetworkDetailSnapshot, String> {
        let mut channel = self
            .session
            .channel_session()
            .map_err(|error| format!("创建远端网络详情通道失败: {error}"))?;
        let result = (|| {
            channel
                .exec(NETWORK_DETAIL_COMMAND)
                .map_err(|error| format!("执行远端网络详情命令失败: {error}"))?;
            read_network_detail_bounded(&mut channel)
        })();
        let _ = channel.close();
        let _ = channel.wait_close();
        result
    }
}

fn read_snapshot_bounded(mut reader: impl Read) -> Result<String, String> {
    read_utf8_bounded(
        &mut reader,
        MAX_SNAPSHOT_BYTES,
        "读取远端监控数据失败",
        "远端监控数据超过 2MiB 限制",
        "远端监控数据不是有效 UTF-8",
    )
}

fn read_process_detail_bounded(mut reader: impl Read) -> Result<String, String> {
    read_utf8_bounded(
        &mut reader,
        MAX_PROCESS_DETAIL_BYTES,
        "读取远端进程详情失败",
        "远端进程详情超过 512KiB 限制",
        "远端进程详情不是有效 UTF-8",
    )
}

fn read_utf8_bounded(
    reader: &mut impl Read,
    limit: usize,
    read_error: &str,
    limit_error: &str,
    utf8_error: &str,
) -> Result<String, String> {
    let mut bytes = Vec::with_capacity(8 * 1024);
    let mut buffer = [0_u8; 8 * 1024];
    loop {
        let remaining = limit + 1 - bytes.len();
        let read_len = buffer.len().min(remaining);
        let count = reader
            .read(&mut buffer[..read_len])
            .map_err(|error| format!("{read_error}: {error}"))?;
        if count == 0 {
            break;
        }
        bytes.extend_from_slice(&buffer[..count]);
        if bytes.len() > limit {
            return Err(limit_error.to_string());
        }
    }
    String::from_utf8(bytes).map_err(|_| utf8_error.to_string())
}

pub(crate) trait EventSink: Send + 'static {
    fn send(&self, event: RemoteMonitorEvent) -> Result<(), ()>;
}

impl EventSink for Sender<RemoteMonitorEvent> {
    fn send(&self, event: RemoteMonitorEvent) -> Result<(), ()> {
        self.send(event).map_err(|_| ())
    }
}

impl<F> EventSink for F
where
    F: Fn(RemoteMonitorEvent) -> Result<(), ()> + Send + 'static,
{
    fn send(&self, event: RemoteMonitorEvent) -> Result<(), ()> {
        self(event)
    }
}

#[derive(Clone, Copy)]
struct WorkerTiming {
    healthy_wait: Duration,
    retry_wait: Duration,
}

impl WorkerTiming {
    fn new(healthy_wait: Duration, retry_wait: Duration) -> Self {
        Self {
            healthy_wait,
            retry_wait,
        }
    }
}

impl Default for WorkerTiming {
    fn default() -> Self {
        Self::new(Duration::from_secs(2), Duration::from_secs(5))
    }
}

pub(crate) fn start_ssh_worker_with_sink<E>(
    key: MonitorKey,
    generation: u64,
    params: crate::ssh::ConnectionParams,
    sink: E,
) -> io::Result<RemoteMonitorHandle>
where
    E: EventSink,
{
    start_worker_with_sink(
        key,
        generation,
        move || SshSnapshotSource::connect(&params),
        sink,
        WorkerTiming::default(),
    )
}

fn start_worker_with_sink<S, F, E>(
    key: MonitorKey,
    generation: u64,
    source_factory: F,
    sink: E,
    timing: WorkerTiming,
) -> io::Result<RemoteMonitorHandle>
where
    S: SnapshotSource + Send + 'static,
    F: FnMut() -> Result<S, String> + Send + 'static,
    E: EventSink,
{
    start_worker_with_optional_done(key, generation, source_factory, sink, timing, None)
}

fn start_worker_with_optional_done<S, F, E>(
    key: MonitorKey,
    generation: u64,
    source_factory: F,
    sink: E,
    timing: WorkerTiming,
    done: Option<Sender<()>>,
) -> io::Result<RemoteMonitorHandle>
where
    S: SnapshotSource + Send + 'static,
    F: FnMut() -> Result<S, String> + Send + 'static,
    E: EventSink,
{
    start_worker_with_optional_done_and_spawner(
        key,
        generation,
        source_factory,
        sink,
        timing,
        done,
        spawn_named_remote_monitor_worker,
    )
}

fn start_worker_with_optional_done_and_spawner<S, F, E, Spawn>(
    key: MonitorKey,
    generation: u64,
    mut source_factory: F,
    sink: E,
    timing: WorkerTiming,
    done: Option<Sender<()>>,
    spawn: Spawn,
) -> io::Result<RemoteMonitorHandle>
where
    S: SnapshotSource + Send + 'static,
    F: FnMut() -> Result<S, String> + Send + 'static,
    E: EventSink,
    Spawn: FnOnce(Box<dyn FnOnce() + Send>) -> io::Result<()>,
{
    let (tx, rx) = mpsc::channel();
    let refresh_pending = Arc::new(AtomicBool::new(false));
    let worker_refresh_pending = Arc::clone(&refresh_pending);
    #[cfg(test)]
    let (worker_done_tx, worker_done_rx) = mpsc::channel();
    #[cfg(test)]
    let worker_done_tx = Some(worker_done_tx);
    #[cfg(not(test))]
    let worker_done_tx = None::<Sender<()>>;
    spawn(Box::new(move || {
        run_worker(
            key,
            generation,
            &mut source_factory,
            &sink,
            timing,
            &rx,
            &worker_refresh_pending,
        );
        if let Some(done) = done {
            let _ = done.send(());
        }
        if let Some(worker_done_tx) = worker_done_tx {
            let _ = worker_done_tx.send(());
        }
    }))?;
    Ok(RemoteMonitorHandle {
        generation,
        tx,
        refresh_pending,
        #[cfg(test)]
        done_rx: Some(worker_done_rx),
    })
}

fn spawn_named_remote_monitor_worker(worker: Box<dyn FnOnce() + Send>) -> io::Result<()> {
    thread::Builder::new()
        .name("liteterm-remote-monitor".to_string())
        .spawn(worker)
        .map(|_| ())
}

fn run_worker<S, F, E>(
    key: MonitorKey,
    generation: u64,
    source_factory: &mut F,
    sink: &E,
    timing: WorkerTiming,
    commands: &Receiver<RemoteMonitorCommand>,
    refresh_pending: &AtomicBool,
) where
    S: SnapshotSource,
    F: FnMut() -> Result<S, String>,
    E: EventSink,
{
    let mut source = None;
    let mut parser = RemoteSnapshotParser::default();
    let mut previous_sample_at = None;
    let mut satisfies_pending_refresh = false;

    loop {
        if source.is_none() {
            if !dispatch_ready_commands(
                commands,
                &mut source,
                sink,
                &key,
                generation,
                &mut satisfies_pending_refresh,
            ) {
                return;
            }
            match source_factory() {
                Ok(new_source) => {
                    source = Some(new_source);
                    parser = RemoteSnapshotParser::default();
                    previous_sample_at = None;
                }
                Err(error) => {
                    if !send_failed(sink, &key, generation, error)
                        || matches!(
                            wait_for_command(
                                commands,
                                &mut source,
                                sink,
                                &key,
                                generation,
                                timing.retry_wait,
                                &mut satisfies_pending_refresh,
                            ),
                            WorkerControl::Shutdown
                        )
                    {
                        return;
                    }
                    continue;
                }
            }
        }

        if !dispatch_ready_commands(
            commands,
            &mut source,
            sink,
            &key,
            generation,
            &mut satisfies_pending_refresh,
        ) {
            return;
        }

        let now = Instant::now();
        let elapsed = previous_sample_at
            .map(|previous| now.saturating_duration_since(previous))
            .unwrap_or(Duration::ZERO);
        let result = source
            .as_mut()
            .expect("source is initialized")
            .collect()
            .and_then(|snapshot| parser.parse(&snapshot, elapsed));

        if satisfies_pending_refresh {
            refresh_pending.store(false, Ordering::Release);
            satisfies_pending_refresh = false;
        }

        match result {
            Ok(data) => {
                previous_sample_at = Some(now);
                if sink
                    .send(RemoteMonitorEvent::Update {
                        key: key.clone(),
                        generation,
                        data: Box::new(data),
                    })
                    .is_err()
                    || matches!(
                        wait_for_command(
                            commands,
                            &mut source,
                            sink,
                            &key,
                            generation,
                            timing.healthy_wait,
                            &mut satisfies_pending_refresh,
                        ),
                        WorkerControl::Shutdown
                    )
                {
                    return;
                }
            }
            Err(error) => {
                source = None;
                parser = RemoteSnapshotParser::default();
                previous_sample_at = None;
                if !send_failed(sink, &key, generation, error)
                    || matches!(
                        wait_for_command(
                            commands,
                            &mut source,
                            sink,
                            &key,
                            generation,
                            timing.retry_wait,
                            &mut satisfies_pending_refresh,
                        ),
                        WorkerControl::Shutdown
                    )
                {
                    return;
                }
            }
        }
    }
}

fn send_failed<E: EventSink>(sink: &E, key: &MonitorKey, generation: u64, error: String) -> bool {
    sink.send(RemoteMonitorEvent::Failed {
        key: key.clone(),
        generation,
        error,
    })
    .is_ok()
}

enum WorkerControl {
    Collect,
    Shutdown,
}

fn dispatch_ready_commands<S, E>(
    commands: &Receiver<RemoteMonitorCommand>,
    source: &mut Option<S>,
    sink: &E,
    key: &MonitorKey,
    generation: u64,
    satisfies_pending_refresh: &mut bool,
) -> bool
where
    S: SnapshotSource,
    E: EventSink,
{
    loop {
        match commands.try_recv() {
            Ok(RemoteMonitorCommand::Refresh) => {
                // A snapshot is already about to be collected, so adjacent
                // refresh requests are intentionally coalesced.
                *satisfies_pending_refresh = true;
            }
            Ok(RemoteMonitorCommand::FetchProcessDetail {
                requester,
                request_id,
                pid,
            }) => {
                if !send_process_detail(source, sink, key, generation, requester, request_id, pid) {
                    return false;
                }
            }
            Ok(RemoteMonitorCommand::FetchNetworkDetail {
                requester,
                request_id,
            }) => {
                if !send_network_detail(source, sink, key, generation, requester, request_id) {
                    return false;
                }
            }
            Ok(RemoteMonitorCommand::Shutdown) | Err(mpsc::TryRecvError::Disconnected) => {
                return false;
            }
            Err(mpsc::TryRecvError::Empty) => return true,
        }
    }
}

fn wait_for_command<S, E>(
    commands: &Receiver<RemoteMonitorCommand>,
    source: &mut Option<S>,
    sink: &E,
    key: &MonitorKey,
    generation: u64,
    duration: Duration,
    satisfies_pending_refresh: &mut bool,
) -> WorkerControl
where
    S: SnapshotSource,
    E: EventSink,
{
    let deadline = Instant::now() + duration;
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        match commands.recv_timeout(remaining) {
            Ok(RemoteMonitorCommand::Refresh) => {
                *satisfies_pending_refresh = true;
                return WorkerControl::Collect;
            }
            Ok(RemoteMonitorCommand::FetchProcessDetail {
                requester,
                request_id,
                pid,
            }) => {
                if !send_process_detail(source, sink, key, generation, requester, request_id, pid) {
                    return WorkerControl::Shutdown;
                }
            }
            Ok(RemoteMonitorCommand::FetchNetworkDetail {
                requester,
                request_id,
            }) => {
                if !send_network_detail(source, sink, key, generation, requester, request_id) {
                    return WorkerControl::Shutdown;
                }
            }
            Ok(RemoteMonitorCommand::Shutdown) | Err(RecvTimeoutError::Disconnected) => {
                return WorkerControl::Shutdown;
            }
            Err(RecvTimeoutError::Timeout) => return WorkerControl::Collect,
        }
    }
}

fn send_process_detail<S, E>(
    source: &mut Option<S>,
    sink: &E,
    key: &MonitorKey,
    generation: u64,
    requester: String,
    request_id: u64,
    pid: u32,
) -> bool
where
    S: SnapshotSource,
    E: EventSink,
{
    let result = source
        .as_mut()
        .ok_or_else(|| "远端监控当前未连接".to_string())
        .and_then(|source| source.fetch_process_detail(pid))
        .map(Box::new);
    sink.send(RemoteMonitorEvent::ProcessDetail {
        key: key.clone(),
        generation,
        requester,
        request_id,
        result,
    })
    .is_ok()
}

fn send_network_detail<S, E>(
    source: &mut Option<S>,
    sink: &E,
    key: &MonitorKey,
    generation: u64,
    requester: String,
    request_id: u64,
) -> bool
where
    S: SnapshotSource,
    E: EventSink,
{
    let result = source
        .as_mut()
        .ok_or_else(|| "远端监控当前未连接".to_string())
        .and_then(SnapshotSource::fetch_network_detail)
        .map(Box::new);
    sink.send(RemoteMonitorEvent::NetworkDetail {
        key: key.clone(),
        generation,
        requester,
        request_id,
        result,
    })
    .is_ok()
}

#[cfg(test)]
fn start_worker_with_sink_for_test<S, F, E>(
    key: MonitorKey,
    generation: u64,
    source_factory: F,
    sink: E,
    timing: WorkerTiming,
    done: Sender<()>,
) -> io::Result<RemoteMonitorHandle>
where
    S: SnapshotSource + Send + 'static,
    F: FnMut() -> Result<S, String> + Send + 'static,
    E: EventSink,
{
    start_worker_with_optional_done(key, generation, source_factory, sink, timing, Some(done))
}

#[cfg(test)]
fn start_worker_with_sink_for_test_and_spawner<S, F, E, Spawn>(
    key: MonitorKey,
    generation: u64,
    source_factory: F,
    sink: E,
    timing: WorkerTiming,
    done: Sender<()>,
    spawn: Spawn,
) -> io::Result<RemoteMonitorHandle>
where
    S: SnapshotSource + Send + 'static,
    F: FnMut() -> Result<S, String> + Send + 'static,
    E: EventSink,
    Spawn: FnOnce(Box<dyn FnOnce() + Send>) -> io::Result<()>,
{
    start_worker_with_optional_done_and_spawner(
        key,
        generation,
        source_factory,
        sink,
        timing,
        Some(done),
        spawn,
    )
}

mod parser;

pub use parser::RemoteSnapshotParser;
#[cfg(test)]
use parser::*;
use parser::{parse_process_detail, process_detail_command};

#[cfg(test)]
#[path = "remote_monitor/tests.rs"]
mod tests;
