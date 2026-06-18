// src-tauri/src/core/zmodem/encode.rs

/// CRC-16/XMODEM (polynomial 0x1021)
pub fn crc16(data: &[u8]) -> u16 {
    let mut crc: u16 = 0;
    for &b in data {
        crc ^= (b as u16) << 8;
        for _ in 0..8 {
            if crc & 0x8000 != 0 {
                crc = (crc << 1) ^ 0x1021;
            } else {
                crc <<= 1;
            }
        }
    }
    crc
}

/// CRC-32 (standard Ethernet polynomial, matching crc32fast)
pub fn crc32(data: &[u8]) -> u32 {
    crc32fast::hash(data)
}

/// CRC-32 over multiple slices in one pass
pub fn crc32_multi(slices: &[&[u8]]) -> u32 {
    let mut hasher = crc32fast::Hasher::new();
    for s in slices {
        hasher.update(s);
    }
    hasher.finalize()
}

use super::{ZPAD, ZDLE, ZHEX, ZBIN32, XON};

fn hex_byte(b: u8) -> [u8; 2] {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    [HEX[(b >> 4) as usize], HEX[(b & 0x0f) as usize]]
}

/// ZDLE-encode a single byte. Returns 1 byte (no escape) or 2 bytes (ZDLE + escaped).
///
/// Only escapes bytes whose escaped form `c ^ 0x40` is a VALID ZDLE escape, i.e.
/// the receiver's un-escape `(c & 0x60) == 0x40` must hold. That is true for
/// 0x00..=0x1f (→0x40..=0x5f) and 0x80..=0x9f (→0xc0..=0xdf), plus ZDLE itself.
/// 0x7f and 0xff would map to 0x3f/0xbf which are NOT valid escape codes — rz
/// rejects them and rewinds (ZRPOS storm). lrzsz uses special ZRUB0/ZRUB1 codes
/// for those; on an 8-bit-clean SSH channel we simply send them RAW, which rz
/// accepts as literal data.
fn zdle_encode_byte(b: u8) -> (u8, Option<u8>) {
    match b {
        ZDLE => (ZDLE, Some(0x58)),                // ZDLE itself: 0x18 ^ 0x40 = 0x58
        0x10 | 0x90 => (ZDLE, Some(b ^ 0x40)),    // DLE
        0x11 | 0x91 => (ZDLE, Some(b ^ 0x40)),    // XON
        0x13 | 0x93 => (ZDLE, Some(b ^ 0x40)),    // XOFF
        0x00..=0x1f => (ZDLE, Some(b ^ 0x40)),    // control chars
        0x80..=0x9f => (ZDLE, Some(b ^ 0x40)),    // high-bit control
        _ => (b, None),                             // pass through
    }
}

/// ZDLE-encode a data buffer
pub fn zdle_encode(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len() * 2);
    for &b in data {
        let (first, second) = zdle_encode_byte(b);
        out.push(first);
        if let Some(s) = second {
            out.push(s);
        }
    }
    out
}

/// Encode a ZHEX header: ZPAD ZPAD ZDLE ZHEX hex(type) hex(flags[0..4]) hex(crc16) CR LF XON
pub fn encode_zhex_header(frame_type: u8, flags: [u8; 4]) -> Vec<u8> {
    let mut out = Vec::with_capacity(32);
    out.push(ZPAD);
    out.push(ZPAD);
    out.push(ZDLE);
    out.push(ZHEX);

    let payload = [frame_type, flags[0], flags[1], flags[2], flags[3]];
    for &b in &payload {
        let h = hex_byte(b);
        out.push(h[0]);
        out.push(h[1]);
    }

    let crc = crc16(&payload);
    let h1 = hex_byte((crc >> 8) as u8);
    let h2 = hex_byte((crc & 0xff) as u8);
    out.push(h1[0]);
    out.push(h1[1]);
    out.push(h2[0]);
    out.push(h2[1]);

    out.push(b'\r');
    out.push(b'\n');
    // ZHEX headers are followed by XON unless it's ZFIN or ZACK
    if frame_type != super::FrameType::ZFIN as u8 && frame_type != super::FrameType::ZACK as u8 {
        out.push(XON);
    }
    out
}

/// Encode a ZBIN32 header: ZPAD ZDLE ZBIN32 ZDLE-encoded(type + flags[4] + crc32[4])
pub fn encode_zbin32_header(frame_type: u8, flags: [u8; 4]) -> Vec<u8> {
    let mut out = Vec::with_capacity(24);
    out.push(ZPAD);
    out.push(ZDLE);
    out.push(ZBIN32);

    let payload = [frame_type, flags[0], flags[1], flags[2], flags[3]];
    let crc = crc32(&payload);
    let crc_bytes = crc.to_le_bytes();

    // ZDLE-encode type + flags + crc32
    for &b in payload.iter().chain(crc_bytes.iter()) {
        let (first, second) = zdle_encode_byte(b);
        out.push(first);
        if let Some(s) = second {
            out.push(s);
        }
    }
    out
}

