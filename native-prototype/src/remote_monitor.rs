use std::collections::HashMap;
use std::{
    fmt,
    io::{self, Read},
    sync::mpsc::{self, Receiver, RecvTimeoutError, Sender},
    thread,
    time::{Duration, Instant},
};

use crate::monitor::{
    format_bytes, format_uptime, DiskItem, MonitorData, MonitorKey, NetIfaceInfo, ProcessInfo,
};

pub const REMOTE_SNAPSHOT_COMMAND: &str = "LC_ALL=C; export LC_ALL; printf '%s\\n' STAT; cat /proc/stat; printf '%s\\n' MEM; cat /proc/meminfo; printf '%s\\n' DISK; df -Pk; printf '%s\\n' NET; cat /proc/net/dev; printf '%s\\n' LOAD; cat /proc/loadavg; printf '%s\\n' UPTIME; cat /proc/uptime; printf '%s\\n' PS; ps -eo rss=,pcpu=,comm= --sort=-pcpu | head -n 24; printf '%s\\n' CPUINFO; (grep -m1 -E '^(model name|Hardware|Processor)[[:space:]]*:' /proc/cpuinfo || true); printf '%s\\n' END";

pub(crate) const MAX_SNAPSHOT_BYTES: usize = 2 * 1024 * 1024;

pub(crate) enum RemoteMonitorCommand {
    Shutdown,
    #[cfg(test)]
    Wake,
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
        }
    }
}

pub(crate) struct RemoteMonitorHandle {
    generation: u64,
    tx: Sender<RemoteMonitorCommand>,
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

    #[cfg(test)]
    fn take_done_receiver_for_test(&mut self) -> Option<Receiver<()>> {
        self.done_rx.take()
    }

    #[cfg(test)]
    fn wake(&self) {
        let _ = self.tx.send(RemoteMonitorCommand::Wake);
    }
}

impl Drop for RemoteMonitorHandle {
    fn drop(&mut self) {
        self.shutdown();
    }
}

trait SnapshotSource {
    fn collect(&mut self) -> Result<String, String>;
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
}

