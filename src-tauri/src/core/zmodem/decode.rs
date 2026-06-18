// src-tauri/src/core/zmodem/decode.rs

use super::{DecodedFrame, FrameType, ZPAD, ZDLE, ZHEX, ZBIN32};
use super::encode::{crc16, crc32};

fn from_hex(c: u8) -> Option<u8> {
    match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'f' => Some(c - b'a' + 10),
        b'A'..=b'F' => Some(c - b'A' + 10),
        _ => None,
    }
}

fn hex_pair(h: u8, l: u8) -> Option<u8> {
    Some((from_hex(h)? << 4) | from_hex(l)?)
}

/// ZDLE-decode: reverse the escaping applied by the sender.
/// Encoder rule: ZDLE(0x18) → ZDLE 0x58; everything else → ZDLE (b ^ 0x40).
/// Decoder (inverse): 0x58 → ZDLE(0x18); anything else → escaped ^ 0x40.
fn zdle_decode_byte(escaped: u8) -> u8 {
    if escaped == 0x58 {
        ZDLE // ZDLE itself was encoded as 0x58
    } else {
        escaped ^ 0x40
    }
}

/// Incremental ZMODEM frame decoder. Handles fragmented input across calls.
pub struct ZmodemDecoder {
    buf: Vec<u8>,
}

impl ZmodemDecoder {
    pub fn new() -> Self {
        Self { buf: Vec::with_capacity(256) }
    }

    /// Feed raw bytes from the remote. Returns all complete frames parsed.
    pub fn feed(&mut self, data: &[u8]) -> Vec<DecodedFrame> {
        self.buf.extend_from_slice(data);
        let mut frames = Vec::new();

        loop {
            // Find the next ZPAD (start of a potential header)
            let start = self.buf.iter().position(|&b| b == ZPAD);
            if start.is_none() {
                self.buf.clear();
                break;
            }
            let start = start.unwrap();

            // Discard bytes before the ZPAD
            if start > 0 {
                self.buf.drain(..start);
            }

            let len_before = self.buf.len();

            // Try to parse ZHEX header: ZPAD ZPAD ZDLE ZHEX + 14 hex chars
            if let Some(frame) = self.try_parse_zhex() {
                frames.push(frame);
                continue;
            }

            // Try to parse ZBIN32 header: ZPAD ZDLE ZBIN32 + ZDLE-encoded 9 bytes
            if let Some(frame) = self.try_parse_zbin32() {
                frames.push(frame);
                continue;
            }

            // If neither parser consumed any bytes, we need more data or the ZPAD is stale.
            // If the buffer grew since we entered this iteration, more data might help — wait.
            // If buffer size is unchanged, this ZPAD cannot start a known frame (e.g. ZBIN/garbage);
            // skip it to prevent an infinite stall.
            if self.buf.len() == len_before {
                // Too short to even classify: a lone ZPAD (or ZPAD ZPAD) — wait for more data
                // rather than discarding it (it could be the start of a valid frame).
                if self.buf.len() < 3 {
                    break;
                }

                // Check if we might just need more data (minimum frame sizes not met)
                let could_be_zhex = self.buf[0] == ZPAD && self.buf[1] == ZPAD
                    && self.buf[2] == ZDLE
                    && (self.buf.len() < 4 || self.buf[3] == ZHEX);
                let could_be_zbin32 = self.buf[0] == ZPAD && self.buf[1] == ZDLE
                    && self.buf[2] == ZBIN32;
                if could_be_zhex || could_be_zbin32 {
                    // Might be a valid frame, just not enough data yet
                    break;
                }
                // Unrecognized or stale ZPAD — skip it
                self.buf.drain(..1);
            }
        }

        frames
    }

    fn try_parse_zhex(&mut self) -> Option<DecodedFrame> {
        // Need at least: ZPAD ZPAD ZDLE ZHEX + 14 hex chars = 18 bytes
        if self.buf.len() < 18 {
            return None;
        }
        if self.buf[0] != ZPAD || self.buf[1] != ZPAD || self.buf[2] != ZDLE || self.buf[3] != ZHEX {
            return None;
        }

        // Copy hex bytes out before borrowing self mutably for drain
        let hex: [u8; 14] = self.buf[4..18].try_into().unwrap();

        macro_rules! hex_pair_or_skip {
            ($h:expr, $l:expr) => {
                match hex_pair(hex[$h], hex[$l]) {
                    Some(v) => v,
                    None => { self.buf.drain(..1); return None; }
                }
            };
        }

        let frame_type_val = hex_pair_or_skip!(0, 1);
        let f0 = hex_pair_or_skip!(2, 3);
        let f1 = hex_pair_or_skip!(4, 5);
        let f2 = hex_pair_or_skip!(6, 7);
        let f3 = hex_pair_or_skip!(8, 9);
        let crc_hi = hex_pair_or_skip!(10, 11);
        let crc_lo = hex_pair_or_skip!(12, 13);

        let received_crc = ((crc_hi as u16) << 8) | crc_lo as u16;
        let payload = [frame_type_val, f0, f1, f2, f3];
        let computed_crc = crc16(&payload);

        if received_crc != computed_crc {
            // CRC mismatch — skip this ZPAD and try next
            self.buf.drain(..1);
            return None;
        }

        // Consume the header + trailing CR LF (and optional XON) before checking frame type,
        // so buffer always advances even if frame type is unknown.
        let mut consumed = 18;
        while consumed < self.buf.len() && (self.buf[consumed] == b'\r' || self.buf[consumed] == b'\n' || self.buf[consumed] == 0x11) {
            consumed += 1;
        }
        self.buf.drain(..consumed);

        let frame_type = FrameType::from_u8(frame_type_val)?;

        Some(DecodedFrame { frame_type, flags: [f0, f1, f2, f3] })
    }

