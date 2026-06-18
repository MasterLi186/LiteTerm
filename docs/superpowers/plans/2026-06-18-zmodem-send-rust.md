# ZMODEM Send Rust Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Implement ZMODEM Send protocol in Rust, replacing zmodem.js for file uploads. Files dragged onto the terminal upload to the remote shell's current directory via `rz`.

**Architecture:** A Rust state machine (`ZmodemSender`) handles the ZMODEM Send protocol on the SSH reader thread. When upload is requested, the reader thread redirects terminal output to the state machine instead of the frontend. The state machine parses rz's responses, sends file data, and handles ZRPOS retransmission. Progress events are emitted to the frontend.

**Tech Stack:** Rust (no new crates — CRC-32 uses `crc32fast` already in deps), Tauri 2 commands, TypeScript/React frontend.

---

### Task 1: Protocol constants and module skeleton

**Files:**
- Delete: `src-tauri/src/core/zmodem/frame.rs`
- Delete: `src-tauri/src/core/zmodem/detect.rs`
- Delete: `src-tauri/src/core/zmodem/send.rs`
- Delete: `src-tauri/src/core/zmodem/receive.rs`
- Rewrite: `src-tauri/src/core/zmodem/mod.rs`

- [ ] **Step 1: Delete old ZMODEM files**

```bash
rm src-tauri/src/core/zmodem/frame.rs \
   src-tauri/src/core/zmodem/detect.rs \
   src-tauri/src/core/zmodem/send.rs \
   src-tauri/src/core/zmodem/receive.rs
```

- [ ] **Step 2: Write the new mod.rs with constants**

```rust
// src-tauri/src/core/zmodem/mod.rs

pub mod encode;
pub mod decode;
pub mod sender;

// ZMODEM protocol constants

pub const ZPAD: u8 = 0x2a;  // '*'
pub const ZDLE: u8 = 0x18;  // CAN
pub const ZHEX: u8 = 0x42;  // 'B'
pub const ZBIN32: u8 = 0x43; // 'C'
pub const XON: u8 = 0x11;
pub const XOFF: u8 = 0x13;

/// Frame types
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
    ZFREECNT = 17,
}

impl FrameType {
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
            17 => Some(Self::ZFREECNT),
            _ => None,
        }
    }
}

/// Subpacket end markers
pub const ZCRCE: u8 = 0x68; // CRC next, frame ends, header follows
pub const ZCRCG: u8 = 0x69; // CRC next, frame continues nonstop
pub const ZCRCQ: u8 = 0x6a; // CRC next, frame continues, ZACK expected
pub const ZCRCW: u8 = 0x6b; // CRC next, frame ends, ZACK expected

/// ZRINIT capability flags (ZF0 byte)
pub const CANFDX: u8 = 0x01;  // full duplex
pub const CANOVIO: u8 = 0x02; // can overlap I/O
pub const CANBRK: u8 = 0x04;  // can send break
pub const CANCRY: u8 = 0x08;  // can decrypt
pub const CANLZW: u8 = 0x10;  // can uncompress
pub const CANFC32: u8 = 0x20; // can use CRC-32 in frames
pub const ESCCTL: u8 = 0x40;  // receiver wants control chars escaped
pub const ESC8: u8 = 0x80;    // receiver wants 8th-bit chars escaped

/// Decoded frame from the wire
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodedFrame {
    pub frame_type: FrameType,
    pub flags: [u8; 4],
}

impl DecodedFrame {
    /// Read flags as a little-endian u32 offset (used by ZRPOS, ZEOF, ZACK, ZDATA)
    pub fn offset(&self) -> u32 {
        u32::from_le_bytes(self.flags)
    }
}
```

- [ ] **Step 3: Create empty encode.rs and decode.rs and sender.rs stubs**

```rust
// src-tauri/src/core/zmodem/encode.rs
// Encoding functions — implemented in Task 2

// src-tauri/src/core/zmodem/decode.rs
// Decoding functions — implemented in Task 3

// src-tauri/src/core/zmodem/sender.rs
// Send state machine — implemented in Task 4
```

- [ ] **Step 4: Verify build**

```bash
cd src-tauri && cargo build 2>&1 | tail -5
```

Expected: Build succeeds (warnings about unused imports are OK).