fn read_snapshot_bounded(mut reader: impl Read) -> Result<String, String> {
    let mut bytes = Vec::with_capacity(8 * 1024);
    let mut buffer = [0_u8; 8 * 1024];
    loop {
        let remaining = MAX_SNAPSHOT_BYTES + 1 - bytes.len();
        let read_len = buffer.len().min(remaining);
        let count = reader
            .read(&mut buffer[..read_len])
            .map_err(|error| format!("读取远端监控数据失败: {error}"))?;
        if count == 0 {
            break;
        }
        bytes.extend_from_slice(&buffer[..count]);
        if bytes.len() > MAX_SNAPSHOT_BYTES {
            return Err("远端监控数据超过 2MiB 限制".to_string());
        }
    }
    String::from_utf8(bytes).map_err(|_| "远端监控数据不是有效 UTF-8".to_string())
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
    #[cfg(test)]
    let (worker_done_tx, worker_done_rx) = mpsc::channel();
    #[cfg(test)]
    let worker_done_tx = Some(worker_done_tx);
    #[cfg(not(test))]
    let worker_done_tx = None::<Sender<()>>;
    spawn(Box::new(move || {
        run_worker(key, generation, &mut source_factory, &sink, timing, &rx);
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
) where
    S: SnapshotSource,
    F: FnMut() -> Result<S, String>,
    E: EventSink,
{
    let mut source = None;
    let mut parser = RemoteSnapshotParser::default();
    let mut previous_sample_at = None;

    loop {
        if shutdown_requested(commands) {
            return;
        }

        if source.is_none() {
            match source_factory() {
                Ok(new_source) => {
                    source = Some(new_source);
                    parser = RemoteSnapshotParser::default();
                    previous_sample_at = None;
                }
                Err(error) => {
                    if !send_failed(sink, &key, generation, error)
                        || wait_for_shutdown(commands, timing.retry_wait)
                    {
                        return;
                    }
                    continue;
                }
            }
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
                    || wait_for_shutdown(commands, timing.healthy_wait)
                {
                    return;
                }
            }
            Err(error) => {
                source = None;
                parser = RemoteSnapshotParser::default();
                previous_sample_at = None;
                if !send_failed(sink, &key, generation, error)
                    || wait_for_shutdown(commands, timing.retry_wait)
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

fn shutdown_requested(commands: &Receiver<RemoteMonitorCommand>) -> bool {
    matches!(
        commands.try_recv(),
        Ok(RemoteMonitorCommand::Shutdown) | Err(mpsc::TryRecvError::Disconnected)
    )
}

fn wait_for_shutdown(commands: &Receiver<RemoteMonitorCommand>, duration: Duration) -> bool {
    matches!(
        commands.recv_timeout(duration),
        Ok(RemoteMonitorCommand::Shutdown) | Err(RecvTimeoutError::Disconnected)
    )
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

#[derive(Default)]
pub struct RemoteSnapshotParser {
    previous_cpu: Option<(u64, u64)>,
    previous_network: HashMap<String, (u64, u64)>,
}

impl RemoteSnapshotParser {
    pub fn parse(&mut self, output: &str, elapsed: Duration) -> Result<MonitorData, String> {
        let sections = split_sections(output)?;
        let (cpu_percent, cpu_name) = (
            self.parse_cpu(
                sections
                    .get("STAT")
                    .ok_or_else(|| "远端监控数据缺少 STAT 段".to_string())?,
            )?,
            parse_cpu_name(sections.get("CPUINFO").map(String::as_str).unwrap_or("")),
        );
        let memory = parse_memory(
            sections
                .get("MEM")
                .ok_or_else(|| "远端监控数据缺少 MEM 段".to_string())?,
        )?;
        let disk_items = parse_disks(sections.get("DISK").map(String::as_str).unwrap_or(""));
        let net_interfaces = self.parse_network(
            sections.get("NET").map(String::as_str).unwrap_or(""),
            elapsed,
        );

        Ok(MonitorData {
            cpu_percent,
            cpu_name,
            memory_used: memory.used,
            memory_total: memory.total,
            memory_text: format!(
                "{} / {}",
                format_bytes(memory.used),
                format_bytes(memory.total)
            ),
            memory_percent: percent(memory.used, memory.total),
            swap_used: memory.swap_used,
            swap_total: memory.swap_total,
            swap_text: format!(
                "{} / {}",
                format_bytes(memory.swap_used),
                format_bytes(memory.swap_total)
            ),
            swap_percent: percent(memory.swap_used, memory.swap_total),
            uptime_text: parse_uptime(sections.get("UPTIME").map(String::as_str).unwrap_or("")),
            load_text: parse_load(sections.get("LOAD").map(String::as_str).unwrap_or("")),
            disk_items,
            processes: parse_processes(sections.get("PS").map(String::as_str).unwrap_or("")),
            net_interfaces,
        })
    }

    fn parse_cpu(&mut self, stat: &str) -> Result<f32, String> {
        let line = stat
            .lines()
            .find(|line| line.split_whitespace().next() == Some("cpu"))
            .ok_or_else(|| "远端监控数据缺少有效 STAT CPU 数据".to_string())?;
        let values = line
            .split_whitespace()
            .skip(1)
            .map(|field| field.parse::<u64>())
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| "远端监控数据缺少有效 STAT CPU 数据".to_string())?;
        if values.len() < 4 {
            return Err("远端监控数据缺少有效 STAT CPU 数据".to_string());
        }
        let total = values
            .iter()
            .try_fold(0_u64, |sum, value| sum.checked_add(*value))
            .ok_or_else(|| "远端监控数据缺少有效 STAT CPU 数据".to_string())?;
        let idle = values
            .get(3)
            .copied()
            .unwrap_or(0)
            .saturating_add(values.get(4).copied().unwrap_or(0));
        let cpu_percent = self
            .previous_cpu
            .map(|(previous_total, previous_idle)| {
                let total_delta = total.saturating_sub(previous_total);
                let idle_delta = idle.saturating_sub(previous_idle);
                if total_delta == 0 {
                    0.0
                } else {
                    ((total_delta.saturating_sub(idle_delta) as f64 / total_delta as f64) * 100.0)
                        as f32
                }
            })
            .unwrap_or(0.0);
        self.previous_cpu = Some((total, idle));
        Ok(cpu_percent)
    }

    fn parse_network(&mut self, network: &str, elapsed: Duration) -> Vec<NetIfaceInfo> {
        let seconds = elapsed.as_secs_f64().max(0.001);
        let mut current = HashMap::new();
        for line in network.lines() {
            let Some((name, counters)) = line.split_once(':') else {
                continue;
            };
            let name = name.trim();
            if name.is_empty() || name == "lo" {
                continue;
            }
            let fields: Vec<_> = counters.split_whitespace().collect();
            let (Some(rx), Some(tx)) = (fields.first(), fields.get(8)) else {
                continue;
            };
            let (Ok(rx), Ok(tx)) = (rx.parse::<u64>(), tx.parse::<u64>()) else {
                continue;
            };
            current.insert(name.to_string(), (rx, tx));
        }

        let mut interfaces = current
            .iter()
            .map(|(name, &(rx, tx))| {
                let (previous_rx, previous_tx) =
                    self.previous_network.get(name).copied().unwrap_or((rx, tx));
                NetIfaceInfo {
                    name: name.clone(),
                    rx_rate: (rx.saturating_sub(previous_rx) as f64 / seconds) as u64,
                    tx_rate: (tx.saturating_sub(previous_tx) as f64 / seconds) as u64,
                }
            })
            .collect::<Vec<_>>();
        interfaces.sort_by(|left, right| left.name.cmp(&right.name));
        self.previous_network = current;
        interfaces
    }
}

struct MemoryValues {
    used: u64,
    total: u64,
    swap_used: u64,
    swap_total: u64,
}

fn split_sections(output: &str) -> Result<HashMap<&'static str, String>, String> {
    let mut sections = HashMap::new();
    let mut current = None;
    let mut saw_end = false;
    for line in output.lines() {
        if let Some(marker) = section_marker(line.trim()) {
            if marker == "END" {
                saw_end = true;
                break;
            }
            current = Some(marker);
            sections.entry(marker).or_insert_with(String::new);
        } else if let Some(marker) = current {
            if marker != "END" {
                sections.entry(marker).or_default().push_str(line);
                sections.entry(marker).or_default().push('\n');
            }
        }
    }
    saw_end
        .then_some(sections)
        .ok_or_else(|| "远端监控数据缺少 END 结束标记".to_string())
}

fn section_marker(line: &str) -> Option<&'static str> {
    match line {
        "STAT" => Some("STAT"),
        "MEM" => Some("MEM"),
        "DISK" => Some("DISK"),
        "NET" => Some("NET"),
        "LOAD" => Some("LOAD"),
        "UPTIME" => Some("UPTIME"),
        "PS" => Some("PS"),
        "CPUINFO" => Some("CPUINFO"),
        "END" => Some("END"),
        _ => None,
    }
}

fn parse_memory(memory: &str) -> Result<MemoryValues, String> {
    let mut fields = HashMap::new();
    for line in memory.lines() {
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        let name = name.trim();
        if let Some(value) = value
            .split_whitespace()
            .next()
            .and_then(|value| value.parse::<u64>().ok())
        {
            if let Some(value) = value.checked_mul(1024) {
                fields.insert(name, value);
            } else if name == "MemTotal" {
                return Err("远端监控数据缺少有效 MEM 内存总量".to_string());
            }
        }
    }
    let total = fields
        .get("MemTotal")
        .copied()
        .ok_or_else(|| "远端监控数据缺少有效 MEM 内存总量".to_string())?;
    let available = fields
        .get("MemAvailable")
        .or_else(|| fields.get("MemFree"))
        .copied()
        .unwrap_or(0);
    let swap_total = fields.get("SwapTotal").copied().unwrap_or(0);
    let swap_free = fields.get("SwapFree").copied().unwrap_or(0);
    Ok(MemoryValues {
        used: total.saturating_sub(available),
        total,
        swap_used: swap_total.saturating_sub(swap_free),
        swap_total,
    })
}

fn parse_disks(disks: &str) -> Vec<DiskItem> {
    disks
        .lines()
        .skip(1)
        .filter_map(|line| {
            let fields: Vec<_> = line.split_whitespace().collect();
            if fields.len() < 6 {
                return None;
            }
            let total = fields[1].parse::<u64>().ok()?.checked_mul(1024)?;
            if total == 0 {
                return None;
            }
            let available = fields[3].parse::<u64>().ok()?.checked_mul(1024)?;
            let percent = fields[4].strip_suffix('%')?.parse::<u64>().ok()?.min(100) as u8;
            Some(DiskItem {
                mount: fields[5..].join(" "),
                avail: format_bytes(available),
                size: format_bytes(total),
                percent,
            })
        })
        .collect()
}

fn parse_uptime(uptime: &str) -> String {
    let seconds = uptime
        .split_whitespace()
        .next()
        .and_then(|value| value.parse::<f64>().ok())
        .filter(|value| value.is_finite() && *value >= 0.0)
        .unwrap_or(0.0) as u64;
    format_uptime(seconds)
}

fn parse_load(load: &str) -> String {
    let values = load
        .split_whitespace()
        .take(3)
        .map(str::parse::<f64>)
        .collect::<Result<Vec<_>, _>>();
    match values {
        Ok(values)
            if values.len() == 3
                && values
                    .iter()
                    .all(|value| value.is_finite() && *value >= 0.0) =>
        {
            format!("{:.2}, {:.2}, {:.2}", values[0], values[1], values[2])
        }
        _ => String::new(),
    }
}

fn parse_cpu_name(cpu_info: &str) -> String {
    cpu_info
        .lines()
        .find_map(|line| line.split_once(':').map(|(_, value)| value.trim()))
        .filter(|name| !name.is_empty())
        .unwrap_or("Unknown CPU")
        .to_string()
}

fn parse_processes(processes: &str) -> Vec<ProcessInfo> {
    processes
        .lines()
        .filter_map(|line| {
            let mut fields = line.split_whitespace();
            let rss = fields.next()?.parse::<u64>().ok()?.checked_mul(1024)?;
            let cpu = fields.next()?.parse::<f32>().ok()?;
            if !cpu.is_finite() || cpu < 0.0 {
                return None;
            }
            let name = fields.collect::<Vec<_>>().join(" ");
            (!name.is_empty()).then(|| ProcessInfo {
                mem_mb: format_bytes(rss),
                mem_bytes: rss,
                cpu,
                name,
            })
        })
        .take(24)
        .collect()
}

fn percent(used: u64, total: u64) -> f32 {
    if total == 0 {
        0.0
    } else {
        (used as f64 / total as f64 * 100.0) as f32
    }
}

#[cfg(test)]
mod tests {
    use std::{
        collections::VecDeque,
        io::{self, Cursor},
        sync::{
            atomic::{AtomicUsize, Ordering},
            mpsc, Arc,
        },
        time::Duration,
    };

    use crate::monitor::MonitorKey;

    use super::{
        read_snapshot_bounded, start_worker_with_sink_for_test,
        start_worker_with_sink_for_test_and_spawner, RemoteMonitorEvent, RemoteSnapshotParser,
        SnapshotSource, WorkerTiming, MAX_SNAPSHOT_BYTES, REMOTE_SNAPSHOT_COMMAND,
    };

    const SAMPLE: &str = "STAT\ncpu  100 0 50 800 50 0 0 0 0 0\ncpu0 50 0 25 400 25 0 0 0 0 0\nMEM\nMemTotal:       2097152 kB\nMemAvailable:   1073152 kB\nSwapTotal:       512000 kB\nSwapFree:        256000 kB\nDISK\nFilesystem 1024-blocks Used Available Capacity Mounted on\n/dev/sda1 1048576 524288 524288 50% /\nNET\nInter-|   Receive                                                |  Transmit\n eth0: 1000 0 0 0 0 0 0 0 2000 0 0 0 0 0 0 0\n lo: 999 0 0 0 0 0 0 0 999 0 0 0 0 0 0 0\nLOAD\n0.10 0.20 0.30 1/100 100\nUPTIME\n90060.00 0.00\nPS\n10240 12.5 /usr/bin/test process\nCPUINFO\nmodel name : Example CPU\nEND\n";

    #[test]
    fn worker_spawn_failure_returns_an_error_without_a_handle() {
        let result = start_worker_with_sink_for_test_and_spawner(
            MonitorKey::remote("alice", "alpha.example", 22),
            1,
            || -> Result<FakeSource, String> {
                Ok(FakeSource {
                    results: VecDeque::new(),
                    collects: Arc::new(AtomicUsize::new(0)),
                    drops: Arc::new(AtomicUsize::new(0)),
                })
            },
            |_| Ok(()),
            WorkerTiming::new(Duration::ZERO, Duration::ZERO),
            mpsc::channel().0,
            |_| Err(io::Error::other("spawn failed")),
        );

        assert!(result.is_err());
    }

    #[test]
    fn parses_complete_snapshot_into_monitor_shape() {
        let mut parser = RemoteSnapshotParser::default();
        let data = parser.parse(SAMPLE, Duration::from_secs(2)).unwrap();

        assert_eq!(data.cpu_percent, 0.0);
        assert_eq!(data.cpu_name, "Example CPU");
        assert_eq!(data.memory_used, 1_048_576_000);
        assert_eq!(data.memory_total, 2_147_483_648);
        assert_eq!(data.memory_text, "1000M / 2.0G");
        assert_eq!(data.memory_percent, 48.828125);
        assert_eq!(data.swap_used, 262_144_000);
        assert_eq!(data.swap_total, 524_288_000);
        assert_eq!(data.swap_text, "250M / 500M");
        assert_eq!(data.swap_percent, 50.0);
        assert_eq!(data.uptime_text, "1天1小时1分钟");
        assert_eq!(data.load_text, "0.10, 0.20, 0.30");
        assert_eq!(data.disk_items.len(), 1);
        assert_eq!(data.disk_items[0].mount, "/");
        assert_eq!(data.disk_items[0].avail, "512M");
        assert_eq!(data.disk_items[0].size, "1.0G");
        assert_eq!(data.disk_items[0].percent, 50);
        assert_eq!(data.processes[0].mem_bytes, 10_485_760);
        assert_eq!(data.processes[0].mem_mb, "10M");
        assert_eq!(data.processes[0].cpu, 12.5);
        assert_eq!(data.processes[0].name, "/usr/bin/test process");
        assert_eq!(data.net_interfaces.len(), 1);
        assert_eq!(data.net_interfaces[0].name, "eth0");
        assert_eq!(data.net_interfaces[0].rx_rate, 0);
        assert_eq!(data.net_interfaces[0].tx_rate, 0);
    }

    #[test]
    fn computes_second_sample_cpu_and_network_rates() {
        let mut parser = RemoteSnapshotParser::default();
        parser.parse(SAMPLE, Duration::from_secs(2)).unwrap();
        let second = SAMPLE
            .replace(
                "cpu  100 0 50 800 50 0 0 0 0 0",
                "cpu  120 0 60 870 50 0 0 0 0 0",
            )
            .replace("eth0: 1000", "eth0: 5000")
            .replace("0 0 0 0 2000", "0 0 0 0 10000");

        let data = parser.parse(&second, Duration::from_secs(2)).unwrap();

        assert_eq!(data.cpu_percent, 30.0);
        assert_eq!(data.net_interfaces[0].rx_rate, 2000);
        assert_eq!(data.net_interfaces[0].tx_rate, 4000);
    }

    #[test]
    fn first_network_sample_has_zero_rates() {
        let mut parser = RemoteSnapshotParser::default();
        let data = parser.parse(SAMPLE, Duration::from_millis(1)).unwrap();

        assert!(data
            .net_interfaces
            .iter()
            .all(|item| item.rx_rate == 0 && item.tx_rate == 0));
    }

    #[test]
    fn rejects_missing_required_stat_or_mem_sections() {
        let mut parser = RemoteSnapshotParser::default();
        assert!(parser
            .parse("MEM\nMemTotal: 1 kB\nEND\n", Duration::from_secs(1))
            .is_err());
        assert!(parser
            .parse("STAT\ncpu 1 0 0 1\nEND\n", Duration::from_secs(1))
            .is_err());
    }

    #[test]
    fn missing_end_rejects_snapshot_without_advancing_parser_history() {
        let mut parser = RemoteSnapshotParser::default();
        parser.parse(SAMPLE, Duration::from_secs(2)).unwrap();
        let advanced = SAMPLE
            .replace(
                "cpu  100 0 50 800 50 0 0 0 0 0",
                "cpu  120 0 60 870 50 0 0 0 0 0",
            )
            .replace("eth0: 1000", "eth0: 5000")
            .replace("0 0 0 0 2000", "0 0 0 0 10000");
        let incomplete = advanced.trim_end_matches("END\n");

        assert!(parser.parse(incomplete, Duration::from_secs(2)).is_err());
        let data = parser.parse(&advanced, Duration::from_secs(2)).unwrap();

        assert!(data.cpu_percent > 0.0);
        assert!(data
            .net_interfaces
            .iter()
            .any(|iface| iface.rx_rate > 0 || iface.tx_rate > 0));
    }

    #[test]
    fn rejects_required_sections_without_valid_keys() {
        let mut parser = RemoteSnapshotParser::default();
        assert!(parser
            .parse(
                "STAT\ncpu 1 nope 2 3\nMEM\nMemTotal: 1 kB\nEND\n",
                Duration::from_secs(1),
            )
            .is_err());
        assert!(parser
            .parse(
                "STAT\ncpu 1 0 2 3\nMEM\nMemTotal: nope kB\nEND\n",
                Duration::from_secs(1),
            )
            .is_err());
    }

    #[test]
    fn end_marker_discards_later_replacement_sections() {
        let output = format!("{SAMPLE}STAT\ncpu 1 nope 2 3\nMEM\nMemTotal: nope kB\n");

        let sections = super::split_sections(&output).unwrap();
        assert_eq!(
            sections["STAT"],
            "cpu  100 0 50 800 50 0 0 0 0 0\ncpu0 50 0 25 400 25 0 0 0 0 0\n"
        );
        assert_eq!(sections["MEM"], "MemTotal:       2097152 kB\nMemAvailable:   1073152 kB\nSwapTotal:       512000 kB\nSwapFree:        256000 kB\n");
    }

    #[test]
    fn end_marker_cannot_be_followed_by_required_sections() {
        let mut parser = RemoteSnapshotParser::default();
        let output = "END\nSTAT\ncpu 1 0 2 3\nMEM\nMemTotal: 1 kB\n";

        assert!(parser.parse(output, Duration::from_secs(1)).is_err());
    }

    #[test]
    fn malformed_optional_sections_are_safe() {
        let mut parser = RemoteSnapshotParser::default();
        let output = "STAT\ncpu 1 0 2 3\nMEM\nMemTotal: 1 kB\nDISK\nbad\nNET\neth0: nope\nLOAD\nnot load\nUPTIME\nnope\nPS\nbad\nCPUINFO\ninvalid\nEND\n";

        let data = parser.parse(output, Duration::ZERO).unwrap();
        assert_eq!(data.cpu_name, "Unknown CPU");
        assert!(data.disk_items.is_empty());
        assert!(data.processes.is_empty());
        assert!(data.net_interfaces.is_empty());
    }

    #[test]
    fn mem_free_is_used_when_mem_available_is_absent() {
        let mut parser = RemoteSnapshotParser::default();
        let output = SAMPLE.replace("MemAvailable:   1073152 kB", "MemFree:        1048576 kB");

        let data = parser.parse(&output, Duration::from_secs(1)).unwrap();
        assert_eq!(data.memory_used, 1_073_741_824);
        assert_eq!(data.memory_text, "1.0G / 2.0G");
    }

    #[test]
    fn network_interfaces_are_sorted_and_loopback_is_ignored() {
        let mut parser = RemoteSnapshotParser::default();
        let output = SAMPLE.replace(
            " lo: 999 0 0 0 0 0 0 0 999 0 0 0 0 0 0 0",
            " zeta: 1 0 0 0 0 0 0 0 1 0 0 0 0 0 0 0\n alpha: 1 0 0 0 0 0 0 0 1 0 0 0 0 0 0 0\n lo: 999 0 0 0 0 0 0 0 999 0 0 0 0 0 0 0",
        );

        let data = parser.parse(&output, Duration::from_secs(1)).unwrap();
        let names: Vec<_> = data
            .net_interfaces
            .iter()
            .map(|item| item.name.as_str())
            .collect();
        assert_eq!(names, ["alpha", "eth0", "zeta"]);
    }

    #[test]
    fn parses_space_padded_ps_rows() {
        let mut parser = RemoteSnapshotParser::default();
        let output = SAMPLE.replace(
            "10240 12.5 /usr/bin/test process",
            "  1024   3.5   worker --flag",
        );

        let data = parser.parse(&output, Duration::from_secs(1)).unwrap();
        assert_eq!(data.processes.len(), 1);
        assert_eq!(data.processes[0].name, "worker --flag");
    }

    #[test]
    fn network_counter_reset_never_overflows() {
        let mut parser = RemoteSnapshotParser::default();
        parser.parse(SAMPLE, Duration::from_secs(2)).unwrap();
        let reset = SAMPLE
            .replace("eth0: 1000", "eth0: 1")
            .replace("0 0 0 0 2000", "0 0 0 0 1");

        let data = parser.parse(&reset, Duration::from_secs(2)).unwrap();
        assert_eq!(data.net_interfaces[0].rx_rate, 0);
        assert_eq!(data.net_interfaces[0].tx_rate, 0);
    }

    #[test]
    fn cpu_counter_reset_never_overflows() {
        let mut parser = RemoteSnapshotParser::default();
        parser.parse(SAMPLE, Duration::from_secs(2)).unwrap();
        let reset = SAMPLE.replace("cpu  100 0 50 800 50 0 0 0 0 0", "cpu  1 0 1 1 0");

        let data = parser.parse(&reset, Duration::from_secs(2)).unwrap();
        assert_eq!(data.cpu_percent, 0.0);
    }

    #[test]
    fn uptime_rejects_non_finite_and_negative_values() {
        for value in ["NaN", "+inf", "-inf", "-1.0"] {
            assert_eq!(super::parse_uptime(value), "0分钟");
        }
    }

    #[test]
    fn load_rejects_non_finite_and_negative_values() {
        for load in ["NaN 0.2 0.3", "+inf 0.2 0.3", "-1.0 0.2 0.3"] {
            assert_eq!(super::parse_load(load), "");
        }
    }

    #[test]
    fn process_parser_rejects_invalid_cpu_and_overflowing_rss() {
        let output = format!(
            "1 NaN nan\n1 inf infinite\n1 -1 negative\n{} 1 overflow\n1024 2.5 valid\n",
            u64::MAX
        );

        let processes = super::parse_processes(&output);
        assert_eq!(processes.len(), 1);
        assert_eq!(processes[0].name, "valid");
        assert_eq!(processes[0].mem_bytes, 1_048_576);
    }

    #[test]
    fn memory_parser_rejects_total_overflow_and_ignores_optional_overflow() {
        let total_overflow = format!("MemTotal: {} kB\n", u64::MAX);
        assert!(super::parse_memory(&total_overflow).is_err());

        let optional_overflow = format!("MemTotal: 1 kB\nMemAvailable: {} kB\n", u64::MAX);
        let memory = super::parse_memory(&optional_overflow).unwrap();
        assert_eq!(memory.used, 1024);
    }

    #[test]
    fn disk_parser_rejects_overflow_and_clamps_percentages() {
        let disks = format!(
            "Filesystem 1024-blocks Used Available Capacity Mounted on\n/dev/overflow {} 0 1 1% /overflow\n/dev/avail-overflow 100 0 {} 1% /avail-overflow\n/dev/one 100 0 50 101% /one\n/dev/two 100 0 50 999% /two\n/dev/invalid 100 0 50 nope /invalid\n/dev/negative 100 0 50 -1% /negative\n",
            u64::MAX
            , u64::MAX
        );

        let parsed = super::parse_disks(&disks);
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].mount, "/one");
        assert_eq!(parsed[0].percent, 100);
        assert_eq!(parsed[1].mount, "/two");
        assert_eq!(parsed[1].percent, 100);
    }

    #[test]
    fn cpu_parser_rejects_aggregate_counter_overflow() {
        let mut parser = RemoteSnapshotParser::default();
        let output = format!("STAT\ncpu {} 1 0 0\nMEM\nMemTotal: 1 kB\nEND\n", u64::MAX);

        assert!(parser.parse(&output, Duration::from_secs(1)).is_err());
    }

    #[test]
    fn reappearing_network_interface_starts_at_zero_rate() {
        let mut parser = RemoteSnapshotParser::default();
        parser.parse(SAMPLE, Duration::from_secs(2)).unwrap();
        let absent = SAMPLE.replacen(" eth0: 1000 0 0 0 0 0 0 0 2000 0 0 0 0 0 0 0\n", "", 1);
        parser.parse(&absent, Duration::from_secs(2)).unwrap();

        let data = parser.parse(SAMPLE, Duration::from_secs(2)).unwrap();
        assert_eq!(data.net_interfaces[0].name, "eth0");
        assert_eq!(data.net_interfaces[0].rx_rate, 0);
        assert_eq!(data.net_interfaces[0].tx_rate, 0);
    }

    #[test]
    fn command_is_fixed_and_contains_all_markers() {
        for fragment in [
            "LC_ALL=C",
            "cat /proc/stat",
            "cat /proc/meminfo",
            "df -Pk",
            "cat /proc/net/dev",
            "cat /proc/loadavg",
            "cat /proc/uptime",
            "ps -eo rss=,pcpu=,comm= --sort=-pcpu | head -n 24",
            "grep -m1 -E '^(model name|Hardware|Processor)[[:space:]]*:' /proc/cpuinfo",
        ] {
            assert!(REMOTE_SNAPSHOT_COMMAND.contains(fragment));
        }
        let markers = [
            "STAT", "MEM", "DISK", "NET", "LOAD", "UPTIME", "PS", "CPUINFO", "END",
        ];
        let mut previous = 0;
        for marker in markers {
            let position = REMOTE_SNAPSHOT_COMMAND.find(marker).unwrap();
            assert!(position >= previous);
            previous = position;
        }
        assert!(!REMOTE_SNAPSHOT_COMMAND.contains("{user}"));
        assert!(!REMOTE_SNAPSHOT_COMMAND.contains("{host}"));
        assert!(!REMOTE_SNAPSHOT_COMMAND.contains("$USER"));
        assert!(!REMOTE_SNAPSHOT_COMMAND.contains("$HOST"));
    }

    struct FakeSource {
        results: VecDeque<Result<String, String>>,
        collects: Arc<AtomicUsize>,
        drops: Arc<AtomicUsize>,
    }

    impl SnapshotSource for FakeSource {
        fn collect(&mut self) -> Result<String, String> {
            self.collects.fetch_add(1, Ordering::SeqCst);
            self.results
                .pop_front()
                .unwrap_or_else(|| Err("fake source exhausted".to_string()))
        }
    }

    impl Drop for FakeSource {
        fn drop(&mut self) {
            self.drops.fetch_add(1, Ordering::SeqCst);
        }
    }

    fn timing() -> WorkerTiming {
        WorkerTiming::new(Duration::from_secs(60), Duration::from_secs(60))
    }

    fn source(
        results: impl IntoIterator<Item = Result<String, String>>,
        collects: &Arc<AtomicUsize>,
        drops: &Arc<AtomicUsize>,
    ) -> FakeSource {
        FakeSource {
            results: results.into_iter().collect(),
            collects: Arc::clone(collects),
            drops: Arc::clone(drops),
        }
    }

    #[test]
    fn remote_event_debug_never_leaks_error_or_monitor_data() {
        let mut data = RemoteSnapshotParser::default()
            .parse(SAMPLE, Duration::ZERO)
            .unwrap();
        data.cpu_name = "RAW_MONITOR_SENTINEL".to_string();
        let update = format!(
            "{:?}",
            RemoteMonitorEvent::Update {
                key: MonitorKey::remote("user", "host", 22),
                generation: 7,
                data: Box::new(data),
            }
        );
        let failed = format!(
            "{:?}",
            RemoteMonitorEvent::Failed {
                key: MonitorKey::remote("user", "host", 22),
                generation: 8,
                error: "RAW_PASSWORD_SENTINEL".to_string(),
            }
        );

        assert!(update.contains("Update"));
        assert!(failed.contains("Failed"));
        assert!(!update.contains("RAW_MONITOR_SENTINEL"));
        assert!(!failed.contains("RAW_PASSWORD_SENTINEL"));
    }

    #[test]
    fn bounded_reader_accepts_limit_and_rejects_one_byte_over() {
        let at_limit = vec![b'x'; MAX_SNAPSHOT_BYTES];
        assert_eq!(
            read_snapshot_bounded(Cursor::new(at_limit)).unwrap().len(),
            MAX_SNAPSHOT_BYTES
        );

        let too_large = vec![b'x'; MAX_SNAPSHOT_BYTES + 1];
        assert!(read_snapshot_bounded(Cursor::new(too_large)).is_err());
    }

    #[test]
    fn shutdown_does_not_block_and_prevents_a_second_collect() {
        let collects = Arc::new(AtomicUsize::new(0));
        let drops = Arc::new(AtomicUsize::new(0));
        let (events_tx, events_rx) = mpsc::channel();
        let (done_tx, done_rx) = mpsc::channel();
        let source = source([Ok(SAMPLE.to_string())], &collects, &drops);
        let mut sources = VecDeque::from([Ok(source)]);
        let handle = start_worker_with_sink_for_test(
            MonitorKey::remote("user", "host", 22),
            1,
            move || {
                sources
                    .pop_front()
                    .unwrap_or_else(|| Err("no source".to_string()))
            },
            events_tx,
            timing(),
            done_tx,
        )
        .unwrap();

        assert!(matches!(
            events_rx.recv_timeout(Duration::from_secs(1)).unwrap(),
            RemoteMonitorEvent::Update { .. }
        ));
        handle.shutdown();
        done_rx.recv_timeout(Duration::from_secs(1)).unwrap();
        assert_eq!(collects.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn shutdown_is_nonblocking_idempotent_and_reports_worker_done() {
        struct BlockingSource {
            entered: mpsc::Sender<()>,
            release: mpsc::Receiver<()>,
        }

        impl SnapshotSource for BlockingSource {
            fn collect(&mut self) -> Result<String, String> {
                self.entered
                    .send(())
                    .map_err(|_| "进入通知接收端已关闭".to_string())?;
                self.release
                    .recv()
                    .map_err(|_| "释放通知发送端已关闭".to_string())?;
                Ok(SAMPLE.to_string())
            }
        }

        let (entered_tx, entered_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let (events_tx, _events_rx) = mpsc::channel();
        let mut source = Some(BlockingSource {
            entered: entered_tx,
            release: release_rx,
        });
        let mut handle = super::start_worker_with_sink(
            MonitorKey::remote("user", "host", 22),
            9,
            move || source.take().ok_or_else(|| "no source".to_string()),
            events_tx,
            timing(),
        )
        .unwrap();
        let done = handle
            .take_done_receiver_for_test()
            .expect("生产 handle 应提供独立完成通知");

        entered_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("worker 应进入阻塞 collect");
        let (returned_tx, returned_rx) = mpsc::channel();
        std::thread::spawn(move || {
            handle.shutdown();
            handle.shutdown();
            drop(handle);
            let _ = returned_tx.send(());
        });
        returned_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("shutdown 和 drop 不应等待阻塞中的 collect");
        assert!(
            done.try_recv().is_err(),
            "释放数据源前 worker 不应提前报告完成"
        );
        release_tx.send(()).expect("应能释放阻塞的数据源");
        done.recv_timeout(Duration::from_secs(1))
            .expect("collect 返回后 worker 应观察 shutdown 并结束");
    }

    #[test]
    fn dropping_handle_signals_shutdown_and_reports_worker_done() {
        let collects = Arc::new(AtomicUsize::new(0));
        let drops = Arc::new(AtomicUsize::new(0));
        let (events_tx, events_rx) = mpsc::channel();
        let source = source([Ok(SAMPLE.to_string())], &collects, &drops);
        let mut sources = VecDeque::from([Ok(source)]);
        let mut handle = super::start_worker_with_sink(
            MonitorKey::remote("user", "host", 22),
            10,
            move || {
                sources
                    .pop_front()
                    .unwrap_or_else(|| Err("no source".to_string()))
            },
            events_tx,
            timing(),
        )
        .unwrap();
        let done = handle
            .take_done_receiver_for_test()
            .expect("生产 handle 应提供独立完成通知");

        assert!(matches!(
            events_rx.recv_timeout(Duration::from_secs(1)).unwrap(),
            RemoteMonitorEvent::Update { .. }
        ));
        drop(handle);

        done.recv_timeout(Duration::from_secs(1))
            .expect("drop 不得遗留等待中的监控 worker");
    }

    #[test]
    fn connect_failure_reports_then_retries_to_update() {
        let collects = Arc::new(AtomicUsize::new(0));
        let drops = Arc::new(AtomicUsize::new(0));
        let (events_tx, events_rx) = mpsc::channel();
        let (done_tx, done_rx) = mpsc::channel();
        let source = source([Ok(SAMPLE.to_string())], &collects, &drops);
        let mut sources = VecDeque::from([Err("SSH 连接失败".to_string()), Ok(source)]);
        let handle = start_worker_with_sink_for_test(
            MonitorKey::remote("user", "host", 22),
            2,
            move || {
                sources
                    .pop_front()
                    .unwrap_or_else(|| Err("no source".to_string()))
            },
            events_tx,
            timing(),
            done_tx,
        )
        .unwrap();

        assert!(matches!(
            events_rx.recv_timeout(Duration::from_secs(1)).unwrap(),
            RemoteMonitorEvent::Failed { .. }
        ));
        handle.wake();
        assert!(matches!(
            events_rx.recv_timeout(Duration::from_secs(1)).unwrap(),
            RemoteMonitorEvent::Update { .. }
        ));
        handle.shutdown();
        done_rx.recv_timeout(Duration::from_secs(1)).unwrap();
    }

    #[test]
    fn collect_failure_drops_source_and_reconnect_resets_parser() {
        let collects = Arc::new(AtomicUsize::new(0));
        let drops = Arc::new(AtomicUsize::new(0));
        let (events_tx, events_rx) = mpsc::channel();
        let (done_tx, done_rx) = mpsc::channel();
        let first = source(
            [
                Ok(SAMPLE.to_string()),
                Err("channel read failed".to_string()),
            ],
            &collects,
            &drops,
        );
        let second_snapshot = SAMPLE
            .replace(
                "cpu  100 0 50 800 50 0 0 0 0 0",
                "cpu  120 0 60 870 50 0 0 0 0 0",
            )
            .replace("eth0: 1000", "eth0: 5000")
            .replace("0 0 0 0 2000", "0 0 0 0 10000");
        let second = source([Ok(second_snapshot)], &collects, &drops);
        let mut sources = VecDeque::from([Ok(first), Ok(second)]);
        let handle = start_worker_with_sink_for_test(
            MonitorKey::remote("user", "host", 22),
            3,
            move || {
                sources
                    .pop_front()
                    .unwrap_or_else(|| Err("no source".to_string()))
            },
            events_tx,
            timing(),
            done_tx,
        )
        .unwrap();

        assert!(matches!(
            events_rx.recv_timeout(Duration::from_secs(1)).unwrap(),
            RemoteMonitorEvent::Update { .. }
        ));
        handle.wake();
        assert!(matches!(
            events_rx.recv_timeout(Duration::from_secs(1)).unwrap(),
            RemoteMonitorEvent::Failed { .. }
        ));
        handle.wake();
        let update = events_rx.recv_timeout(Duration::from_secs(1)).unwrap();
        let RemoteMonitorEvent::Update { data, .. } = update else {
            panic!("expected update")
        };
        assert_eq!(data.cpu_percent, 0.0);
        assert!(data
            .net_interfaces
            .iter()
            .all(|iface| iface.rx_rate == 0 && iface.tx_rate == 0));
        assert!(drops.load(Ordering::SeqCst) >= 1);
        handle.shutdown();
        done_rx.recv_timeout(Duration::from_secs(1)).unwrap();
    }

    #[test]
    fn closed_sink_stops_worker() {
        let collects = Arc::new(AtomicUsize::new(0));
        let drops = Arc::new(AtomicUsize::new(0));
        let (done_tx, done_rx) = mpsc::channel();
        let source = source([Ok(SAMPLE.to_string())], &collects, &drops);
        let mut sources = VecDeque::from([Ok(source)]);
        let handle = start_worker_with_sink_for_test(
            MonitorKey::remote("user", "host", 22),
            4,
            move || {
                sources
                    .pop_front()
                    .unwrap_or_else(|| Err("no source".to_string()))
            },
            |_event| Err(()),
            timing(),
            done_tx,
        )
        .unwrap();

        done_rx.recv_timeout(Duration::from_secs(1)).unwrap();
        assert_eq!(collects.load(Ordering::SeqCst), 1);
        drop(handle);
    }
}