    fn try_parse_zbin32(&mut self) -> Option<DecodedFrame> {
        // ZPAD ZDLE ZBIN32 + ZDLE-encoded(type[1] + flags[4] + crc32[4]) = min 12, max ~21 bytes
        if self.buf.len() < 12 {
            return None;
        }
        if !(self.buf[0] == ZPAD && self.buf[1] == ZDLE && self.buf[2] == ZBIN32) {
            return None;
        }

        // ZDLE-decode 9 bytes (type + 4 flags + 4 crc) starting at offset 3
        let mut decoded = Vec::with_capacity(9);
        let mut i = 3;
        while decoded.len() < 9 && i < self.buf.len() {
            if self.buf[i] == ZDLE {
                if i + 1 >= self.buf.len() {
                    return None; // need more data
                }
                decoded.push(zdle_decode_byte(self.buf[i + 1]));
                i += 2;
            } else {
                decoded.push(self.buf[i]);
                i += 1;
            }
        }

        if decoded.len() < 9 {
            return None; // need more data
        }

        let frame_type_val = decoded[0];
        let flags = [decoded[1], decoded[2], decoded[3], decoded[4]];
        let received_crc = u32::from_le_bytes([decoded[5], decoded[6], decoded[7], decoded[8]]);

        let payload = [frame_type_val, flags[0], flags[1], flags[2], flags[3]];
        let computed_crc = crc32(&payload);

        if received_crc != computed_crc {
            self.buf.drain(..1);
            return None;
        }

        // Drain buffer before checking frame type so buffer always advances even if unknown.
        self.buf.drain(..i);

        let frame_type = FrameType::from_u8(frame_type_val)?;

        Some(DecodedFrame { frame_type, flags })
    }

    /// Detect 5+ consecutive CAN bytes (abort from remote)
    pub fn detect_cancel(data: &[u8]) -> bool {
        let mut count = 0;
        for &b in data {
            if b == 0x18 {
                count += 1;
                if count >= 5 {
                    return true;
                }
            } else {
                count = 0;
            }
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::encode::encode_zhex_header;

    #[test]
    fn test_decode_zhex_zrinit() {
        let raw = encode_zhex_header(FrameType::ZRINIT as u8, [0x00, 0x00, 0x00, 0x23]);
        let mut dec = ZmodemDecoder::new();
        let frames = dec.feed(&raw);
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].frame_type, FrameType::ZRINIT);
        assert_eq!(frames[0].flags, [0x00, 0x00, 0x00, 0x23]);
    }

    #[test]
    fn test_decode_zhex_fragmented() {
        let raw = encode_zhex_header(FrameType::ZRPOS as u8, [0x00, 0x10, 0x00, 0x00]);
        let mut dec = ZmodemDecoder::new();
        // Feed first 10 bytes, then the rest
        let frames1 = dec.feed(&raw[..10]);
        assert_eq!(frames1.len(), 0);
        let frames2 = dec.feed(&raw[10..]);
        assert_eq!(frames2.len(), 1);
        assert_eq!(frames2[0].frame_type, FrameType::ZRPOS);
        assert_eq!(frames2[0].offset(), 0x00001000);
    }

    #[test]
    fn test_decode_zbin32_header() {
        use super::super::encode::encode_zbin32_header;
        let raw = encode_zbin32_header(FrameType::ZRINIT as u8, [0x01, 0x02, 0x00, 0x20]);
        let mut dec = ZmodemDecoder::new();
        let frames = dec.feed(&raw);
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].frame_type, FrameType::ZRINIT);
        assert_eq!(frames[0].flags, [0x01, 0x02, 0x00, 0x20]);
    }

    #[test]
    fn test_detect_cancel() {
        assert!(ZmodemDecoder::detect_cancel(&[0x18, 0x18, 0x18, 0x18, 0x18]));
        assert!(!ZmodemDecoder::detect_cancel(&[0x18, 0x18, 0x18, 0x18]));
        assert!(ZmodemDecoder::detect_cancel(&[0x41, 0x18, 0x18, 0x18, 0x18, 0x18, 0x08]));
    }

    #[test]
    fn test_decode_garbage_before_header() {
        let mut raw = vec![0x41, 0x42, 0x43]; // garbage
        raw.extend_from_slice(&encode_zhex_header(FrameType::ZFIN as u8, [0; 4]));
        let mut dec = ZmodemDecoder::new();
        let frames = dec.feed(&raw);
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].frame_type, FrameType::ZFIN);
    }
}
