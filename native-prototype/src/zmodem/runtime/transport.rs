use super::*;

pub struct ProtocolWriteRequest {
    bytes: Vec<u8>,
    deadline: Instant,
    cancelled: Arc<AtomicBool>,
    started: Arc<AtomicBool>,
    gate_released: AtomicBool,
    protocol_gate: Arc<ProtocolGate>,
    completion: mpsc::Sender<WriteAck>,
}

impl ProtocolWriteRequest {
    pub(crate) fn begin(&self) -> bool {
        if self.cancelled.load(Ordering::Acquire) || Instant::now() >= self.deadline {
            return false;
        }
        let _transition = self.protocol_gate.transition.lock().unwrap();
        if self.cancelled.load(Ordering::Acquire) || Instant::now() >= self.deadline {
            return false;
        }
        let started = self
            .started
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_ok();
        if started {
            self.protocol_gate
                .in_flight_protocol_writes
                .fetch_add(1, Ordering::AcqRel);
        }
        started
    }

    pub(crate) fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    pub(crate) fn may_continue(&self) -> bool {
        !self.cancelled.load(Ordering::Acquire) && Instant::now() < self.deadline
    }

    pub(crate) fn complete(self, result: io::Result<()>) {
        self.release_gate();
        let _ = self
            .completion
            .send(result.map_err(|error| error.to_string()));
    }

    fn release_gate(&self) {
        if self.started.load(Ordering::Acquire)
            && self
                .gate_released
                .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
                .is_ok()
        {
            self.protocol_gate
                .in_flight_protocol_writes
                .fetch_sub(1, Ordering::AcqRel);
        }
    }
}

impl Drop for ProtocolWriteRequest {
    fn drop(&mut self) {
        self.release_gate();
    }
}

pub enum TransportWrite {
    Normal { bytes: Vec<u8>, epoch: u64 },
    Protocol(ProtocolWriteRequest),
    TerminalReply(TerminalReplyRequest),
}

pub struct TerminalReplyRequest {
    bytes: Vec<u8>,
    deadline: Instant,
    cancelled: Arc<AtomicBool>,
    started: Arc<AtomicBool>,
    progressed: Arc<AtomicBool>,
    completion: mpsc::Sender<WriteAck>,
}

impl TerminalReplyRequest {
    pub(crate) fn begin(&self) -> bool {
        if self.cancelled.load(Ordering::Acquire) || Instant::now() >= self.deadline {
            return false;
        }
        self.started
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
    }

    pub(crate) fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    pub(crate) fn may_continue(&self) -> bool {
        self.progressed.load(Ordering::Acquire)
            || (!self.cancelled.load(Ordering::Acquire) && Instant::now() < self.deadline)
    }

    pub(crate) fn mark_progress(&self) {
        self.progressed.store(true, Ordering::Release);
    }

    pub(crate) fn complete(self, result: io::Result<()>) {
        let _ = self
            .completion
            .send(result.map_err(|error| error.to_string()));
    }
}

pub struct ProtocolGate {
    active: AtomicBool,
    in_flight_protocol_writes: AtomicU64,
    epoch: AtomicU64,
    transition: Mutex<()>,
}

impl ProtocolGate {
    pub fn new() -> Self {
        Self {
            active: AtomicBool::new(false),
            in_flight_protocol_writes: AtomicU64::new(0),
            epoch: AtomicU64::new(0),
            transition: Mutex::new(()),
        }
    }

    pub fn is_active(&self) -> bool {
        self.active.load(Ordering::Acquire)
            || self.in_flight_protocol_writes.load(Ordering::Acquire) != 0
    }

    pub fn activate(&self) {
        let _transition = self.transition.lock().unwrap();
        self.epoch.fetch_add(1, Ordering::AcqRel);
        self.active.store(true, Ordering::Release);
    }

    pub fn deactivate(&self) {
        let _transition = self.transition.lock().unwrap();
        self.active.store(false, Ordering::Release);
    }

    pub(crate) fn epoch(&self) -> u64 {
        self.epoch.load(Ordering::Acquire)
    }
}

impl Default for ProtocolGate {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone)]
pub struct TransportWriter {
    sender: mpsc::SyncSender<TransportWrite>,
    protocol_gate: Arc<ProtocolGate>,
}

impl TransportWriter {
    pub fn protocol_active_gate(&self) -> Arc<ProtocolGate> {
        Arc::clone(&self.protocol_gate)
    }

    pub fn try_send_normal(&self, bytes: Vec<u8>) -> io::Result<()> {
        let _transition = self.protocol_gate.transition.lock().unwrap();
        if self.protocol_gate.is_active() {
            return Err(io::Error::new(
                io::ErrorKind::WouldBlock,
                "ZMODEM 独占传输期间已拒绝终端输入",
            ));
        }
        let epoch = self.protocol_gate.epoch();
        self.sender
            .try_send(TransportWrite::Normal { bytes, epoch })
            .map_err(|error| match error {
                mpsc::TrySendError::Full(_) => {
                    io::Error::new(io::ErrorKind::WouldBlock, "终端写入队列已满")
                }
                mpsc::TrySendError::Disconnected(_) => {
                    io::Error::new(io::ErrorKind::BrokenPipe, "终端写入队列已断开")
                }
            })
    }
}

pub fn transport_write_channel(
    protocol_gate: Arc<ProtocolGate>,
) -> (TransportWriter, mpsc::Receiver<TransportWrite>) {
    let (sender, receiver) = mpsc::sync_channel(TRANSPORT_WRITE_QUEUE_CAPACITY);
    (
        TransportWriter {
            sender,
            protocol_gate,
        },
        receiver,
    )
}

