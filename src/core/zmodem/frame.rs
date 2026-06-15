/// ZMODEM frame encoding/decoding with CRC-16/XMODEM.
///
/// Implements the ZHEX header format used in ZMODEM file transfer protocol.
/// Each hex header is structured as: ZPAD ZPAD ZDLE ZHEX type[1] flags[4] crc16[2] CR LF
/// where all payload bytes are transmitted as two ASCII hex digits.

/// ZMODEM frame types.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum FrameType {
    ZRQINIT = 0,
    ZRINIT = 1,
    ZSINIT = 2,
    ZACK = 3,
    ZFILE = 4,
    ZSKIP = 5,
    ZNAK = 6,
    ZABORT = 7,
    ZFIN = 8,
    ZRPOS = 9,
    ZDATA = 10,
    ZEOF = 11,
    ZFERR = 12,
    ZCRC = 13,
    ZCHALLENGE = 14,
    ZCOMPL = 15,
    ZCAN = 16,
}

impl FrameType {
    /// Convert a raw byte value to a `FrameType`, returning `None` for unknown values.
    pub fn from_u8(v: u8) -> Option<Self> {
        match v {
            0 => Some(Self::ZRQINIT),
            1 => Some(Self::ZRINIT),
            2 => Some(Self::ZSINIT),
            3 => Some(Self::ZACK),
            4 => Some(Self::ZFILE),
            5 => Some(Self::ZSKIP),
            6 => Some(Self::ZNAK),
            7 => Some(Self::ZABORT),
            8 => Some(Self::ZFIN),
            9 => Some(Self::ZRPOS),
            10 => Some(Self::ZDATA),
            11 => Some(Self::ZEOF),
            12 => Some(Self::ZFERR),
            13 => Some(Self::ZCRC),
            14 => Some(Self::ZCHALLENGE),
            15 => Some(Self::ZCOMPL),
            16 => Some(Self::ZCAN),
            _ => None,
        }
    }
}

/// A ZMODEM frame consisting of a type byte and four flag bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ZmodemFrame {
    pub frame_type: FrameType,
    pub flags: [u8; 4],
}

// ---------------------------------------------------------------------------
// CRC-16/XMODEM (polynomial 0x1021)
// ---------------------------------------------------------------------------

