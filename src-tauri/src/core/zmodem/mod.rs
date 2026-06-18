// src-tauri/src/core/zmodem/mod.rs

pub mod encode;
pub mod decode;
pub mod sender;

// ZMODEM 协议常量

pub const ZPAD: u8 = 0x2a;  // '*'
pub const ZDLE: u8 = 0x18;  // CAN
pub const ZHEX: u8 = 0x42;  // 'B'
pub const ZBIN: u8 = 0x41;  // 'A'（二进制 CRC-16 帧）
pub const ZBIN32: u8 = 0x43; // 'C'
pub const XON: u8 = 0x11;
pub const XOFF: u8 = 0x13;

/// 帧类型
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

/// 子包结束标记
pub const ZCRCE: u8 = 0x68; // 校验下一字节，帧结束，后跟帧头
pub const ZCRCG: u8 = 0x69; // 校验下一字节，帧持续不间断
pub const ZCRCQ: u8 = 0x6a; // 校验下一字节，帧持续，期待 ZACK
pub const ZCRCW: u8 = 0x6b; // 校验下一字节，帧结束，期待 ZACK

/// ZRINIT 能力标志位（ZF0 字节）
pub const CANFDX: u8 = 0x01;  // 全双工
pub const CANOVIO: u8 = 0x02; // 支持 I/O 重叠
pub const CANBRK: u8 = 0x04;  // 可发送 break 信号
pub const CANCRY: u8 = 0x08;  // 可解密
pub const CANLZW: u8 = 0x10;  // 可解压
pub const CANFC32: u8 = 0x20; // 帧中可使用 CRC-32
pub const ESCCTL: u8 = 0x40;  // 接收方要求转义控制字符
pub const ESC8: u8 = 0x80;    // 接收方要求转义第 8 位字符

/// 从线路解码出的帧
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodedFrame {
    pub frame_type: FrameType,
    pub flags: [u8; 4],
}

impl DecodedFrame {
    /// 将 flags 读取为小端序 u32 偏移量（用于 ZRPOS、ZEOF、ZACK、ZDATA）
    pub fn offset(&self) -> u32 {
        u32::from_le_bytes(self.flags)
    }
}