/// Encode a ZMODEM data subpacket with CRC-32.
/// `end_type` is one of ZCRCE, ZCRCG, ZCRCQ, ZCRCW.
pub fn encode_data_subpacket(data: &[u8], end_type: u8) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len() * 2 + 10);

    // ZDLE-encode the data bytes
    for &b in data {
        let (first, second) = zdle_encode_byte(b);
        out.push(first);
        if let Some(s) = second {
            out.push(s);
        }
    }

    // Append ZDLE + end_type marker
    out.push(ZDLE);
    out.push(end_type);

    // CRC-32 covers data + end_type (single pass)
    let crc = crc32_multi(&[data, &[end_type]]);
    let crc_bytes = crc.to_le_bytes();
    for &b in &crc_bytes {
        let (first, second) = zdle_encode_byte(b);
        out.push(first);
        if let Some(s) = second {
            out.push(s);
        }
    }
    out
}

/// Encode ZFILE metadata subpacket: "filename\0size mtime 0 0 0 filecount remaining\0"
pub fn encode_zfile_subpacket(name: &str, size: u64, mtime: u64) -> Vec<u8> {
    let info = format!("{}\0{} {} 0 0 0 1 0\0", name, size, mtime);
    info.into_bytes()
}

/// Cancel sequence: 8 CAN + 8 BS
pub fn zcancel() -> Vec<u8> {
    let mut out = Vec::with_capacity(16);
    out.extend([0x18u8; 8]);
    out.extend([0x08u8; 8]);
    out
}

/// "Over and Out": OO (sent after ZFIN exchange)
pub fn over_and_out() -> Vec<u8> {
    vec![b'O', b'O']
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_crc16_empty() {
        assert_eq!(crc16(b""), 0);
    }

    #[test]
    fn test_crc16_known() {
        // "123456789" has CRC-16/XMODEM = 0x31C3
        assert_eq!(crc16(b"123456789"), 0x31C3);
    }

    #[test]
    fn test_crc32_known() {
        // "123456789" has CRC-32 = 0xCBF43926
        assert_eq!(crc32(b"123456789"), 0xCBF43926);
    }

    #[test]
    fn test_zdle_encode_passthrough() {
        // Printable ASCII passes through
        assert_eq!(zdle_encode(b"hello"), b"hello".to_vec());
    }

    #[test]
    fn test_zdle_encode_control_chars() {
        // 0x00 → ZDLE 0x40, 0x0d → ZDLE 0x4d
        let encoded = zdle_encode(&[0x00, 0x0d]);
        assert_eq!(encoded, vec![0x18, 0x40, 0x18, 0x4d]);
    }

    #[test]
    fn test_zdle_encode_zdle_itself() {
        // ZDLE (0x18) → ZDLE 0x58
        let encoded = zdle_encode(&[0x18]);
        assert_eq!(encoded, vec![0x18, 0x58]);
    }

    #[test]
    fn test_zhex_header_roundtrip_structure() {
        let hdr = encode_zhex_header(0, [0, 0, 0, 0]); // ZRQINIT
        assert_eq!(hdr[0], b'*'); // ZPAD
        assert_eq!(hdr[1], b'*'); // ZPAD
        assert_eq!(hdr[2], 0x18); // ZDLE
        assert_eq!(hdr[3], b'B'); // ZHEX
        // Ends with CR LF XON
        let len = hdr.len();
        assert_eq!(hdr[len - 3], b'\r');
        assert_eq!(hdr[len - 2], b'\n');
        assert_eq!(hdr[len - 1], 0x11); // XON
    }

    #[test]
    fn test_zbin32_header_starts_correctly() {
        let hdr = encode_zbin32_header(10, [0, 0, 0, 0]); // ZDATA
        assert_eq!(hdr[0], b'*'); // ZPAD
        assert_eq!(hdr[1], 0x18); // ZDLE
        assert_eq!(hdr[2], b'C'); // ZBIN32
    }

    #[test]
    fn test_zfile_subpacket_format() {
        let pkt = encode_zfile_subpacket("test.txt", 1024, 1700000000);
        let s = String::from_utf8(pkt).unwrap();
        assert!(s.starts_with("test.txt\0"));
        assert!(s.contains("1024 1700000000"));
    }

    #[test]
    fn test_zcancel_length() {
        let c = zcancel();
        assert_eq!(c.len(), 16);
        assert!(c[..8].iter().all(|&b| b == 0x18));
        assert!(c[8..].iter().all(|&b| b == 0x08));
    }
}