/// Bounded, acknowledged protocol writer over the typed transport channel.
/// The transport worker acknowledges only after complete write+flush.
#[derive(Clone)]
pub struct ProtocolWriter {
    transport: TransportWriter,
}

impl ProtocolWriter {
    pub fn from_transport_writer(transport: TransportWriter) -> Self {
        Self { transport }
    }

    pub fn write_and_flush(&self, bytes: &[u8], timeout: Duration) -> io::Result<()> {
        let (completion, completed) = mpsc::channel();
        let cancelled = Arc::new(AtomicBool::new(false));
        let started = Arc::new(AtomicBool::new(false));
        let deadline = Instant::now()
            .checked_add(timeout)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "协议写超时无效"))?;
        self.transport
            .sender
            .try_send(TransportWrite::Protocol(ProtocolWriteRequest {
                bytes: bytes.to_vec(),
                deadline,
                cancelled: Arc::clone(&cancelled),
                started: Arc::clone(&started),
                gate_released: AtomicBool::new(false),
                protocol_gate: Arc::clone(&self.transport.protocol_gate),
                completion,
            }))
            .map_err(|error| match error {
                mpsc::TrySendError::Full(_) => {
                    io::Error::new(io::ErrorKind::WouldBlock, "ZMODEM 协议写队列已满")
                }
                mpsc::TrySendError::Disconnected(_) => {
                    io::Error::new(io::ErrorKind::BrokenPipe, "ZMODEM 协议写队列已断开")
                }
            })?;
        match completed.recv_timeout(timeout) {
            Ok(result) => result.map_err(io::Error::other),
            Err(error) => {
                cancelled.store(true, Ordering::Release);
                let state = if started.load(Ordering::Acquire) {
                    "底层写入已开始并已请求停止"
                } else {
                    "排队请求已取消且不会写入"
                };
                Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    format!("等待底层 write+flush ACK 超时（{state}）: {error}"),
                ))
            }
        }
    }
}

/// Acknowledged terminal-emulator reply writer.
///
/// Unlike `ProtocolWriter`, this path does not activate the ZMODEM exclusivity
/// gate. It exists for immediate DA/DSR/mode-report replies generated while
/// parsing PTY output, and acknowledges only after the transport has completed
/// `write + flush`.
#[derive(Clone)]
pub struct TerminalReplyWriter {
    transport: TransportWriter,
}

impl TerminalReplyWriter {
    pub fn from_transport_writer(transport: TransportWriter) -> Self {
        Self { transport }
    }

    /// Queue a terminal-emulator reply without waiting under the terminal parser lock.
    pub fn try_enqueue(&self, bytes: &[u8], timeout: Duration) -> io::Result<()> {
        let (completion, _completed) = mpsc::channel();
        let deadline = Instant::now()
            .checked_add(timeout)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "终端应答写超时无效"))?;
        self.transport
            .sender
            .try_send(TransportWrite::TerminalReply(TerminalReplyRequest {
                bytes: bytes.to_vec(),
                deadline,
                cancelled: Arc::new(AtomicBool::new(false)),
                started: Arc::new(AtomicBool::new(false)),
                progressed: Arc::new(AtomicBool::new(false)),
                completion,
            }))
            .map_err(|error| match error {
                mpsc::TrySendError::Full(_) => {
                    io::Error::new(io::ErrorKind::WouldBlock, "终端应答写队列已满")
                }
                mpsc::TrySendError::Disconnected(_) => {
                    io::Error::new(io::ErrorKind::BrokenPipe, "终端应答写队列已断开")
                }
            })
    }

    pub fn write_and_flush(&self, bytes: &[u8], timeout: Duration) -> io::Result<()> {
        let (completion, completed) = mpsc::channel();
        let cancelled = Arc::new(AtomicBool::new(false));
        let started = Arc::new(AtomicBool::new(false));
        let progressed = Arc::new(AtomicBool::new(false));
        let deadline = Instant::now()
            .checked_add(timeout)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "终端应答写超时无效"))?;
        let mut message = TransportWrite::TerminalReply(TerminalReplyRequest {
            bytes: bytes.to_vec(),
            deadline,
            cancelled: Arc::clone(&cancelled),
            started: Arc::clone(&started),
            progressed,
            completion,
        });
        loop {
            match self.transport.sender.try_send(message) {
                Ok(()) => break,
                Err(mpsc::TrySendError::Full(returned)) => {
                    if Instant::now() >= deadline {
                        return Err(io::Error::new(
                            io::ErrorKind::TimedOut,
                            "等待终端应答写队列空位超时",
                        ));
                    }
                    message = returned;
                    std::thread::sleep(Duration::from_millis(1));
                }
                Err(mpsc::TrySendError::Disconnected(_)) => {
                    return Err(io::Error::new(
                        io::ErrorKind::BrokenPipe,
                        "终端应答写队列已断开",
                    ));
                }
            }
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        match completed.recv_timeout(remaining) {
            Ok(result) => result.map_err(io::Error::other),
            Err(error) => {
                cancelled.store(true, Ordering::Release);
                let state = if started.load(Ordering::Acquire) {
                    "底层写入已开始并已请求停止"
                } else {
                    "排队请求已取消且不会写入"
                };
                Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    format!("等待终端应答 write+flush ACK 超时（{state}）: {error}"),
                ))
            }
        }
    }
}
