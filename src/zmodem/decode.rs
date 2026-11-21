use super::encode::{crc16, crc32, crc32_multi};
use super::{
    ChecksumMode, DecodedFrame, FrameType, HeaderFormat, DEFAULT_MAX_SUBPACKET_SIZE, ZBIN, ZBIN32,
    ZCRCE, ZCRCG, ZCRCQ, ZCRCW, ZDLE, ZHEX, ZPAD, ZRUB0, ZRUB1,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HeaderParse {
    Complete {
        frame: DecodedFrame,
        consumed: usize,
    },
    NeedMore,
    Invalid,
}

fn hex_nibble(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

fn hex_pair(high: u8, low: u8) -> Option<u8> {
    Some((hex_nibble(high)? << 4) | hex_nibble(low)?)
}

fn decode_escaped_byte(bytes: &[u8], index: &mut usize) -> Result<Option<u8>, ()> {
    let Some(&byte) = bytes.get(*index) else {
        return Ok(None);
    };
    *index += 1;
    if byte != ZDLE {
        return Ok(Some(byte));
    }
    let Some(&escaped) = bytes.get(*index) else {
        *index -= 1;
        return Ok(None);
    };
    *index += 1;
    let decoded = match escaped {
        ZRUB0 => 0x7f,
        ZRUB1 => 0xff,
        value if value & 0x60 == 0x40 => value ^ 0x40,
        _ => return Err(()),
    };
    Ok(Some(decoded))
}

/// Parse exactly one header at the beginning of `bytes`.
///
/// The returned `consumed` count includes any already-present CR/LF/XON suffix
/// of a ZHEX header. It never consumes bytes merely because more input is
/// needed, which lets the detector replay false positives byte-for-byte.
pub fn parse_header_prefix(bytes: &[u8]) -> HeaderParse {
    if bytes.is_empty() || bytes == [ZPAD] {
        return HeaderParse::NeedMore;
    }
    if bytes[0] != ZPAD {
        return HeaderParse::Invalid;
    }

    if bytes[1] == ZPAD {
        if bytes.len() < 4 {
            return if bytes.get(2).is_none_or(|byte| *byte == ZDLE) {
                HeaderParse::NeedMore
            } else {
                HeaderParse::Invalid
            };
        }
        if bytes[2] != ZDLE || bytes[3] != ZHEX {
            return HeaderParse::Invalid;
        }
        if bytes.len() < 18 {
            return HeaderParse::NeedMore;
        }
        let mut decoded = [0u8; 7];
        for (index, output) in decoded.iter_mut().enumerate() {
            let offset = 4 + index * 2;
            let Some(value) = hex_pair(bytes[offset], bytes[offset + 1]) else {
                return HeaderParse::Invalid;
            };
            *output = value;
        }
        let payload = &decoded[..5];
        if crc16(payload) != u16::from_be_bytes([decoded[5], decoded[6]]) {
            return HeaderParse::Invalid;
        }
        let Some(frame_type) = FrameType::from_u8(decoded[0]) else {
            return HeaderParse::Invalid;
        };
        let mut consumed = 18;
        while bytes
            .get(consumed)
            .is_some_and(|byte| matches!(*byte, b'\r' | b'\n' | super::XON))
        {
            consumed += 1;
        }
        return HeaderParse::Complete {
            frame: DecodedFrame::with_format(
                frame_type,
                [decoded[1], decoded[2], decoded[3], decoded[4]],
                HeaderFormat::Hex,
            ),
            consumed,
        };
    }

    if bytes[1] != ZDLE {
        return HeaderParse::Invalid;
    }
    let Some(&format) = bytes.get(2) else {
        return HeaderParse::NeedMore;
    };
    let (decoded_len, header_format) = match format {
        ZBIN => (7, HeaderFormat::Binary16),
        ZBIN32 => (9, HeaderFormat::Binary32),
        _ => return HeaderParse::Invalid,
    };
    let mut index = 3;
    let mut decoded = [0u8; 9];
    for output in &mut decoded[..decoded_len] {
        match decode_escaped_byte(bytes, &mut index) {
            Ok(Some(value)) => *output = value,
            Ok(None) => return HeaderParse::NeedMore,
            Err(()) => return HeaderParse::Invalid,
        }
    }
    match header_format {
        HeaderFormat::Binary16 => {
            if crc16(&decoded[..5]) != u16::from_be_bytes([decoded[5], decoded[6]]) {
                return HeaderParse::Invalid;
            }
        }
        HeaderFormat::Binary32 => {
            if crc32(&decoded[..5]) != u32::from_le_bytes(decoded[5..9].try_into().unwrap()) {
                return HeaderParse::Invalid;
            }
        }
        HeaderFormat::Hex => unreachable!("binary prefix cannot produce a hexadecimal header"),
    }
    let Some(frame_type) = FrameType::from_u8(decoded[0]) else {
        return HeaderParse::Invalid;
    };
    HeaderParse::Complete {
        frame: DecodedFrame::with_format(
            frame_type,
            [decoded[1], decoded[2], decoded[3], decoded[4]],
            header_format,
        ),
        consumed: index,
    }
}

#[derive(Debug, Default)]
pub struct HeaderDecoder {
    buffer: Vec<u8>,
    compactions: usize,
}

impl HeaderDecoder {
    const BUFFER_LIMIT: usize = 4096;

    pub fn new() -> Self {
        Self::default()
    }

    pub fn feed(&mut self, bytes: &[u8]) -> Vec<DecodedFrame> {
        let mut frames = Vec::new();
        let mut input_offset = 0;
        while input_offset < bytes.len() {
            let available = Self::BUFFER_LIMIT.saturating_sub(self.buffer.len());
            if available == 0 {
                // No valid supported header needs this much lookahead. Replaying
                // one byte through the scanner guarantees bounded progress.
                self.compact(1);
            }
            let take = (Self::BUFFER_LIMIT - self.buffer.len()).min(bytes.len() - input_offset);
            self.buffer
                .extend_from_slice(&bytes[input_offset..input_offset + take]);
            input_offset += take;

            let mut cursor = 0;
            loop {
                let Some(relative_start) =
                    self.buffer[cursor..].iter().position(|byte| *byte == ZPAD)
                else {
                    cursor = self.buffer.len();
                    break;
                };
                cursor += relative_start;
                match parse_header_prefix(&self.buffer[cursor..]) {
                    HeaderParse::Complete { frame, consumed } => {
                        cursor += consumed;
                        frames.push(frame);
                    }
                    HeaderParse::NeedMore => break,
                    HeaderParse::Invalid => cursor += 1,
                }
            }
            self.compact(cursor);
        }
        frames
    }

    pub fn buffered_len(&self) -> usize {
        self.buffer.len()
    }

    #[cfg(test)]
    pub fn compactions(&self) -> usize {
        self.compactions
    }

    fn compact(&mut self, consumed: usize) {
        if consumed == 0 {
            return;
        }
        self.buffer.drain(..consumed);
        self.compactions += 1;
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DataEnd {
    End,
    Continue,
    ContinueAck,
    EndAck,
}

impl DataEnd {
    pub fn from_wire(value: u8) -> Option<Self> {
        Some(match value {
            ZCRCE => Self::End,
            ZCRCG => Self::Continue,
            ZCRCQ => Self::ContinueAck,
            ZCRCW => Self::EndAck,
            _ => return None,
        })
    }

    pub const fn to_wire(self) -> u8 {
        match self {
            Self::End => ZCRCE,
            Self::Continue => ZCRCG,
            Self::ContinueAck => ZCRCQ,
            Self::EndAck => ZCRCW,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DataSubpacket {
    pub data: Vec<u8>,
    pub end: DataEnd,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DataDecodeError {
    InvalidEscape(u8),
    CrcMismatch { expected: u32, actual: u32 },
    TooLarge { limit: usize },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DataFeed {
    pub consumed: usize,
    pub result: Option<Result<DataSubpacket, DataDecodeError>>,
}

#[derive(Debug)]
pub struct DataSubpacketDecoder {
    data: Vec<u8>,
    terminator: Option<DataEnd>,
    crc: Vec<u8>,
    escaped: bool,
    limit: usize,
    checksum_mode: ChecksumMode,
}

impl Default for DataSubpacketDecoder {
    fn default() -> Self {
        Self::new(DEFAULT_MAX_SUBPACKET_SIZE)
    }
}

impl DataSubpacketDecoder {
    pub fn new(limit: usize) -> Self {
        Self {
            data: Vec::new(),
            terminator: None,
            crc: Vec::with_capacity(4),
            escaped: false,
            limit,
            checksum_mode: ChecksumMode::Crc32,
        }
    }

    pub fn set_checksum_mode(&mut self, checksum_mode: ChecksumMode) {
        self.reset();
        self.checksum_mode = checksum_mode;
    }

    pub fn reset(&mut self) {
        self.data.clear();
        self.terminator = None;
        self.crc.clear();
        self.escaped = false;
    }

    fn fail(&mut self, consumed: usize, error: DataDecodeError) -> DataFeed {
        self.reset();
        DataFeed {
            consumed,
            result: Some(Err(error)),
        }
    }

    /// Consume at most one complete data subpacket.
    ///
    /// `consumed` may be smaller than `bytes.len()`; the caller must feed the
    /// remainder according to the state transition caused by the packet.
    pub fn feed_one(&mut self, bytes: &[u8]) -> DataFeed {
        let mut index = 0;
        while index < bytes.len() {
            let byte = bytes[index];
            index += 1;
            if self.escaped {
                self.escaped = false;
                if self.terminator.is_none() {
                    if let Some(end) = DataEnd::from_wire(byte) {
                        self.terminator = Some(end);
                        continue;
                    }
                }
                let decoded = match byte {
                    ZRUB0 => 0x7f,
                    ZRUB1 => 0xff,
                    value if value & 0x60 == 0x40 => value ^ 0x40,
                    _ => return self.fail(index, DataDecodeError::InvalidEscape(byte)),
                };
                if self.terminator.is_some() {
                    self.crc.push(decoded);
                } else {
                    self.data.push(decoded);
                }
            } else if byte == ZDLE {
                self.escaped = true;
                continue;
            } else if self.terminator.is_some() {
                self.crc.push(byte);
            } else {
                self.data.push(byte);
            }

            if self.data.len() > self.limit {
                return self.fail(index, DataDecodeError::TooLarge { limit: self.limit });
            }
            let checksum_len = match self.checksum_mode {
                ChecksumMode::Crc16 => 2,
                ChecksumMode::Crc32 => 4,
            };
            if self.crc.len() == checksum_len {
                let end = self
                    .terminator
                    .expect("CRC is collected only after a terminator");
                let (expected, actual) = match self.checksum_mode {
                    ChecksumMode::Crc16 => {
                        let mut payload = Vec::with_capacity(self.data.len().saturating_add(1));
                        payload.extend_from_slice(&self.data);
                        payload.push(end.to_wire());
                        (
                            u32::from(crc16(&payload)),
                            u32::from(u16::from_be_bytes([self.crc[0], self.crc[1]])),
                        )
                    }
                    ChecksumMode::Crc32 => (
                        crc32_multi(&[&self.data, &[end.to_wire()]]),
                        u32::from_le_bytes(self.crc[..4].try_into().unwrap()),
                    ),
                };
                if actual != expected {
                    return self.fail(index, DataDecodeError::CrcMismatch { expected, actual });
                }
                let packet = DataSubpacket {
                    data: std::mem::take(&mut self.data),
                    end,
                };
                self.reset();
                return DataFeed {
                    consumed: index,
                    result: Some(Ok(packet)),
                };
            }
        }
        DataFeed {
            consumed: index,
            result: None,
        }
    }
}

#[derive(Debug, Default)]
pub struct CancelDetector {
    consecutive: usize,
}

impl CancelDetector {
    pub fn feed(&mut self, bytes: &[u8]) -> bool {
        for &byte in bytes {
            if byte == ZDLE {
                self.consecutive += 1;
                if self.consecutive >= 5 {
                    return true;
                }
            } else {
                self.consecutive = 0;
            }
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::zmodem::encode::{
        encode_data_subpacket, encode_data_subpacket_with_checksum, encode_zbin16_header,
        encode_zbin32_header, encode_zhex_header,
    };

    #[test]
    fn headers_decode_one_byte_at_a_time() {
        for raw in [
            encode_zhex_header(FrameType::Zrpos, 4096u32.to_le_bytes()),
            encode_zbin16_header(FrameType::Zfile, [0; 4]),
            encode_zbin32_header(FrameType::Zdata, 17u32.to_le_bytes()),
        ] {
            let mut decoder = HeaderDecoder::new();
            let mut decoded = Vec::new();
            for byte in raw {
                decoded.extend(decoder.feed(&[byte]));
            }
            assert_eq!(decoded.len(), 1);
        }
    }

    #[test]
    fn crc16_data_packet_decodes_across_every_boundary() {
        let payload = [0, ZDLE, 0x7f, 0xff, b'a'];
        let raw = encode_data_subpacket_with_checksum(&payload, ZCRCW, true, ChecksumMode::Crc16)
            .unwrap();
        for split in 0..raw.len() {
            let mut decoder = DataSubpacketDecoder::default();
            decoder.set_checksum_mode(ChecksumMode::Crc16);
            assert!(decoder.feed_one(&raw[..split]).result.is_none());
            let packet = decoder.feed_one(&raw[split..]).result.unwrap().unwrap();
            assert_eq!(packet.data, payload);
            assert_eq!(packet.end, DataEnd::EndAck);
        }
    }

    #[test]
    fn garbage_and_bad_crc_do_not_hide_following_header() {
        let mut raw = b"noise***not-a-header".to_vec();
        raw.extend(encode_zhex_header(FrameType::Zfin, [0; 4]));
        let frames = HeaderDecoder::new().feed(&raw);
        assert_eq!(frames, vec![DecodedFrame::new(FrameType::Zfin, [0; 4])]);
    }

    #[test]
    fn data_packet_decodes_across_every_boundary() {
        let payload = [0, ZDLE, 0x7f, 0xff, b'a'];
        let raw = encode_data_subpacket(&payload, ZCRCW, true).unwrap();
        for split in 0..raw.len() {
            let mut decoder = DataSubpacketDecoder::default();
            assert!(decoder.feed_one(&raw[..split]).result.is_none());
            let result = decoder.feed_one(&raw[split..]).result.unwrap().unwrap();
            assert_eq!(result.data, payload);
            assert_eq!(result.end, DataEnd::EndAck);
        }
        let result = DataSubpacketDecoder::default()
            .feed_one(&raw)
            .result
            .unwrap()
            .unwrap();
        assert_eq!(result.data, payload);
    }

    #[test]
    fn cancel_detection_crosses_chunks() {
        let mut detector = CancelDetector::default();
        assert!(!detector.feed(&[ZDLE; 4]));
        assert!(detector.feed(&[ZDLE]));
    }

    #[test]
    fn large_garbage_feed_stays_bounded_and_compacts_per_chunk_not_byte() {
        let mut decoder = HeaderDecoder::new();
        let garbage = vec![b'x'; 2 * 1024 * 1024 - 17];
        assert!(decoder.feed(&garbage).is_empty());
        assert_eq!(decoder.buffered_len(), 0);
        assert!(decoder.compactions() <= garbage.len().div_ceil(HeaderDecoder::BUFFER_LIMIT) + 1);
    }
}
