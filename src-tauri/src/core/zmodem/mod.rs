// src-tauri/src/core/zmodem/mod.rs

pub mod encode;
pub mod decode;
pub mod sender;

// ZMODEM protocol constants

pub const ZPAD: u8 = 0x2a;  // '*'
pub const ZDLE: u8 = 0x18;  // CAN
pub const ZHEX: u8 = 0x42;  // 'B'
pub const ZBIN: u8 = 0x41;  // 'A' (binary-16 CRC-16 frame)
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
