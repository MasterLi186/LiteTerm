//! Pure ZMODEM protocol core.
//!
//! This module deliberately has no dependency on `TerminalState`: callers feed
//! raw transport bytes into a session and write the returned protocol bytes at
//! the PTY/SSH raw-reader boundary.

pub mod decode;
pub mod detect;
pub mod encode;
pub mod receiver;
pub mod runtime;
pub mod sender;
pub mod session;
pub mod ui;

pub const ZPAD: u8 = b'*';
pub const ZDLE: u8 = 0x18;
pub const ZBIN: u8 = b'A';
pub const ZHEX: u8 = b'B';
pub const ZBIN32: u8 = b'C';
pub const XON: u8 = 0x11;
pub const XOFF: u8 = 0x13;

pub const ZCRCE: u8 = b'h';
pub const ZCRCG: u8 = b'i';
pub const ZCRCQ: u8 = b'j';
pub const ZCRCW: u8 = b'k';
pub const ZRUB0: u8 = b'l';
pub const ZRUB1: u8 = b'm';

pub const CANFDX: u8 = 0x01;
pub const CANOVIO: u8 = 0x02;
pub const CANBRK: u8 = 0x04;
pub const CANCRY: u8 = 0x08;
pub const CANLZW: u8 = 0x10;
pub const CANFC32: u8 = 0x20;
pub const ESCCTL: u8 = 0x40;
pub const ESC8: u8 = 0x80;

pub const MAX_ZMODEM_FILE_SIZE: u64 = u32::MAX as u64;
pub const DEFAULT_MAX_SUBPACKET_SIZE: usize = 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum FrameType {
    Zrqinit = 0,
    Zrinit = 1,
    Zsinit = 2,
    Zack = 3,
    Zfile = 4,
    Zskip = 5,
    Znak = 6,
    Zabort = 7,
    Zfin = 8,
    Zrpos = 9,
    Zdata = 10,
    Zeof = 11,
    Zferr = 12,
    Zcrc = 13,
    Zchallenge = 14,
    Zcompl = 15,
    Zcan = 16,
    Zfreecnt = 17,
}

impl FrameType {
    pub fn from_u8(value: u8) -> Option<Self> {
        Some(match value {
            0 => Self::Zrqinit,
            1 => Self::Zrinit,
            2 => Self::Zsinit,
            3 => Self::Zack,
            4 => Self::Zfile,
            5 => Self::Zskip,
            6 => Self::Znak,
            7 => Self::Zabort,
            8 => Self::Zfin,
            9 => Self::Zrpos,
            10 => Self::Zdata,
            11 => Self::Zeof,
            12 => Self::Zferr,
            13 => Self::Zcrc,
            14 => Self::Zchallenge,
            15 => Self::Zcompl,
            16 => Self::Zcan,
            17 => Self::Zfreecnt,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HeaderFormat {
    Hex,
    Binary16,
    Binary32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChecksumMode {
    Crc16,
    Crc32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DecodedFrame {
    pub frame_type: FrameType,
    pub flags: [u8; 4],
    pub format: HeaderFormat,
}

impl DecodedFrame {
    pub const fn new(frame_type: FrameType, flags: [u8; 4]) -> Self {
        Self {
            frame_type,
            flags,
            format: HeaderFormat::Hex,
        }
    }

    pub const fn with_format(frame_type: FrameType, flags: [u8; 4], format: HeaderFormat) -> Self {
        Self {
            frame_type,
            flags,
            format,
        }
    }

    pub fn offset(self) -> u32 {
        u32::from_le_bytes(self.flags)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ZmodemError {
    Protocol(String),
    Io(String),
    UnsafeFilename(String),
    FileTooLarge(u64),
    DestinationExists(String),
    Cancelled,
}

impl std::fmt::Display for ZmodemError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Protocol(message) | Self::Io(message) => formatter.write_str(message),
            Self::UnsafeFilename(name) => write!(formatter, "不安全的文件名: {name:?}"),
            Self::FileTooLarge(size) => {
                write!(formatter, "ZMODEM MVP 不支持超过 u32::MAX 的文件: {size}")
            }
            Self::DestinationExists(name) => write!(formatter, "目标文件已存在: {name}"),
            Self::Cancelled => formatter.write_str("传输已取消"),
        }
    }
}

impl std::error::Error for ZmodemError {}

impl From<std::io::Error> for ZmodemError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error.to_string())
    }
}
