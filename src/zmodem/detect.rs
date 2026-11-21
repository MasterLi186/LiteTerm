use super::decode::{parse_header_prefix, HeaderParse};
use super::{DecodedFrame, FrameType, ZPAD};

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Detection {
    /// Bytes proven not to be an auto-receive trigger. Replay them to the
    /// terminal parser unchanged and in order.
    pub replay: Vec<u8>,
    /// A valid ZRQINIT is consumed and reported here.
    pub trigger: Option<DecodedFrame>,
    /// Bytes after the trigger belong to the new protocol session.
    pub trailing: Vec<u8>,
}

#[derive(Debug, Default)]
pub struct AutoReceiveDetector {
    pending: Vec<u8>,
    detected: bool,
    compactions: usize,
}

impl AutoReceiveDetector {
    const PENDING_LIMIT: usize = 4096;

    pub fn new() -> Self {
        Self::default()
    }

    pub fn feed(&mut self, bytes: &[u8]) -> Detection {
        if self.detected {
            return Detection {
                trailing: bytes.to_vec(),
                ..Detection::default()
            };
        }
        let mut output = Detection::default();
        let mut input_offset = 0;
        while input_offset < bytes.len() {
            if self.pending.len() == Self::PENDING_LIMIT {
                output.replay.push(self.pending[0]);
                self.compact(1);
            }
            let take = (Self::PENDING_LIMIT - self.pending.len()).min(bytes.len() - input_offset);
            self.pending
                .extend_from_slice(&bytes[input_offset..input_offset + take]);
            input_offset += take;

            let mut cursor = 0;
            loop {
                let Some(relative_start) =
                    self.pending[cursor..].iter().position(|byte| *byte == ZPAD)
                else {
                    output.replay.extend_from_slice(&self.pending[cursor..]);
                    cursor = self.pending.len();
                    break;
                };
                let start = cursor + relative_start;
                output
                    .replay
                    .extend_from_slice(&self.pending[cursor..start]);
                cursor = start;
                match parse_header_prefix(&self.pending[cursor..]) {
                    HeaderParse::Complete { frame, consumed }
                        if frame.frame_type == FrameType::Zrqinit =>
                    {
                        cursor += consumed;
                        output.trigger = Some(frame);
                        output.trailing.extend_from_slice(&self.pending[cursor..]);
                        output.trailing.extend_from_slice(&bytes[input_offset..]);
                        self.pending.clear();
                        self.detected = true;
                        return output;
                    }
                    HeaderParse::Complete { consumed, .. } => {
                        output
                            .replay
                            .extend_from_slice(&self.pending[cursor..cursor + consumed]);
                        cursor += consumed;
                    }
                    HeaderParse::NeedMore => break,
                    HeaderParse::Invalid => {
                        output.replay.push(self.pending[cursor]);
                        cursor += 1;
                    }
                }
            }
            self.compact(cursor);
        }
        output
    }

    pub fn reset(&mut self) -> Vec<u8> {
        self.detected = false;
        std::mem::take(&mut self.pending)
    }

    #[cfg(test)]
    fn compactions(&self) -> usize {
        self.compactions
    }

    fn compact(&mut self, consumed: usize) {
        if consumed == 0 {
            return;
        }
        self.pending.drain(..consumed);
        self.compactions += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::zmodem::encode::encode_zhex_header;

    #[test]
    fn false_candidates_replay_losslessly_across_chunks() {
        let input = b"hello **\x18B00000000000001zz tail";
        let mut detector = AutoReceiveDetector::new();
        let mut replay = Vec::new();
        for byte in input {
            replay.extend(detector.feed(&[*byte]).replay);
        }
        replay.extend(detector.reset());
        assert_eq!(replay, input);
    }

    #[test]
    fn only_valid_zrqinit_triggers_and_preserves_neighbors() {
        let zrinit = encode_zhex_header(FrameType::Zrinit, [0; 4]);
        let zrqinit = encode_zhex_header(FrameType::Zrqinit, [0; 4]);
        let mut input = b"prefix".to_vec();
        input.extend(zrinit.clone());
        input.extend(zrqinit);
        input.extend(b"next");
        let output = AutoReceiveDetector::new().feed(&input);
        let mut expected = b"prefix".to_vec();
        expected.extend(zrinit);
        assert_eq!(output.replay, expected);
        assert_eq!(output.trigger.unwrap().frame_type, FrameType::Zrqinit);
        assert_eq!(output.trailing, b"next");
    }

    #[test]
    fn large_garbage_replays_exactly_with_bounded_compactions() {
        let garbage = vec![b'x'; 2 * 1024 * 1024 - 31];
        let mut detector = AutoReceiveDetector::new();
        let output = detector.feed(&garbage);
        assert_eq!(output.replay, garbage);
        assert!(detector.pending.len() < 24);
        assert!(
            detector.compactions()
                <= garbage.len().div_ceil(AutoReceiveDetector::PENDING_LIMIT) + 1
        );
    }
}
