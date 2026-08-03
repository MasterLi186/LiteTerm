use std::io;
use std::path::Path;

use super::decode::{
    parse_header_prefix, CancelDetector, DataSubpacketDecoder, HeaderDecoder, HeaderParse,
};
use super::receiver::{ReceiverEvent, ReceiverOutput, ZmodemReceiver};
use super::sender::{SenderAction, ZmodemSender};
use super::{ChecksumMode, HeaderFormat, ZmodemError};

const MAX_PENDING_PROTOCOL_BYTES: usize = 2 * 1024 * 1024;
const FEED_CHUNK_BYTES: usize = 64 * 1024;
const MAX_PUMP_CHUNKS: usize = 64;

pub trait ProtocolTransport {
    fn write_protocol(&mut self, bytes: &[u8]) -> io::Result<()>;
}

#[derive(Debug, Default)]
pub struct FakeTransport {
    writes: Vec<Vec<u8>>,
    fail_after: Option<usize>,
}

impl FakeTransport {
    pub fn writes(&self) -> &[Vec<u8>] {
        &self.writes
    }

    pub fn take_writes(&mut self) -> Vec<Vec<u8>> {
        std::mem::take(&mut self.writes)
    }

    pub fn fail_after(&mut self, successful_writes: usize) {
        self.fail_after = Some(successful_writes);
    }
}