- [ ] **Step 5: Commit**

```bash
git add -A src-tauri/src/core/zmodem/
git commit -m "refactor: 重写 ZMODEM 模块骨架，定义协议常量和类型"
```

---

### Task 2: Encoding layer (encode.rs)

**Files:**
- Create: `src-tauri/src/core/zmodem/encode.rs`
- Test: `src-tauri/src/core/zmodem/encode.rs` (inline #[cfg(test)])

- [ ] **Step 1: Write CRC functions and tests**

```rust
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

/// Update a running CRC-32 with additional data
pub fn crc32_update(crc: u32, data: &[u8]) -> u32 {
    let mut hasher = crc32fast::Hasher::new_with_initial(crc);
    hasher.update(data);
    hasher.finalize()
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
}
```

- [ ] **Step 2: Run tests**

```bash
cd src-tauri && cargo test --lib core::zmodem::encode::tests -v
```

Expected: 3 tests pass.

- [ ] **Step 3: Add ZDLE encoding and hex helpers**

Append to `encode.rs`:

```rust
use super::{ZPAD, ZDLE, ZHEX, ZBIN32, ZCRCE, ZCRCG, ZCRCQ, ZCRCW, XON};

fn hex_byte(b: u8) -> [u8; 2] {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    [HEX[(b >> 4) as usize], HEX[(b & 0x0f) as usize]]
}

/// ZDLE-encode a single byte. Returns 1 byte (no escape) or 2 bytes (ZDLE + escaped).
/// Uses ESCALL mode: escapes all control chars, DEL, high-bit control chars, and ZDLE itself.
fn zdle_encode_byte(b: u8) -> (u8, Option<u8>) {
    match b {
        ZDLE => (ZDLE, Some(0x58)),                           // ZDLE itself
        0x10 | 0x90 => (ZDLE, Some(b ^ 0x40)),               // DLE
        0x11 | 0x91 => (ZDLE, Some(b ^ 0x40)),               // XON
        0x13 | 0x93 => (ZDLE, Some(b ^ 0x40)),               // XOFF
        0x00..=0x1f => (ZDLE, Some(b | 0x40)),                // control chars
        0x7f => (ZDLE, Some(0x6f)),                            // DEL
        0x80..=0x9f => (ZDLE, Some((b & 0x7f) | 0x40)),       // high-bit control
        0xff => (ZDLE, Some(0x6f)),                            // 0xff
        _ => (b, None),                                        // pass through
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

    // CRC-32 covers data + end_type
    let mut crc = crc32(data);
    crc = crc32_update(crc, &[end_type]);
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
```

- [ ] **Step 4: Add encoding tests**

Append to the `#[cfg(test)] mod tests` block:

```rust
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
```

- [ ] **Step 5: Run all encode tests**

```bash
cd src-tauri && cargo test --lib core::zmodem::encode -v
```

Expected: All tests pass.

- [ ] **Step 6: Commit**

```bash
git add src-tauri/src/core/zmodem/encode.rs
git commit -m "feat(zmodem): 编码层 — CRC、ZDLE 转义、ZHEX/ZBIN32 头、数据子包"
```

---

### Task 3: Decoding layer (decode.rs)

**Files:**
- Create: `src-tauri/src/core/zmodem/decode.rs`
- Test: inline #[cfg(test)]

- [ ] **Step 1: Write the incremental decoder**

```rust
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

/// ZDLE-decode: reverse the escaping applied by the sender
fn zdle_decode_byte(escaped: u8) -> u8 {
    if escaped == 0x58 {
        ZDLE // ZDLE escaped
    } else if escaped == 0x6f {
        0x7f // DEL or 0xff
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

            // Not enough data yet — wait for more
            break;
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

        let hex = &self.buf[4..18];
        let frame_type_val = hex_pair(hex[0], hex[1])?;
        let f0 = hex_pair(hex[2], hex[3])?;
        let f1 = hex_pair(hex[4], hex[5])?;
        let f2 = hex_pair(hex[6], hex[7])?;
        let f3 = hex_pair(hex[8], hex[9])?;
        let crc_hi = hex_pair(hex[10], hex[11])?;
        let crc_lo = hex_pair(hex[12], hex[13])?;

        let received_crc = ((crc_hi as u16) << 8) | crc_lo as u16;
        let payload = [frame_type_val, f0, f1, f2, f3];
        let computed_crc = crc16(&payload);

        if received_crc != computed_crc {
            // CRC mismatch — skip this ZPAD and try next
            self.buf.drain(..1);
            return None;
        }

        let frame_type = FrameType::from_u8(frame_type_val)?;

        // Consume the header + trailing CR LF (and optional XON)
        let mut consumed = 18;
        while consumed < self.buf.len() && (self.buf[consumed] == b'\r' || self.buf[consumed] == b'\n' || self.buf[consumed] == 0x11) {
            consumed += 1;
        }
        self.buf.drain(..consumed);

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

        let frame_type = FrameType::from_u8(frame_type_val)?;
        self.buf.drain(..i);

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
```

- [ ] **Step 2: Run decode tests**

```bash
cd src-tauri && cargo test --lib core::zmodem::decode -v
```

Expected: All 5 tests pass.

- [ ] **Step 3: Commit**

```bash
git add src-tauri/src/core/zmodem/decode.rs
git commit -m "feat(zmodem): 解码层 — ZHEX/ZBIN32 帧解析、ZDLE 反转义、增量解析器"
```

---

### Task 4: Send state machine (sender.rs)

**Files:**
- Create: `src-tauri/src/core/zmodem/sender.rs`
- Test: inline #[cfg(test)]

- [ ] **Step 1: Write the state machine**

```rust
// src-tauri/src/core/zmodem/sender.rs

use std::io::{Read, Seek, SeekFrom};
use std::path::PathBuf;

use super::{DecodedFrame, FrameType, CANFC32, ESCCTL};
use super::{ZCRCE, ZCRCG, ZCRCQ};
use super::encode::*;

const SUBPACKET_SIZE: usize = 8192;
const WINDOW_SUBPACKETS: usize = 32; // send ZCRCQ every 32 subpackets for flow control

pub struct FileInfo {
    pub path: PathBuf,
    pub name: String,
    pub size: u64,
    pub mtime: u64,
}

#[derive(Debug)]
pub enum SenderAction {
    Send(Vec<u8>),
    Progress { bytes_sent: u64, total: u64, filename: String },
    FileComplete(String),
    AllComplete,
    Error(String),
    None,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum State {
    Init,
    WaitZrinit,
    SendZfile,
    WaitFileAccept,
    SendData,
    SentEof,
    SendZfin,
    WaitZfinReply,
    Done,
}

pub struct ZmodemSender {
    state: State,
    files: Vec<FileInfo>,
    current_idx: usize,
    file: Option<std::fs::File>,
    file_offset: u64,
    file_size: u64,
    subpacket_count: usize,
    escape_all: bool,
}

impl ZmodemSender {
    pub fn new(files: Vec<FileInfo>) -> Self {
        Self {
            state: State::Init,
            files,
            current_idx: 0,
            file: None,
            file_offset: 0,
            file_size: 0,
            subpacket_count: 0,
            escape_all: false,
        }
    }

    pub fn is_done(&self) -> bool {
        self.state == State::Done
    }

    /// Generate the initial ZRQINIT to kick off the session
    pub fn start(&mut self) -> SenderAction {
        self.state = State::WaitZrinit;
        SenderAction::Send(encode_zhex_header(FrameType::ZRQINIT as u8, [0; 4]))
    }

    /// Process an incoming frame from rz
    pub fn handle_frame(&mut self, frame: &DecodedFrame) -> SenderAction {
        match self.state {
            State::WaitZrinit => {
                if frame.frame_type == FrameType::ZRINIT {
                    self.escape_all = (frame.flags[3] & ESCCTL) != 0;
                    self.state = State::SendZfile;
                    return self.send_zfile();
                }
            }
            State::WaitFileAccept => {
                match frame.frame_type {
                    FrameType::ZRPOS => {
                        let offset = frame.offset() as u64;
                        return self.seek_and_send_data(offset);
                    }
                    FrameType::ZSKIP => {
                        return self.advance_to_next_file();
                    }
                    FrameType::ZNAK => {
                        // Resend ZFILE
                        return self.send_zfile();
                    }
                    _ => {}
                }
            }
            State::SendData => {
                match frame.frame_type {
                    FrameType::ZRPOS => {
                        let offset = frame.offset() as u64;
                        return self.seek_and_send_data(offset);
                    }
                    FrameType::ZACK => {
                        // Flow control ACK — continue sending
                        return SenderAction::None;
                    }
                    FrameType::ZSKIP => {
                        return self.advance_to_next_file();
                    }
                    FrameType::ZABORT | FrameType::ZCAN => {
                        self.state = State::Done;
                        return SenderAction::Error("远端取消传输".into());
                    }
                    _ => {}
                }
            }
            State::SentEof => {
                match frame.frame_type {
                    FrameType::ZRINIT => {
                        let name = self.current_file_name();
                        let action = SenderAction::FileComplete(name);
                        // Advance to next file or finish
                        let next = self.advance_to_next_file();
                        // Return file complete first; caller should also process next
                        return match next {
                            SenderAction::AllComplete => SenderAction::FileComplete(self.files[self.current_idx.saturating_sub(1)].name.clone()),
                            _ => action,
                        };
                    }
                    FrameType::ZRPOS => {
                        let offset = frame.offset() as u64;
                        return self.seek_and_send_data(offset);
                    }
                    _ => {}
                }
            }
            State::WaitZfinReply => {
                if frame.frame_type == FrameType::ZFIN {
                    self.state = State::Done;
                    return SenderAction::Send(over_and_out());
                }
            }
            _ => {}
        }
        SenderAction::None
    }

    /// Generate the next chunk of data to send. Call repeatedly while in SendData state.
    /// Returns None when EOF reached (and sends ZEOF).
    pub fn next_data_chunk(&mut self) -> Option<SenderAction> {
        if self.state != State::SendData {
            return None;
        }

        let file = self.file.as_mut()?;
        let mut buf = vec![0u8; SUBPACKET_SIZE];
        let n = match file.read(&mut buf) {
            Ok(n) => n,
            Err(e) => return Some(SenderAction::Error(format!("读取文件失败: {}", e))),
        };

        if n == 0 {
            // EOF — send ZEOF
            self.state = State::SentEof;
            let offset = self.file_offset as u32;
            let mut out = encode_data_subpacket(&[], ZCRCE);
            out.extend(encode_zbin32_header(FrameType::ZEOF as u8, offset.to_le_bytes()));
            return Some(SenderAction::Send(out));
        }

        self.file_offset += n as u64;
        self.subpacket_count += 1;

        // Choose subpacket end type
        let end_type = if self.subpacket_count % WINDOW_SUBPACKETS == 0 {
            ZCRCQ // request ZACK for flow control
        } else {
            ZCRCG // continue nonstop
        };

        let encoded = encode_data_subpacket(&buf[..n], end_type);

        Some(SenderAction::Send(encoded))
    }

    /// Get current progress
    pub fn progress(&self) -> Option<SenderAction> {
        if self.current_idx < self.files.len() {
            Some(SenderAction::Progress {
                bytes_sent: self.file_offset,
                total: self.file_size,
                filename: self.files[self.current_idx].name.clone(),
            })
        } else {
            None
        }
    }

    // --- internal ---

    fn send_zfile(&mut self) -> SenderAction {
        if self.current_idx >= self.files.len() {
            return self.send_zfin();
        }

        let info = &self.files[self.current_idx];
        match std::fs::File::open(&info.path) {
            Ok(f) => {
                self.file_size = info.size;
                self.file = Some(f);
                self.file_offset = 0;
                self.subpacket_count = 0;

                // ZFILE header (ZBIN32) + file info subpacket
                let zfile_flags: [u8; 4] = [0, 0, 0, 0]; // no special conversion
                let mut out = encode_zbin32_header(FrameType::ZFILE as u8, zfile_flags);
                let subpkt = encode_zfile_subpacket(&info.name, info.size, info.mtime);
                out.extend(encode_data_subpacket(&subpkt, ZCRCW));

                self.state = State::WaitFileAccept;
                SenderAction::Send(out)
            }
            Err(e) => SenderAction::Error(format!("无法打开文件 {}: {}", info.path.display(), e)),
        }
    }

    fn seek_and_send_data(&mut self, offset: u64) -> SenderAction {
        if let Some(ref mut f) = self.file {
            if let Err(e) = f.seek(SeekFrom::Start(offset)) {
                return SenderAction::Error(format!("Seek 失败: {}", e));
            }
            self.file_offset = offset;
            self.subpacket_count = 0;
            self.state = State::SendData;

            // Send ZDATA header with the offset
            let offset_bytes = (offset as u32).to_le_bytes();
            SenderAction::Send(encode_zbin32_header(FrameType::ZDATA as u8, offset_bytes))
        } else {
            SenderAction::Error("文件未打开".into())
        }
    }

    fn advance_to_next_file(&mut self) -> SenderAction {
        self.file = None;
        self.current_idx += 1;
        if self.current_idx >= self.files.len() {
            return self.send_zfin();
        }
        self.state = State::SendZfile;
        self.send_zfile()
    }

    fn send_zfin(&mut self) -> SenderAction {
        self.state = State::WaitZfinReply;
        SenderAction::Send(encode_zhex_header(FrameType::ZFIN as u8, [0; 4]))
    }

    fn current_file_name(&self) -> String {
        if self.current_idx < self.files.len() {
            self.files[self.current_idx].name.clone()
        } else {
            String::new()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::FrameType;

    #[test]
    fn test_sender_start_sends_zrqinit() {
        let mut sender = ZmodemSender::new(vec![]);
        match sender.start() {
            SenderAction::Send(data) => {
                // Should be a ZHEX ZRQINIT header
                assert!(data.contains(&0x2a)); // ZPAD
                assert!(!sender.is_done());
            }
            _ => panic!("Expected Send action"),
        }
    }

    #[test]
    fn test_sender_empty_files_sends_zfin() {
        let mut sender = ZmodemSender::new(vec![]);
        sender.start();
        let zrinit = DecodedFrame { frame_type: FrameType::ZRINIT, flags: [0, 0, 0, 0x23] };
        match sender.handle_frame(&zrinit) {
            SenderAction::Send(data) => {
                // With no files, should send ZFIN
                let s = String::from_utf8_lossy(&data);
                // ZHEX ZFIN header contains "08" (ZFIN = 8) in hex
                assert!(s.contains("08") || data.contains(&(FrameType::ZFIN as u8)));
            }
            _ => panic!("Expected Send action for ZFIN"),
        }
    }
}
```

- [ ] **Step 2: Run sender tests**

```bash
cd src-tauri && cargo test --lib core::zmodem::sender -v
```

Expected: 2 tests pass.

- [ ] **Step 3: Commit**

```bash
git add src-tauri/src/core/zmodem/sender.rs
git commit -m "feat(zmodem): Send 状态机 — ZFILE/ZDATA/ZEOF/ZFIN/ZRPOS 完整协议"
```

---

### Task 5: Tauri command and SSH reader thread integration

**Files:**
- Create: `src-tauri/src/commands/zmodem.rs`
- Modify: `src-tauri/src/commands/mod.rs`
- Modify: `src-tauri/src/state.rs`
- Modify: `src-tauri/src/commands/ssh.rs`
- Modify: `src-tauri/src/lib.rs`

- [ ] **Step 1: Add zmodem fields to ManagedSession**

In `src-tauri/src/state.rs`, add to `ManagedSession`:

```rust
pub struct ManagedSession {
    pub id: String,
    pub label: String,
    pub input_tx: std::sync::mpsc::Sender<Vec<u8>>,
    pub resize_tx: std::sync::mpsc::Sender<(u32, u32)>,
    pub monitor_stop: Arc<AtomicBool>,
    pub sftp_request_tx: std::sync::mpsc::Sender<SftpRequest>,
    pub zmodem_active: Arc<AtomicBool>,
    pub zmodem_tx: Arc<Mutex<Option<std::sync::mpsc::Sender<Vec<u8>>>>>,
}
```

- [ ] **Step 2: Update ssh.rs session creation to include zmodem fields**

In `src-tauri/src/commands/ssh.rs`, where `ManagedSession` is created (around line 325-335), add the new fields:

```rust
let zmodem_active = Arc::new(AtomicBool::new(false));
let zmodem_tx: Arc<Mutex<Option<std::sync::mpsc::Sender<Vec<u8>>>>> = Arc::new(Mutex::new(None));
```

Pass clones into the reader thread and add to `ManagedSession`:

```rust
pub zmodem_active: zmodem_active.clone(),
pub zmodem_tx: zmodem_tx.clone(),
```

- [ ] **Step 3: Modify SSH reader thread to check zmodem_active**

In the reader thread loop (around line 277-304 in ssh.rs), change the `Ok(n)` branch:

```rust
Ok(n) => {
    if zmodem_active_clone.load(std::sync::atomic::Ordering::Relaxed) {
        if let Some(ref tx) = *zmodem_tx_clone.lock().unwrap() {
            let _ = tx.send(buf[..n].to_vec());
        }
    } else {
        let _ = app_clone.emit(
            "terminal-output",
            serde_json::json!({
                "id": id_for_read,
                "data": &buf[..n],
            }),
        );
    }
}
```

Where `zmodem_active_clone` and `zmodem_tx_clone` are clones passed into the thread.

- [ ] **Step 4: Create the zmodem command**

```rust
// src-tauri/src/commands/zmodem.rs

use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use tauri::{AppHandle, Emitter, State};

use crate::app_log;
use crate::core::zmodem::decode::ZmodemDecoder;
use crate::core::zmodem::sender::{FileInfo, SenderAction, ZmodemSender};
use crate::state::AppState;

#[tauri::command]
pub async fn zmodem_send(
    state: State<'_, AppState>,
    app: AppHandle,
    session_id: String,
    files: Vec<String>,
) -> Result<(), String> {
    app_log!("ZMODEM", "SEND START: session={}, files={}", session_id, files.len());

    // Collect file info
    let mut file_infos = Vec::new();
    for path_str in &files {
        let expanded = shellexpand::tilde(path_str).to_string();
        let path = PathBuf::from(&expanded);
        let meta = std::fs::metadata(&path)
            .map_err(|e| format!("无法读取文件: {} - {}", path_str, e))?;
        let name = path.file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| path_str.clone());
        let mtime = meta.modified().ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_secs())
            .unwrap_or(0);
        file_infos.push(FileInfo {
            path,
            name,
            size: meta.len(),
            mtime,
        });
    }

    // Get session resources
    let (input_tx, zmodem_active, zmodem_tx_holder) = {
        let sessions = state.sessions.lock().unwrap();
        let session = sessions.get(&session_id)
            .ok_or_else(|| "会话未找到".to_string())?;
        (
            session.input_tx.clone(),
            session.zmodem_active.clone(),
            session.zmodem_tx.clone(),
        )
    };

    // Create channel for receiving terminal output from the reader thread
    let (tx, rx) = std::sync::mpsc::channel::<Vec<u8>>();

    // Activate ZMODEM mode
    *zmodem_tx_holder.lock().unwrap() = Some(tx);
    zmodem_active.store(true, Ordering::Relaxed);

    // Send "rz\r" to start rz on the remote
    let _ = input_tx.send(b"rz\r".to_vec());

    // Run the protocol on a blocking thread
    let app_clone = app.clone();
    let session_id_clone = session_id.clone();
    let result = tokio::task::spawn_blocking(move || {
        run_zmodem_protocol(file_infos, input_tx, rx, app_clone, &session_id_clone)
    }).await.map_err(|e| format!("ZMODEM 线程异常: {}", e))?;

    // Deactivate ZMODEM mode
    zmodem_active.store(false, Ordering::Relaxed);
    *zmodem_tx_holder.lock().unwrap() = None;

    app_log!("ZMODEM", "SEND END: session={}, result={:?}", session_id, result.is_ok());

    // Send a newline to refresh the prompt
    {
        let sessions = state.sessions.lock().unwrap();
        if let Some(session) = sessions.get(&session_id) {
            let _ = session.input_tx.send(b"\r".to_vec());
        }
    }

    result
}

fn run_zmodem_protocol(
    files: Vec<FileInfo>,
    input_tx: std::sync::mpsc::Sender<Vec<u8>>,
    rx: std::sync::mpsc::Receiver<Vec<u8>>,
    app: AppHandle,
    session_id: &str,
) -> Result<(), String> {
    let mut sender = ZmodemSender::new(files);
    let mut decoder = ZmodemDecoder::new();

    // Send ZRQINIT to initiate
    match sender.start() {
        SenderAction::Send(data) => { let _ = input_tx.send(data); }
        _ => {}
    }

    let mut last_progress = Instant::now();
    let timeout = Duration::from_secs(30);
    let mut last_activity = Instant::now();

    loop {
        if sender.is_done() {
            break;
        }

        // Check for timeout
        if last_activity.elapsed() > timeout {
            app_log!("ZMODEM", "TIMEOUT: 30 秒无响应");
            let _ = input_tx.send(crate::core::zmodem::encode::zcancel());
            return Err("ZMODEM 超时".into());
        }

        // Try to receive data from the reader thread (non-blocking with short timeout)
        match rx.recv_timeout(Duration::from_millis(10)) {
            Ok(data) => {
                last_activity = Instant::now();

                // Check for cancel (5+ CAN bytes)
                if ZmodemDecoder::detect_cancel(&data) {
                    app_log!("ZMODEM", "远端取消");
                    return Err("远端取消传输".into());
                }

                // Parse frames
                let frames = decoder.feed(&data);
                for frame in frames {
                    app_log!("ZMODEM", "收到帧: {:?} offset={}", frame.frame_type, frame.offset());
                    match sender.handle_frame(&frame) {
                        SenderAction::Send(out) => { let _ = input_tx.send(out); }
                        SenderAction::Error(e) => {
                            app_log!("ZMODEM", "ERROR: {}", e);
                            let _ = input_tx.send(crate::core::zmodem::encode::zcancel());
                            return Err(e);
                        }
                        SenderAction::FileComplete(name) => {
                            app_log!("ZMODEM", "文件完成: {}", name);
                        }
                        SenderAction::AllComplete => {
                            app_log!("ZMODEM", "所有文件传输完成");
                        }
                        SenderAction::Progress { .. } | SenderAction::None => {}
                    }
                }
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                return Err("终端连接断开".into());
            }
        }

        // Pump data while in SendData state
        while let Some(action) = sender.next_data_chunk() {
            match action {
                SenderAction::Send(data) => {
                    let _ = input_tx.send(data);
                    last_activity = Instant::now();
                }
                SenderAction::Error(e) => {
                    let _ = input_tx.send(crate::core::zmodem::encode::zcancel());
                    return Err(e);
                }
                _ => break,
            }

            // Emit progress every 200ms
            if last_progress.elapsed() > Duration::from_millis(200) {
                if let Some(SenderAction::Progress { bytes_sent, total, filename }) = sender.progress() {
                    let _ = app.emit("transfer-progress", serde_json::json!({
                        "filename": filename,
                        "bytes_transferred": bytes_sent,
                        "total_bytes": total,
                        "direction": "zmodem-upload"
                    }));
                }
                last_progress = Instant::now();
            }

            // Check for incoming frames during data sending (non-blocking)
            if let Ok(data) = rx.try_recv() {
                last_activity = Instant::now();
                if ZmodemDecoder::detect_cancel(&data) {
                    return Err("远端取消传输".into());
                }
                let frames = decoder.feed(&data);
                for frame in frames {
                    app_log!("ZMODEM", "数据发送中收到帧: {:?}", frame.frame_type);
                    match sender.handle_frame(&frame) {
                        SenderAction::Send(out) => { let _ = input_tx.send(out); }
                        SenderAction::Error(e) => {
                            let _ = input_tx.send(crate::core::zmodem::encode::zcancel());
                            return Err(e);
                        }
                        _ => {}
                    }
                }
            }
        }
    }

    // Final progress
    if let Some(SenderAction::Progress { bytes_sent, total, filename }) = sender.progress() {
        let _ = app.emit("transfer-progress", serde_json::json!({
            "filename": filename,
            "bytes_transferred": bytes_sent,
            "total_bytes": total,
            "direction": "zmodem-upload"
        }));
    }

    Ok(())
}
```

- [ ] **Step 5: Register the command**

In `src-tauri/src/commands/mod.rs`, add:

```rust
pub mod zmodem;
```

In `src-tauri/src/lib.rs`, add to `invoke_handler`:

```rust
commands::zmodem::zmodem_send,
```

- [ ] **Step 6: Build and verify**

```bash
cd src-tauri && cargo build 2>&1 | tail -10
```

Expected: Build succeeds.

- [ ] **Step 7: Commit**

```bash
git add src-tauri/src/commands/zmodem.rs src-tauri/src/commands/mod.rs \
        src-tauri/src/state.rs src-tauri/src/commands/ssh.rs src-tauri/src/lib.rs
git commit -m "feat(zmodem): Tauri 命令 zmodem_send + SSH reader 线程集成"
```

---

### Task 6: Frontend integration

**Files:**
- Modify: `src/App.tsx`
- Modify: `src/components/Terminal/TerminalPane.tsx`
- Modify: `src/components/Terminal/SplitContainer.tsx`

- [ ] **Step 1: Simplify drag-drop handler in App.tsx**

Replace the entire drag-drop upload handler (the `useEffect` with `onDragDropEvent`) to use `zmodem_send`:

```typescript
// Drag-drop upload via ZMODEM
useEffect(() => {
    const webview = getCurrentWebview();
    const unlisten = webview.onDragDropEvent((event) => {
      if (!activeSshSessionId) return;
      if (event.payload.type === 'enter' || event.payload.type === 'over') {
        setDragOverTerminal(true);
      } else if (event.payload.type === 'leave') {
        setDragOverTerminal(false);
      } else if (event.payload.type === 'drop') {
        setDragOverTerminal(false);
        const paths = event.payload.paths;
        if (!paths.length) return;
        log('拖拽上传', `${paths.length} 个文件通过 ZMODEM 上传`, paths);
        invoke('zmodem_send', { sessionId: activeSshSessionId, files: paths })
          .then(() => log('拖拽上传', '完成'))
          .catch((e) => {
            log('拖拽上传', `失败: ${e}`);
            setError(`上传失败: ${e}`);
          });
      }
    });
    return () => { unlisten.then(fn => fn()); };
}, [activeSshSessionId]);
```

Remove `terminalPwdQueryFns` ref and all pwd-query related code.

- [ ] **Step 2: Remove ZMODEM Send code from TerminalPane.tsx**

In the `handleZmodemDetection` function, simplify the `else` branch (Send session) to just close:

```typescript
} else {
    // Send session — handled by Rust backend (zmodem_send command)
    session.close();
}
```

Remove `onRegisterPwdQuery` prop from TerminalPane `Props` interface and component signature.

- [ ] **Step 3: Remove onRegisterPwdQuery from SplitContainer.tsx**

Remove `onRegisterPwdQuery` from `SplitContainerProps` interface, all function signatures, and all prop passes.

- [ ] **Step 4: Build frontend**

```bash
npm run build 2>&1 | tail -5
```

Expected: Build succeeds.

- [ ] **Step 5: Full build**

```bash
./build.sh 2>&1 | tail -5
```

Expected: Build succeeds.

- [ ] **Step 6: Commit**

```bash
git add src/App.tsx src/components/Terminal/TerminalPane.tsx src/components/Terminal/SplitContainer.tsx
git commit -m "feat(zmodem): 前端集成 — 拖拽上传走 Rust ZMODEM，移除 JS Send session"
```

---

### Task 7: Integration test with local rz

**Files:**
- Test: manual

- [ ] **Step 1: Test with a small file**

```bash
./run.sh
# In the app:
# 1. Connect to an SSH server
# 2. Drag a small text file (< 1KB) onto the terminal
# 3. Verify:
#    - "rz" appears in terminal briefly
#    - File appears in remote cwd
#    - Progress bar shows in top-right
#    - Terminal is responsive after transfer
```

- [ ] **Step 2: Test with a large binary file**

```bash
# Drag a 100MB+ binary file onto the terminal
# Verify:
#    - Transfer completes without timeout
#    - md5sum matches on both sides
#    - Progress updates smoothly
```

- [ ] **Step 3: Test cancel**

```bash
# Drag a large file, then click cancel in the progress panel
# Verify:
#    - Transfer stops
#    - Terminal becomes responsive
```

- [ ] **Step 4: Check logs**

```bash
tail -50 ~/guishell.log | grep ZMODEM
# Verify log entries for SEND START, frame exchanges, PROGRESS, SEND END
```

- [ ] **Step 5: Final commit**

```bash
git add -A
git commit -m "feat(zmodem): ZMODEM Send Rust 实现完成 — 对标 lrzsz 兼容"
```
