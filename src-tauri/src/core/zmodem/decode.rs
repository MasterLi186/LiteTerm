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

/// ZDLE 解码：还原发送方施加的转义。
/// 编码规则：ZDLE(0x18) → ZDLE 0x58；其余字节 → ZDLE (b ^ 0x40)。
/// 解码（逆过程）：0x58 → ZDLE(0x18)；其余转义字节 → escaped ^ 0x40。
fn zdle_decode_byte(escaped: u8) -> u8 {
    if escaped == 0x58 {
        ZDLE // ZDLE 自身被编码为 0x58
    } else {
        escaped ^ 0x40
    }
}

/// 增量式 ZMODEM 帧解码器，支持跨调用的分片输入。
pub struct ZmodemDecoder {
    buf: Vec<u8>,
}

impl ZmodemDecoder {
    pub fn new() -> Self {
        Self { buf: Vec::with_capacity(256) }
    }

    /// 输入来自远端的原始字节，返回所有已解析完整的帧。
    pub fn feed(&mut self, data: &[u8]) -> Vec<DecodedFrame> {
        self.buf.extend_from_slice(data);
        let mut frames = Vec::new();

        loop {
            // 查找下一个 ZPAD（潜在帧头的起始位置）
            let start = self.buf.iter().position(|&b| b == ZPAD);
            if start.is_none() {
                self.buf.clear();
                break;
            }
            let start = start.unwrap();

            // 丢弃 ZPAD 之前的字节
            if start > 0 {
                self.buf.drain(..start);
            }

            let len_before = self.buf.len();

            // 尝试解析 ZHEX 帧头：ZPAD ZPAD ZDLE ZHEX + 14 个十六进制字符
            if let Some(frame) = self.try_parse_zhex() {
                frames.push(frame);
                continue;
            }

            // 尝试解析 ZBIN32 帧头：ZPAD ZDLE ZBIN32 + ZDLE 编码的 9 字节
            if let Some(frame) = self.try_parse_zbin32() {
                frames.push(frame);
                continue;
            }

            // 若两个解析器都未消耗任何字节，说明需要更多数据或该 ZPAD 已过期。
            // 若自本轮开始后缓冲区有增长，等待更多数据可能有效。
            // 若缓冲区大小不变，该 ZPAD 无法开始一个已知帧（如 ZBIN 或乱码），
            // 跳过以防止无限停滞。
            if self.buf.len() == len_before {
                // 数据太短无法分类：单独的 ZPAD（或 ZPAD ZPAD）——等待更多数据，
                // 而不是直接丢弃（它可能是合法帧的开头）。
                if self.buf.len() < 3 {
                    break;
                }

                // 检查是否只是需要更多数据（未达到最小帧大小）
                let could_be_zhex = self.buf[0] == ZPAD && self.buf[1] == ZPAD
                    && self.buf[2] == ZDLE
                    && (self.buf.len() < 4 || self.buf[3] == ZHEX);
                let could_be_zbin32 = self.buf[0] == ZPAD && self.buf[1] == ZDLE
                    && self.buf[2] == ZBIN32;
                if could_be_zhex || could_be_zbin32 {
                    // 可能是合法帧，只是数据尚不完整
                    break;
                }
                // 无法识别或已过期的 ZPAD——跳过
                self.buf.drain(..1);
            }
        }

        frames
    }

    fn try_parse_zhex(&mut self) -> Option<DecodedFrame> {
        // 至少需要：ZPAD ZPAD ZDLE ZHEX + 14 个十六进制字符 = 18 字节
        if self.buf.len() < 18 {
            return None;
        }
        if self.buf[0] != ZPAD || self.buf[1] != ZPAD || self.buf[2] != ZDLE || self.buf[3] != ZHEX {
            return None;
        }

        // 在可变借用 self 执行 drain 之前，先复制十六进制字节
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
            // CRC 不匹配——跳过此 ZPAD，尝试下一个
            self.buf.drain(..1);
            return None;
        }

        // 在检查帧类型之前，先消费帧头及末尾的 CR LF（以及可选的 XON），
        // 确保即使帧类型未知，缓冲区也始终向前推进。
        let mut consumed = 18;
        while consumed < self.buf.len() && (self.buf[consumed] == b'\r' || self.buf[consumed] == b'\n' || self.buf[consumed] == 0x11) {
            consumed += 1;
        }
        self.buf.drain(..consumed);

        let frame_type = FrameType::from_u8(frame_type_val)?;

        Some(DecodedFrame { frame_type, flags: [f0, f1, f2, f3] })
    }

    fn try_parse_zbin32(&mut self) -> Option<DecodedFrame> {
        // ZPAD ZDLE ZBIN32 + ZDLE编码(type[1] + flags[4] + crc32[4]) = 最少 12 字节，最多约 21 字节
        if self.buf.len() < 12 {
            return None;
        }
        if !(self.buf[0] == ZPAD && self.buf[1] == ZDLE && self.buf[2] == ZBIN32) {
            return None;
        }

        // 从偏移量 3 开始，ZDLE 解码 9 字节（type + 4 flags + 4 crc）
        let mut decoded = Vec::with_capacity(9);
        let mut i = 3;
        while decoded.len() < 9 && i < self.buf.len() {
            if self.buf[i] == ZDLE {
                if i + 1 >= self.buf.len() {
                    return None; // 需要更多数据
                }
                decoded.push(zdle_decode_byte(self.buf[i + 1]));
                i += 2;
            } else {
                decoded.push(self.buf[i]);
                i += 1;
            }
        }

        if decoded.len() < 9 {
            return None; // 需要更多数据
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

        // 在检查帧类型之前先清空缓冲区，确保即使帧类型未知也始终向前推进。
        self.buf.drain(..i);

        let frame_type = FrameType::from_u8(frame_type_val)?;

        Some(DecodedFrame { frame_type, flags })
    }

    /// 检测 5 个或更多连续的 CAN 字节（远端发起的中止信号）
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