/// Compute CRC-16/XMODEM over `data` using polynomial 0x1021.
pub fn crc16(data: &[u8]) -> u16 {
    let mut crc: u16 = 0;
    for &byte in data {
        crc ^= (byte as u16) << 8;
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

// ---------------------------------------------------------------------------
// Hex helpers
// ---------------------------------------------------------------------------

/// Encode a single byte as two lowercase ASCII hex digits.
fn hex_byte(b: u8) -> [u8; 2] {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    [HEX[(b >> 4) as usize], HEX[(b & 0x0f) as usize]]
}

/// Decode a pair of ASCII hex digits into a byte. Returns `None` on invalid input.
fn from_hex_pair(h: u8, l: u8) -> Option<u8> {
    let high = hex_digit(h)?;
    let low = hex_digit(l)?;
    Some((high << 4) | low)
}

/// Convert a single ASCII hex character to its 4-bit value.
fn hex_digit(c: u8) -> Option<u8> {
    match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'f' => Some(c - b'a' + 10),
        b'A'..=b'F' => Some(c - b'A' + 10),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// ZHEX header encoding
// ---------------------------------------------------------------------------

/// ZPAD character — `*` (0x2A).
const ZPAD: u8 = b'*';
/// ZDLE character — CAN (0x18).
const ZDLE: u8 = 0x18;
/// ZHEX encoding type marker — `B` (0x42).
const ZHEX: u8 = b'B';

/// Encode a `ZmodemFrame` as a ZHEX header.
///
/// Format: `ZPAD ZPAD ZDLE ZHEX hex(type) hex(flags[0..4]) hex(crc16_hi) hex(crc16_lo) CR LF`
///
/// The CRC-16 is computed over the 5-byte sequence `[type, flags[0], flags[1], flags[2], flags[3]]`.
pub fn encode_zhex_header(frame: &ZmodemFrame) -> Vec<u8> {
    let mut out = Vec::with_capacity(32);

    // Preamble
    out.push(ZPAD);
    out.push(ZPAD);
    out.push(ZDLE);
    out.push(ZHEX);

    // Payload: type byte + 4 flag bytes
    let payload: [u8; 5] = [
        frame.frame_type as u8,
        frame.flags[0],
        frame.flags[1],
        frame.flags[2],
        frame.flags[3],
    ];

    for &b in &payload {
        let h = hex_byte(b);
        out.push(h[0]);
        out.push(h[1]);
    }

    // CRC-16 over the payload
    let crc = crc16(&payload);
    let crc_hi = (crc >> 8) as u8;
    let crc_lo = (crc & 0xff) as u8;
    let h1 = hex_byte(crc_hi);
    let h2 = hex_byte(crc_lo);
    out.push(h1[0]);
    out.push(h1[1]);
    out.push(h2[0]);
    out.push(h2[1]);

    // Terminator
    out.push(b'\r');
    out.push(b'\n');

    out
}

// ---------------------------------------------------------------------------
// ZHEX header decoding
// ---------------------------------------------------------------------------

/// Decode a ZHEX header from raw bytes.
///
/// Searches for the `ZPAD ZPAD ZDLE ZHEX` preamble, then reads 14 hex characters
/// (type[2] + flags[8] + crc[4]), and verifies the CRC-16.
///
/// Returns `None` if the preamble is not found, hex decoding fails, or CRC does not match.
pub fn decode_zhex_header(data: &[u8]) -> Option<ZmodemFrame> {
    // Find the preamble
    let preamble = [ZPAD, ZPAD, ZDLE, ZHEX];
    let pos = data
        .windows(4)
        .position(|w| w == preamble)?;

    let hex_start = pos + 4;
    // Need 14 hex characters: 2 (type) + 8 (flags) + 4 (crc)
    if data.len() < hex_start + 14 {
        return None;
    }

    let hex = &data[hex_start..hex_start + 14];

    // Decode type
    let frame_type_val = from_hex_pair(hex[0], hex[1])?;
    let frame_type = FrameType::from_u8(frame_type_val)?;

    // Decode flags
    let f0 = from_hex_pair(hex[2], hex[3])?;
    let f1 = from_hex_pair(hex[4], hex[5])?;
    let f2 = from_hex_pair(hex[6], hex[7])?;
    let f3 = from_hex_pair(hex[8], hex[9])?;

    // Decode CRC
    let crc_hi = from_hex_pair(hex[10], hex[11])?;
    let crc_lo = from_hex_pair(hex[12], hex[13])?;
    let received_crc = ((crc_hi as u16) << 8) | (crc_lo as u16);

    // Verify CRC
    let payload: [u8; 5] = [frame_type_val, f0, f1, f2, f3];
    let computed_crc = crc16(&payload);
    if received_crc != computed_crc {
        return None;
    }

    Some(ZmodemFrame {
        frame_type,
        flags: [f0, f1, f2, f3],
    })
}

// ---------------------------------------------------------------------------
// Cancel sequence
// ---------------------------------------------------------------------------

/// Generate the ZMODEM cancel sequence: 8 CAN bytes (0x18) followed by 8 BS bytes (0x08).
pub fn zcancel() -> Vec<u8> {
    let mut out = Vec::with_capacity(16);
    for _ in 0..8 {
        out.push(0x18); // CAN
    }
    for _ in 0..8 {
        out.push(0x08); // BS
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hex_byte_roundtrip() {
        for b in 0..=255u8 {
            let [h, l] = hex_byte(b);
            assert_eq!(from_hex_pair(h, l), Some(b));
        }
    }

    #[test]
    fn test_zcancel_length() {
        let cancel = zcancel();
        assert_eq!(cancel.len(), 16);
        assert!(cancel[..8].iter().all(|&b| b == 0x18));
        assert!(cancel[8..].iter().all(|&b| b == 0x08));
    }

    #[test]
    fn test_crc16_known_value() {
        // CRC-16/XMODEM of empty input is 0
        assert_eq!(crc16(b""), 0);
    }
}