impl ProtocolTransport for FakeTransport {
    fn write_protocol(&mut self, bytes: &[u8]) -> io::Result<()> {
        if self
            .fail_after
            .is_some_and(|successful_writes| self.writes.len() >= successful_writes)
        {
            return Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "fake transport failure",
            ));
        }
        self.writes.push(bytes.to_vec());
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionEvent {
    Sender(SenderAction),
    Receiver(ReceiverEvent),
    ProtocolError(ZmodemError),
    StaleInput,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SessionReport {
    pub events: Vec<SessionEvent>,
    /// Bytes observed after ZFIN which are not the `OO` terminator. The raw
    /// reader may replay these to the terminal parser without loss.
    pub replay: Vec<u8>,
}

enum Mode {
    Sending {
        sender: ZmodemSender,
        headers: HeaderDecoder,
    },
    Receiving {
        receiver: ZmodemReceiver,
        pending: Vec<u8>,
        data: DataSubpacketDecoder,
        compactions: usize,
    },
}

pub struct ZmodemSession<T> {
    generation: u64,
    transport: T,
    mode: Mode,
    cancel_detector: CancelDetector,
    finished: bool,
}

impl<T: ProtocolTransport> ZmodemSession<T> {
    pub fn sending(generation: u64, sender: ZmodemSender, transport: T) -> Self {
        Self {
            generation,
            transport,
            mode: Mode::Sending {
                sender,
                headers: HeaderDecoder::new(),
            },
            cancel_detector: CancelDetector::default(),
            finished: false,
        }
    }

    pub fn receiving(
        generation: u64,
        destination: impl AsRef<Path>,
        transport: T,
    ) -> Result<Self, ZmodemError> {
        Ok(Self {
            generation,
            transport,
            mode: Mode::Receiving {
                receiver: ZmodemReceiver::new(destination)?,
                pending: Vec::new(),
                data: DataSubpacketDecoder::default(),
                compactions: 0,
            },
            cancel_detector: CancelDetector::default(),
            finished: false,
        })
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub fn transport(&self) -> &T {
        &self.transport
    }

    pub fn transport_mut(&mut self) -> &mut T {
        &mut self.transport
    }

    pub fn is_finished(&self) -> bool {
        self.finished
    }

    #[cfg(test)]
    fn pending_metrics(&self) -> (usize, usize) {
        match &self.mode {
            Mode::Receiving {
                pending,
                compactions,
                ..
            } => (pending.len(), *compactions),
            Mode::Sending { .. } => (0, 0),
        }
    }

    pub fn start(&mut self) -> Result<SessionReport, ZmodemError> {
        let mut report = SessionReport::default();
        match &mut self.mode {
            Mode::Sending { sender, .. } => {
                let action = sender.start();
                write_sender_action(&mut self.transport, action, &mut report)?;
            }
            Mode::Receiving { receiver, .. } => {
                write_receiver_output(&mut self.transport, receiver.start(), &mut report)?;
            }
        }
        Ok(report)
    }

    /// Feed bytes tagged with the transfer generation.
    ///
    /// A stale generation is ignored without touching decoder state. Transport
    /// writes are acknowledged synchronously by `ProtocolTransport`; the real
    /// Terminal/SSH bridge can map this trait to its bounded writer queue.
    pub fn feed(&mut self, generation: u64, bytes: &[u8]) -> Result<SessionReport, ZmodemError> {
        if generation != self.generation {
            return Ok(SessionReport {
                events: vec![SessionEvent::StaleInput],
                ..SessionReport::default()
            });
        }
        if self.finished {
            return Ok(SessionReport::default());
        }
        if self.cancel_detector.feed(bytes) {
            return self.remote_cancel();
        }

        let mut report = SessionReport::default();
        match &mut self.mode {
            Mode::Sending { sender, headers } => {
                for frame in headers.feed(bytes) {
                    let action = sender.handle_frame(frame);
                    write_sender_action(&mut self.transport, action, &mut report)?;
                }
                self.finished = sender.is_done();
            }
            Mode::Receiving {
                receiver,
                pending,
                data,
                compactions,
            } => {
                let mut input_offset = 0;
                while input_offset < bytes.len() {
                    if pending.len() == MAX_PENDING_PROTOCOL_BYTES {
                        let output = receiver.cancel();
                        self.finished = true;
                        let write_result =
                            write_receiver_output(&mut self.transport, output, &mut report);
                        report
                            .events
                            .push(SessionEvent::ProtocolError(ZmodemError::Protocol(
                                "ZMODEM 待解析输入超过安全上限".into(),
                            )));
                        write_result?;
                        return Ok(report);
                    }
                    let take = FEED_CHUNK_BYTES
                        .min(bytes.len() - input_offset)
                        .min(MAX_PENDING_PROTOCOL_BYTES - pending.len());
                    pending.extend_from_slice(&bytes[input_offset..input_offset + take]);
                    input_offset += take;

                    let mut cursor = 0;
                    while cursor < pending.len() {
                        if receiver.expects_data_subpacket() {
                            let fed = data.feed_one(&pending[cursor..]);
                            cursor += fed.consumed;
                            let Some(result) = fed.result else {
                                break;
                            };
                            match result {
                                Ok(packet) => {
                                    let output = receiver.handle_data(packet);
                                    let write_result = write_receiver_output_or_cleanup(
                                        &mut self.transport,
                                        receiver,
                                        output,
                                        &mut report,
                                    );
                                    if write_result.is_err() {
                                        self.finished = true;
                                    }
                                    write_result?;
                                }
                                Err(error) => {
                                    let output = receiver.cancel();
                                    self.finished = true;
                                    let write_result = write_receiver_output(
                                        &mut self.transport,
                                        output,
                                        &mut report,
                                    );
                                    report.events.push(SessionEvent::ProtocolError(
                                        ZmodemError::Protocol(format!(
                                            "ZMODEM 数据子包校验失败: {error:?}"
                                        )),
                                    ));
                                    write_result?;
                                    break;
                                }
                            }
                            continue;
                        }

                        if receiver.is_finishing() {
                            if pending[cursor] == b'O' {
                                if cursor + 1 == pending.len() {
                                    break;
                                }
                                if pending[cursor + 1] == b'O' {
                                    cursor += 2;
                                    let output = receiver.handle_over_and_out(b"OO");
                                    let write_result = write_receiver_output_or_cleanup(
                                        &mut self.transport,
                                        receiver,
                                        output,
                                        &mut report,
                                    );
                                    if write_result.is_err() {
                                        self.finished = true;
                                    }
                                    write_result?;
                                    self.finished = receiver.is_done();
                                    if self.finished {
                                        report.replay.extend_from_slice(&pending[cursor..]);
                                        cursor = pending.len();
                                        break;
                                    }
                                    continue;
                                }
                            }
                            let next_o = pending[cursor + 1..]
                                .iter()
                                .position(|byte| *byte == b'O')
                                .map_or(pending.len(), |relative| cursor + 1 + relative);
                            report.replay.extend_from_slice(&pending[cursor..next_o]);
                            cursor = next_o;
                            continue;
                        }

                        match parse_header_prefix(&pending[cursor..]) {
                            HeaderParse::Complete { frame, consumed } => {
                                cursor += consumed;
                                if matches!(
                                    frame.frame_type,
                                    super::FrameType::Zfile | super::FrameType::Zdata
                                ) {
                                    data.set_checksum_mode(match frame.format {
                                        HeaderFormat::Binary16 => ChecksumMode::Crc16,
                                        HeaderFormat::Binary32 => ChecksumMode::Crc32,
                                        HeaderFormat::Hex => ChecksumMode::Crc16,
                                    });
                                }
                                let output = receiver.handle_header(frame);
                                let write_result = write_receiver_output_or_cleanup(
                                    &mut self.transport,
                                    receiver,
                                    output,
                                    &mut report,
                                );
                                if write_result.is_err() {
                                    self.finished = true;
                                }
                                write_result?;
                                self.finished = receiver.is_done();
                            }
                            HeaderParse::NeedMore => break,
                            HeaderParse::Invalid => cursor += 1,
                        }
                    }
                    if cursor != 0 {
                        pending.drain(..cursor);
                        *compactions += 1;
                    }
                    if self.finished {
                        if input_offset < bytes.len() {
                            report.replay.extend_from_slice(&bytes[input_offset..]);
                        }
                        break;
                    }
                }
            }
        }
        Ok(report)
    }

    /// Pump a bounded number of sender subpackets so a caller cannot monopolize
    /// its event-loop thread.
    pub fn pump_sender(&mut self, max_chunks: usize) -> Result<SessionReport, ZmodemError> {
        let mut report = SessionReport::default();
        let Mode::Sending { sender, .. } = &mut self.mode else {
            return Ok(report);
        };
        for _ in 0..max_chunks.min(MAX_PUMP_CHUNKS) {
            let progress_before = sender.progress();
            let Some(action) = sender.next_data_chunk() else {
                break;
            };
            write_sender_action(&mut self.transport, action, &mut report)?;
            let progress_after = sender.progress();
            if progress_after != progress_before {
                if let Some(progress) = progress_after {
                    report.events.push(SessionEvent::Sender(progress));
                }
            }
            if !sender.in_send_data() {
                break;
            }
        }
        self.finished = sender.is_done();
        Ok(report)
    }

    pub fn cancel(&mut self, generation: u64) -> Result<SessionReport, ZmodemError> {
        if generation != self.generation {
            return Ok(SessionReport {
                events: vec![SessionEvent::StaleInput],
                ..SessionReport::default()
            });
        }
        let mut report = SessionReport::default();
        self.finished = true;
        match &mut self.mode {
            Mode::Sending { sender, .. } => {
                write_sender_action(&mut self.transport, sender.cancel(), &mut report)?;
            }
            Mode::Receiving { receiver, .. } => {
                write_receiver_output(&mut self.transport, receiver.cancel(), &mut report)?;
            }
        }
        Ok(report)
    }

    /// Timeout is an explicit coordinator input, keeping the core deterministic
    /// and testable without sleeping.
    pub fn timeout(&mut self, generation: u64) -> Result<SessionReport, ZmodemError> {
        let mut report = self.cancel(generation)?;
        if generation == self.generation {
            report
                .events
                .push(SessionEvent::ProtocolError(ZmodemError::Protocol(
                    "ZMODEM 传输超时".into(),
                )));
        }
        Ok(report)
    }

    fn remote_cancel(&mut self) -> Result<SessionReport, ZmodemError> {
        let mut report = SessionReport::default();
        self.finished = true;
        match &mut self.mode {
            Mode::Sending { sender, .. } => {
                let _ =
                    sender.handle_frame(super::DecodedFrame::new(super::FrameType::Zcan, [0; 4]));
                report
                    .events
                    .push(SessionEvent::ProtocolError(ZmodemError::Cancelled));
            }
            Mode::Receiving { receiver, .. } => {
                let output = receiver
                    .handle_header(super::DecodedFrame::new(super::FrameType::Zcan, [0; 4]));
                write_receiver_output(&mut self.transport, output, &mut report)?;
            }
        }
        Ok(report)
    }
}

fn write_sender_action<T: ProtocolTransport>(
    transport: &mut T,
    action: SenderAction,
    report: &mut SessionReport,
) -> Result<(), ZmodemError> {
    match action {
        SenderAction::Send(bytes) => transport.write_protocol(&bytes)?,
        SenderAction::Error(error) => report.events.push(SessionEvent::ProtocolError(error)),
        SenderAction::None => {}
        event => report.events.push(SessionEvent::Sender(event)),
    }
    Ok(())
}

fn write_receiver_output<T: ProtocolTransport>(
    transport: &mut T,
    output: ReceiverOutput,
    report: &mut SessionReport,
) -> Result<(), ZmodemError> {
    for bytes in output.writes {
        transport.write_protocol(&bytes)?;
    }
    report
        .events
        .extend(output.events.into_iter().map(SessionEvent::Receiver));
    Ok(())
}

fn write_receiver_output_or_cleanup<T: ProtocolTransport>(
    transport: &mut T,
    receiver: &mut ZmodemReceiver,
    output: ReceiverOutput,
    report: &mut SessionReport,
) -> Result<(), ZmodemError> {
    if let Err(error) = write_receiver_output(transport, output, report) {
        // `cancel` removes an active .part before constructing CAN bytes. Do not
        // attempt another transport write here: the original response already
        // proved the writer unavailable, but filesystem cleanup must still win.
        let _ = receiver.cancel();
        Err(error)
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::zmodem::decode::DataEnd;
    use crate::zmodem::encode::{
        encode_data_subpacket, encode_data_subpacket_with_checksum, encode_zbin16_header,
        encode_zbin32_header, encode_zfile_metadata, encode_zhex_header,
    };
    use crate::zmodem::{ChecksumMode, FrameType, ZCRCE, ZCRCW};

    #[test]
    fn receiver_session_handles_fragmented_header_metadata_and_data() {
        let directory = tempfile::tempdir().unwrap();
        let mut session =
            ZmodemSession::receiving(7, directory.path(), FakeTransport::default()).unwrap();
        session.start().unwrap();

        let mut wire = encode_zbin32_header(FrameType::Zfile, [0; 4]);
        wire.extend(
            encode_data_subpacket(&encode_zfile_metadata("received", 3, 0, 1), ZCRCW, true)
                .unwrap(),
        );
        wire.extend(encode_zbin32_header(FrameType::Zdata, [0; 4]));
        wire.extend(encode_data_subpacket(b"abc", ZCRCE, true).unwrap());
        wire.extend(encode_zbin32_header(FrameType::Zeof, 3u32.to_le_bytes()));
        for byte in wire {
            session.feed(7, &[byte]).unwrap();
        }
        assert_eq!(
            std::fs::read(directory.path().join("received")).unwrap(),
            b"abc"
        );
    }

    #[test]
    fn sender_pump_reports_progress_only_when_file_offset_advances() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("send.bin");
        std::fs::write(&path, b"abc").unwrap();
        let sender = crate::zmodem::sender::ZmodemSender::new(vec![
            crate::zmodem::sender::FileInfo::from_path(&path).unwrap(),
        ])
        .unwrap();
        let mut session = ZmodemSession::sending(21, sender, FakeTransport::default());
        session.start().unwrap();
        session
            .feed(
                21,
                &encode_zhex_header(FrameType::Zrinit, [0, 0, 0, crate::zmodem::CANFC32]),
            )
            .unwrap();
        session
            .feed(
                21,
                &encode_zhex_header(FrameType::Zrpos, 0u32.to_le_bytes()),
            )
            .unwrap();

        let report = session.pump_sender(1).unwrap();
        assert!(report.events.iter().any(|event| matches!(
            event,
            SessionEvent::Sender(SenderAction::Progress {
                bytes_sent: 3,
                total: 3,
                ..
            })
        )));
        let eof = session.pump_sender(1).unwrap();
        assert!(!eof
            .events
            .iter()
            .any(|event| matches!(event, SessionEvent::Sender(SenderAction::Progress { .. }))));
    }

    #[test]
    fn receiver_session_accepts_crc16_headers_and_data() {
        let directory = tempfile::tempdir().unwrap();
        let mut session =
            ZmodemSession::receiving(8, directory.path(), FakeTransport::default()).unwrap();
        session.start().unwrap();

        let mut wire = encode_zbin16_header(FrameType::Zfile, [0; 4]);
        wire.extend(
            encode_data_subpacket_with_checksum(
                &encode_zfile_metadata("crc16", 3, 0, 1),
                ZCRCW,
                true,
                ChecksumMode::Crc16,
            )
            .unwrap(),
        );
        wire.extend(encode_zbin16_header(FrameType::Zdata, [0; 4]));
        wire.extend(
            encode_data_subpacket_with_checksum(b"abc", ZCRCE, true, ChecksumMode::Crc16).unwrap(),
        );
        wire.extend(encode_zbin16_header(FrameType::Zeof, 3u32.to_le_bytes()));
        for byte in wire {
            session.feed(8, &[byte]).unwrap();
        }
        assert_eq!(
            std::fs::read(directory.path().join("crc16")).unwrap(),
            b"abc"
        );
    }

    #[test]
    fn stale_generation_does_not_mutate_or_write() {
        let directory = tempfile::tempdir().unwrap();
        let mut session =
            ZmodemSession::receiving(9, directory.path(), FakeTransport::default()).unwrap();
        session.start().unwrap();
        let before = session.transport().writes().len();
        let report = session
            .feed(8, &encode_zhex_header(FrameType::Zfile, [0; 4]))
            .unwrap();
        assert_eq!(report.events, vec![SessionEvent::StaleInput]);
        assert_eq!(session.transport().writes().len(), before);
    }

    #[test]
    fn timeout_cleans_partial_receiver_file() {
        let directory = tempfile::tempdir().unwrap();
        let mut session =
            ZmodemSession::receiving(1, directory.path(), FakeTransport::default()).unwrap();
        session.start().unwrap();
        let mut wire = encode_zbin32_header(FrameType::Zfile, [0; 4]);
        wire.extend(
            encode_data_subpacket(
                &encode_zfile_metadata("partial", 10, 0, 1),
                DataEnd::EndAck.to_wire(),
                true,
            )
            .unwrap(),
        );
        session.feed(1, &wire).unwrap();
        session.timeout(1).unwrap();
        assert!(std::fs::read_dir(directory.path())
            .unwrap()
            .all(|entry| !entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .ends_with(".part")));
    }

    #[test]
    fn transport_failure_after_part_creation_cleans_immediately() {
        let directory = tempfile::tempdir().unwrap();
        let mut session =
            ZmodemSession::receiving(10, directory.path(), FakeTransport::default()).unwrap();
        session.start().unwrap();
        session
            .feed(10, &encode_zbin32_header(FrameType::Zfile, [0; 4]))
            .unwrap();
        let successful_writes = session.transport().writes().len();
        session.transport_mut().fail_after(successful_writes);

        let metadata =
            encode_data_subpacket(&encode_zfile_metadata("partial", 10, 0, 1), ZCRCW, true)
                .unwrap();
        assert!(session.feed(10, &metadata).is_err());
        assert!(session.is_finished());
        assert!(!has_part_file(directory.path()));
    }

    fn begin_partial(session: &mut ZmodemSession<FakeTransport>, generation: u64, name: &str) {
        session.start().unwrap();
        let mut wire = encode_zbin32_header(FrameType::Zfile, [0; 4]);
        wire.extend(
            encode_data_subpacket(&encode_zfile_metadata(name, 2_000_000, 0, 1), ZCRCW, true)
                .unwrap(),
        );
        session.feed(generation, &wire).unwrap();
    }

    fn has_part_file(directory: &Path) -> bool {
        std::fs::read_dir(directory).unwrap().any(|entry| {
            entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .ends_with(".part")
        })
    }

    #[test]
    fn cancel_and_timeout_finish_and_clean_even_when_can_write_fails() {
        for timeout in [false, true] {
            let directory = tempfile::tempdir().unwrap();
            let mut session =
                ZmodemSession::receiving(11, directory.path(), FakeTransport::default()).unwrap();
            begin_partial(&mut session, 11, "partial");
            assert!(has_part_file(directory.path()));
            let successful_writes = session.transport().writes().len();
            session.transport_mut().fail_after(successful_writes);

            let result = if timeout {
                session.timeout(11)
            } else {
                session.cancel(11)
            };
            assert!(result.is_err());
            assert!(session.is_finished());
            assert!(!has_part_file(directory.path()));
        }
    }

    #[test]
    fn data_overflow_finishes_and_cleans_even_when_can_write_fails() {
        let directory = tempfile::tempdir().unwrap();
        let mut session =
            ZmodemSession::receiving(12, directory.path(), FakeTransport::default()).unwrap();
        begin_partial(&mut session, 12, "overflow");
        session
            .feed(12, &encode_zbin32_header(FrameType::Zdata, [0; 4]))
            .unwrap();
        let successful_writes = session.transport().writes().len();
        session.transport_mut().fail_after(successful_writes);

        let oversized = vec![b'a'; crate::zmodem::DEFAULT_MAX_SUBPACKET_SIZE + 1];
        assert!(session.feed(12, &oversized).is_err());
        assert!(session.is_finished());
        assert!(!has_part_file(directory.path()));
    }

    #[test]
    fn fin_over_and_out_is_incremental_and_replays_non_terminator_bytes() {
        let directory = tempfile::tempdir().unwrap();
        let mut session =
            ZmodemSession::receiving(3, directory.path(), FakeTransport::default()).unwrap();
        session.start().unwrap();
        session
            .feed(3, &encode_zhex_header(FrameType::Zfin, [0; 4]))
            .unwrap();
        assert!(!session.is_finished());

        let first = session.feed(3, b"noiseO").unwrap();
        assert_eq!(first.replay, b"noise");
        assert_eq!(session.pending_metrics().0, 1);
        assert!(!session.is_finished());

        let second = session.feed(3, b"Otail").unwrap();
        assert!(session.is_finished());
        assert_eq!(second.replay, b"tail");
    }

    #[test]
    fn large_garbage_session_feed_compacts_per_chunk() {
        let directory = tempfile::tempdir().unwrap();
        let mut session =
            ZmodemSession::receiving(4, directory.path(), FakeTransport::default()).unwrap();
        let garbage = vec![b'x'; 2 * 1024 * 1024 - 9];
        session.feed(4, &garbage).unwrap();
        let (pending, compactions) = session.pending_metrics();
        assert_eq!(pending, 0);
        assert!(compactions <= garbage.len().div_ceil(FEED_CHUNK_BYTES) + 1);
    }
}
