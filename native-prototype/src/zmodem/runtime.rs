use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant};

use super::detect::AutoReceiveDetector;
use super::receiver::ReceiverEvent;
use super::sender::{FileInfo, SenderAction, ZmodemSender};
use super::session::{ProtocolTransport, SessionEvent, SessionReport, ZmodemSession};
use super::ZmodemError;

pub const TRANSPORT_WRITE_QUEUE_CAPACITY: usize = 32;
pub const DEFAULT_PROTOCOL_WRITE_TIMEOUT: Duration = Duration::from_secs(5);
pub const READER_PUMP_CAPACITY: usize = 8;
pub const READER_POLL_INTERVAL: Duration = Duration::from_millis(20);
pub const ACTIVE_READER_POLL_INTERVAL: Duration = Duration::from_millis(1);
pub const RUNTIME_COMMAND_CAPACITY: usize = 8;
pub const MAX_RUNTIME_PUMP_CHUNKS: usize = 1;

type WriteAck = Result<(), String>;

mod transport;

pub use transport::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RuntimeCapability {
    Local,
    DirectSsh,
    SerialDisabled,
}

impl RuntimeCapability {
    pub const fn supports_zmodem(self) -> bool {
        matches!(self, Self::Local | Self::DirectSsh)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TransferIdentity {
    pub transfer_id: u64,
    pub generation: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RuntimeCommand {
    StartSend {
        identity: TransferIdentity,
        paths: Vec<PathBuf>,
    },
    Cancel(TransferIdentity),
    Timeout(TransferIdentity),
    Shutdown,
}

pub type RuntimeCommandSender = mpsc::SyncSender<RuntimeCommand>;

pub fn runtime_command_channel() -> (RuntimeCommandSender, mpsc::Receiver<RuntimeCommand>) {
    mpsc::sync_channel(RUNTIME_COMMAND_CAPACITY)
}

pub struct RuntimeConfig {
    pub capability: RuntimeCapability,
    pub receive_directory: PathBuf,
    pub identity: TransferIdentity,
    pub commands: mpsc::Receiver<RuntimeCommand>,
    pub enabled: bool,
    pub auto_detect: bool,
    pub protocol_write_timeout: Duration,
    pub transfer_timeout: Option<Duration>,
    settings_source: RuntimeSettingsSource,
    settings_version: u64,
    pub allow_settings_enable: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RuntimeSettings {
    pub enabled: bool,
    pub auto_detect: bool,
    pub receive_directory: PathBuf,
    pub transfer_timeout: Option<Duration>,
}

#[derive(Clone)]
pub struct RuntimeSettingsSource {
    inner: Arc<Mutex<VersionedRuntimeSettings>>,
}

struct VersionedRuntimeSettings {
    version: u64,
    settings: RuntimeSettings,
}

impl RuntimeSettingsSource {
    pub fn new(settings: RuntimeSettings) -> Self {
        Self {
            inner: Arc::new(Mutex::new(VersionedRuntimeSettings {
                version: 0,
                settings,
            })),
        }
    }

    pub fn update(&self, settings: RuntimeSettings) {
        let mut current = self
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        current.version = current.version.wrapping_add(1);
        current.settings = settings;
    }

    pub fn snapshot(&self) -> (u64, RuntimeSettings) {
        let current = self
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        (current.version, current.settings.clone())
    }
}

impl RuntimeSettings {
    pub fn new(receive_directory: impl AsRef<Path>) -> Self {
        Self {
            enabled: true,
            auto_detect: true,
            receive_directory: receive_directory.as_ref().to_path_buf(),
            transfer_timeout: None,
        }
    }
}

impl RuntimeConfig {
    pub fn new(
        capability: RuntimeCapability,
        receive_directory: impl AsRef<Path>,
        identity: TransferIdentity,
        commands: mpsc::Receiver<RuntimeCommand>,
    ) -> Self {
        let receive_directory = receive_directory.as_ref().to_path_buf();
        let settings_source = RuntimeSettingsSource::new(RuntimeSettings {
            enabled: true,
            auto_detect: true,
            receive_directory: receive_directory.clone(),
            transfer_timeout: None,
        });
        Self {
            capability,
            receive_directory,
            identity,
            commands,
            enabled: true,
            auto_detect: true,
            protocol_write_timeout: DEFAULT_PROTOCOL_WRITE_TIMEOUT,
            transfer_timeout: None,
            settings_source,
            settings_version: 0,
            allow_settings_enable: true,
        }
    }

    pub fn use_settings_source(&mut self, source: RuntimeSettingsSource) {
        let (version, settings) = source.snapshot();
        self.enabled = settings.enabled && self.allow_settings_enable;
        self.auto_detect = settings.auto_detect;
        self.receive_directory = settings.receive_directory;
        self.transfer_timeout = settings.transfer_timeout;
        self.settings_source = source;
        self.settings_version = version;
    }

    pub fn settings(&self) -> RuntimeSettings {
        RuntimeSettings {
            enabled: self.enabled,
            auto_detect: self.auto_detect,
            receive_directory: self.receive_directory.clone(),
            transfer_timeout: self.transfer_timeout,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RuntimeEventKind {
    Started {
        direction: TransferDirection,
        filename: Option<String>,
        total: Option<u64>,
    },
    Receiver(ReceiverEvent),
    Sender(SenderAction),
    Error(ZmodemError),
    StaleCommand,
    Finished,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TransferDirection {
    Send,
    Receive,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RuntimeEvent {
    pub identity: TransferIdentity,
    pub kind: RuntimeEventKind,
}

#[derive(Debug, Default)]
pub struct RuntimeOutput {
    pub replay: Vec<u8>,
    pub events: Vec<RuntimeEvent>,
    pub shutdown: bool,
    pub discard_reader_epoch: bool,
}

struct AckTransport {
    writer: ProtocolWriter,
    timeout: Duration,
}

impl ProtocolTransport for AckTransport {
    fn write_protocol(&mut self, bytes: &[u8]) -> io::Result<()> {
        self.writer.write_and_flush(bytes, self.timeout)
    }
}

pub struct ZmodemRuntime {
    config: RuntimeConfig,
    detector: AutoReceiveDetector,
    session: Option<ZmodemSession<AckTransport>>,
    input_gate: Arc<ProtocolGate>,
    protocol_writer: Option<ProtocolWriter>,
    last_activity: Instant,
    active_identity: Option<TransferIdentity>,
    active_direction: Option<TransferDirection>,
    next_transfer_id: Option<u64>,
    highest_used_transfer_id: Option<u64>,
    pending_settings: Option<RuntimeSettings>,
    settings_source: RuntimeSettingsSource,
    observed_settings_version: u64,
}

impl ZmodemRuntime {
    pub fn new(
        config: RuntimeConfig,
        protocol_writer: Option<ProtocolWriter>,
        input_gate: Arc<ProtocolGate>,
    ) -> Self {
        let next_transfer_id = Some(config.identity.transfer_id);
        let settings_source = config.settings_source.clone();
        let observed_settings_version = config.settings_version;
        Self {
            config,
            detector: AutoReceiveDetector::new(),
            session: None,
            input_gate,
            protocol_writer,
            last_activity: Instant::now(),
            active_identity: None,
            active_direction: None,
            next_transfer_id,
            highest_used_transfer_id: None,
            pending_settings: None,
            settings_source,
            observed_settings_version,
        }
    }

    pub fn feed(&mut self, bytes: &[u8]) -> RuntimeOutput {
        if !self.config.enabled || !self.config.capability.supports_zmodem() {
            return RuntimeOutput {
                replay: bytes.to_vec(),
                ..RuntimeOutput::default()
            };
        }
        self.last_activity = Instant::now();
        if let Some(session) = &mut self.session {
            let generation = self.active_identity.unwrap().generation;
            let result = session.feed(generation, bytes);
            let mut output = self.finish_session_result(result);
            self.append_pump(&mut output);
            return output;
        }
        if !self.config.auto_detect {
            return RuntimeOutput {
                replay: bytes.to_vec(),
                ..RuntimeOutput::default()
            };
        }

        let detected = self.detector.feed(bytes);
        let mut output = RuntimeOutput {
            replay: detected.replay,
            ..RuntimeOutput::default()
        };
        let Some(_trigger) = detected.trigger else {
            return output;
        };
        let Some(writer) = self.protocol_writer.clone() else {
            output.discard_reader_epoch = true;
            output
                .events
                .push(self.event(RuntimeEventKind::Error(ZmodemError::Protocol(
                    "当前 transport 不支持 ZMODEM 协议 ACK 写入".into(),
                ))));
            self.detector.reset();
            return output;
        };
        let transport = AckTransport {
            writer,
            timeout: self.config.protocol_write_timeout,
        };
        let identity = match self.allocate_auto_identity() {
            Ok(identity) => identity,
            Err(error) => {
                output.discard_reader_epoch = true;
                output
                    .events
                    .push(self.event(RuntimeEventKind::Error(error)));
                self.detector.reset();
                return output;
            }
        };
        match ZmodemSession::receiving(
            identity.generation,
            &self.config.receive_directory,
            transport,
        ) {
            Ok(mut session) => {
                self.active_identity = Some(identity);
                self.active_direction = Some(TransferDirection::Receive);
                self.input_gate.activate();
                output.events.push(self.event(RuntimeEventKind::Started {
                    direction: TransferDirection::Receive,
                    filename: None,
                    total: None,
                }));
                match session.start() {
                    Ok(report) => output.events.extend(self.report_events(report.events)),
                    Err(error) => {
                        output.discard_reader_epoch = true;
                        self.input_gate.deactivate();
                        output
                            .events
                            .push(self.event(RuntimeEventKind::Error(error)));
                        self.detector.reset();
                        self.active_identity = None;
                        self.active_direction = None;
                        return output;
                    }
                }
                self.session = Some(session);
                if !detected.trailing.is_empty() {
                    let result = self
                        .session
                        .as_mut()
                        .unwrap()
                        .feed(identity.generation, &detected.trailing);
                    let trailing = self.finish_session_result(result);
                    output.replay.extend(trailing.replay);
                    output.events.extend(trailing.events);
                    output.discard_reader_epoch |= trailing.discard_reader_epoch;
                }
            }
            Err(error) => {
                output.discard_reader_epoch = true;
                self.detector.reset();
                output
                    .events
                    .push(self.event_for(identity, RuntimeEventKind::Error(error)));
            }
        }
        output
    }

    pub fn poll(&mut self) -> RuntimeOutput {
        let mut output = RuntimeOutput::default();
        if let Some(updated) = self.poll_settings_source() {
            output.replay.extend(updated.replay);
            output.events.extend(updated.events);
            output.discard_reader_epoch |= updated.discard_reader_epoch;
        }
        for _ in 0..RUNTIME_COMMAND_CAPACITY {
            let Ok(command) = self.config.commands.try_recv() else {
                break;
            };
            match command {
                RuntimeCommand::Shutdown => {
                    output.shutdown = true;
                    self.input_gate.deactivate();
                    return output;
                }
                RuntimeCommand::StartSend { identity, paths } => {
                    output.events.extend(self.start_send(identity, paths));
                }
                RuntimeCommand::Cancel(identity) | RuntimeCommand::Timeout(identity)
                    if Some(identity) != self.active_identity =>
                {
                    output
                        .events
                        .push(self.event_for(identity, RuntimeEventKind::StaleCommand));
                }
                RuntimeCommand::Cancel(_) => {
                    let generation = self.active_identity.unwrap().generation;
                    let result = self
                        .session
                        .as_mut()
                        .map(|session| session.cancel(generation));
                    if let Some(result) = result {
                        let mut next = self.finish_session_result(result);
                        next.discard_reader_epoch = true;
                        output.replay.extend(next.replay);
                        output.events.extend(next.events);
                        output.discard_reader_epoch |= next.discard_reader_epoch;
                    }
                }
                RuntimeCommand::Timeout(_) => {
                    let generation = self.active_identity.unwrap().generation;
                    let result = self
                        .session
                        .as_mut()
                        .map(|session| session.timeout(generation));
                    if let Some(result) = result {
                        let mut next = self.finish_session_result(result);
                        next.discard_reader_epoch = true;
                        output.replay.extend(next.replay);
                        output.events.extend(next.events);
                        output.discard_reader_epoch |= next.discard_reader_epoch;
                    }
                }
            }
        }
        if self.session.is_some()
            && self
                .config
                .transfer_timeout
                .is_some_and(|timeout| self.last_activity.elapsed() >= timeout)
        {
            if let Some(session) = &mut self.session {
                let generation = self.active_identity.unwrap().generation;
                let result = session.timeout(generation);
                let mut next = self.finish_session_result(result);
                next.discard_reader_epoch = true;
                output.replay.extend(next.replay);
                output.events.extend(next.events);
                output.discard_reader_epoch |= next.discard_reader_epoch;
            }
        }
        self.append_pump(&mut output);
        output
    }

    pub fn active(&self) -> bool {
        self.session.is_some()
    }

    pub fn reader_eof(&mut self) -> RuntimeOutput {
        self.finish_reader("ZMODEM 传输期间底层 reader 已到达 EOF")
    }

    pub fn reader_error(&mut self, error: &io::Error) -> RuntimeOutput {
        self.finish_reader(&format!("ZMODEM 底层 reader 读取失败: {error}"))
    }

    fn finish_reader(&mut self, message: &str) -> RuntimeOutput {
        let mut events = Vec::new();
        let mut discard_reader_epoch = false;
        if let Some(identity) = self.active_identity {
            if let Some(session) = &mut self.session {
                let _ = session.cancel(identity.generation);
            }
            events.push(self.event_for(
                identity,
                RuntimeEventKind::Error(ZmodemError::Protocol(message.into())),
            ));
            events.push(self.event_for(identity, RuntimeEventKind::Finished));
            discard_reader_epoch = true;
        }
        // The reader loop exits immediately after this call. Explicitly reject
        // queued sends so their pending UI state cannot survive forever.
        for _ in 0..RUNTIME_COMMAND_CAPACITY {
            match self.config.commands.try_recv() {
                Ok(RuntimeCommand::StartSend { identity, .. }) => {
                    events.push(self.event_for(
                        identity,
                        RuntimeEventKind::Error(ZmodemError::Protocol(
                            "终端连接已结束，无法开始 ZMODEM 发送".into(),
                        )),
                    ));
                }
                Ok(RuntimeCommand::Shutdown)
                | Ok(RuntimeCommand::Cancel(_))
                | Ok(RuntimeCommand::Timeout(_)) => {}
                Err(_) => break,
            }
        }
        self.session = None;
        self.input_gate.deactivate();
        self.detector.reset();
        self.active_direction = None;
        self.active_identity = None;
        let replay = self.apply_pending_settings_if_idle();
        RuntimeOutput {
            replay,
            events,
            discard_reader_epoch,
            ..RuntimeOutput::default()
        }
    }

    fn poll_settings_source(&mut self) -> Option<RuntimeOutput> {
        let (version, settings) = self.settings_source.snapshot();
        if version == self.observed_settings_version {
            return None;
        }
        self.observed_settings_version = version;
        Some(self.update_settings(settings))
    }

    fn finish_session_result(
        &mut self,
        result: Result<SessionReport, ZmodemError>,
    ) -> RuntimeOutput {
        let mut output = RuntimeOutput::default();
        match result {
            Ok(report) => {
                output.discard_reader_epoch = report
                    .events
                    .iter()
                    .any(|event| matches!(event, SessionEvent::ProtocolError(_)));
                output.replay = report.replay;
                output.events.extend(self.report_events(report.events));
            }
            Err(error) => {
                output.discard_reader_epoch = true;
                output
                    .events
                    .push(self.event(RuntimeEventKind::Error(error)));
                self.session = None;
            }
        }
        let finished = self
            .session
            .as_ref()
            .is_some_and(ZmodemSession::is_finished);
        if finished || (self.session.is_none() && self.active_identity.is_some()) {
            self.session = None;
            self.input_gate.deactivate();
            self.detector.reset();
            output.events.push(self.event(RuntimeEventKind::Finished));
            self.active_identity = None;
            self.active_direction = None;
            output.replay.extend(self.apply_pending_settings_if_idle());
        }
        output
    }

    fn update_settings(&mut self, settings: RuntimeSettings) -> RuntimeOutput {
        if self.session.is_none() {
            self.pending_settings = None;
            return RuntimeOutput {
                replay: self.apply_settings(settings),
                ..RuntimeOutput::default()
            };
        }
        if settings.enabled {
            self.pending_settings = Some(settings);
            return RuntimeOutput::default();
        }

        // Prevent any later command in this poll turn from starting a new
        // transfer before cancellation has completed.
        self.config.enabled = false;
        self.pending_settings = None;
        let generation = self.active_identity.unwrap().generation;
        let result = self
            .session
            .as_mut()
            .map(|session| session.cancel(generation));
        let mut output = result
            .map(|result| self.finish_session_result(result))
            .unwrap_or_default();
        output.discard_reader_epoch = true;
        output.replay.extend(self.apply_settings(settings));
        output
    }

    fn apply_pending_settings_if_idle(&mut self) -> Vec<u8> {
        if self.session.is_none() {
            if let Some(settings) = self.pending_settings.take() {
                return self.apply_settings(settings);
            }
        }
        Vec::new()
    }

    fn apply_settings(&mut self, settings: RuntimeSettings) -> Vec<u8> {
        self.config.enabled = settings.enabled && self.config.allow_settings_enable;
        self.config.auto_detect = settings.auto_detect;
        self.config.receive_directory = settings.receive_directory;
        self.config.transfer_timeout = settings.transfer_timeout;
        self.detector.reset()
    }

    fn report_events(&self, events: Vec<SessionEvent>) -> Vec<RuntimeEvent> {
        events
            .into_iter()
            .map(|event| {
                let kind = match event {
                    SessionEvent::Receiver(event) => RuntimeEventKind::Receiver(event),
                    SessionEvent::Sender(event) => RuntimeEventKind::Sender(event),
                    SessionEvent::ProtocolError(error) => RuntimeEventKind::Error(error),
                    SessionEvent::StaleInput => RuntimeEventKind::StaleCommand,
                };
                self.event(kind)
            })
            .collect()
    }

    fn event(&self, kind: RuntimeEventKind) -> RuntimeEvent {
        RuntimeEvent {
            identity: self.active_identity.unwrap_or(self.config.identity),
            kind,
        }
    }

    fn event_for(&self, identity: TransferIdentity, kind: RuntimeEventKind) -> RuntimeEvent {
        RuntimeEvent { identity, kind }
    }

    fn allocate_auto_identity(&mut self) -> Result<TransferIdentity, ZmodemError> {
        let transfer_id = self.next_transfer_id.ok_or_else(|| {
            ZmodemError::Protocol("ZMODEM transfer_id 已耗尽，必须创建新的终端 generation".into())
        })?;
        let identity = TransferIdentity {
            transfer_id,
            generation: self.config.identity.generation,
        };
        self.highest_used_transfer_id = Some(transfer_id);
        self.next_transfer_id = transfer_id.checked_add(1);
        Ok(identity)
    }

    fn start_send(&mut self, identity: TransferIdentity, paths: Vec<PathBuf>) -> Vec<RuntimeEvent> {
        if identity.generation != self.config.identity.generation {
            return vec![self.event_for(identity, RuntimeEventKind::StaleCommand)];
        }
        if self
            .highest_used_transfer_id
            .is_some_and(|highest| identity.transfer_id <= highest)
        {
            return vec![self.event_for(identity, RuntimeEventKind::StaleCommand)];
        }
        if self.session.is_some() {
            return vec![self.event_for(
                identity,
                RuntimeEventKind::Error(ZmodemError::Protocol("已有 ZMODEM 传输正在进行".into())),
            )];
        }
        if !self.config.enabled || !self.config.capability.supports_zmodem() {
            return vec![self.event_for(
                identity,
                RuntimeEventKind::Error(ZmodemError::Protocol(
                    "当前 transport 已禁用 ZMODEM".into(),
                )),
            )];
        }
        if paths.is_empty() {
            return vec![self.event_for(
                identity,
                RuntimeEventKind::Error(ZmodemError::Protocol("未选择要发送的文件".into())),
            )];
        }
        let files: Result<Vec<_>, _> = paths.into_iter().map(FileInfo::from_path).collect();
        let files = match files {
            Ok(files) => files,
            Err(error) => {
                return vec![self.event_for(identity, RuntimeEventKind::Error(error))];
            }
        };
        let first_name = files.first().map(|file| file.name.clone());
        let first_total = files.first().map(|file| file.size);
        let sender = match ZmodemSender::new(files) {
            Ok(sender) => sender,
            Err(error) => {
                return vec![self.event_for(identity, RuntimeEventKind::Error(error))];
            }
        };
        let Some(writer) = self.protocol_writer.clone() else {
            return vec![self.event_for(
                identity,
                RuntimeEventKind::Error(ZmodemError::Protocol(
                    "当前 transport 不支持 ZMODEM 协议 ACK 写入".into(),
                )),
            )];
        };
        let transport = AckTransport {
            writer,
            timeout: self.config.protocol_write_timeout,
        };
        let mut session = ZmodemSession::sending(identity.generation, sender, transport);
        self.highest_used_transfer_id = Some(identity.transfer_id);
        self.next_transfer_id = identity.transfer_id.checked_add(1);
        self.active_identity = Some(identity);
        self.active_direction = Some(TransferDirection::Send);
        self.input_gate.activate();
        let mut events = vec![self.event(RuntimeEventKind::Started {
            direction: TransferDirection::Send,
            filename: first_name,
            total: first_total,
        })];
        match session.start() {
            Ok(report) => events.extend(self.report_events(report.events)),
            Err(error) => {
                events.push(self.event(RuntimeEventKind::Error(error)));
                events.push(self.event(RuntimeEventKind::Finished));
                self.input_gate.deactivate();
                self.active_identity = None;
                self.active_direction = None;
                return events;
            }
        }
        self.session = Some(session);
        self.last_activity = Instant::now();
        events
    }

    fn append_pump(&mut self, output: &mut RuntimeOutput) {
        if self.active_direction != Some(TransferDirection::Send) {
            return;
        }
        let Some(session) = self.session.as_mut() else {
            return;
        };
        let result = session.pump_sender(MAX_RUNTIME_PUMP_CHUNKS);
        if result.as_ref().is_ok_and(|report| {
            report
                .events
                .iter()
                .any(|event| matches!(event, SessionEvent::Sender(SenderAction::Progress { .. })))
        }) {
            self.last_activity = Instant::now();
        }
        let pumped = self.finish_session_result(result);
        output.replay.extend(pumped.replay);
        output.events.extend(pumped.events);
        output.discard_reader_epoch |= pumped.discard_reader_epoch;
    }
}

impl Drop for ZmodemRuntime {
    fn drop(&mut self) {
        self.input_gate.deactivate();
    }
}

#[cfg(test)]
#[path = "runtime/tests.rs"]
mod tests;
