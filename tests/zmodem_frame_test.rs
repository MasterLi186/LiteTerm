use guishell::core::zmodem::frame::{FrameType, ZmodemFrame, encode_zhex_header, decode_zhex_header, crc16};

#[test]
fn test_frame_type_values() {
    assert_eq!(FrameType::ZRINIT as u8, 1);
    assert_eq!(FrameType::ZFILE as u8, 4);
    assert_eq!(FrameType::ZDATA as u8, 10);
    assert_eq!(FrameType::ZEOF as u8, 11);
    assert_eq!(FrameType::ZFIN as u8, 8);
}

#[test]
fn test_encode_zhex_header_roundtrip() {
    let frame = ZmodemFrame {
        frame_type: FrameType::ZRINIT,
        flags: [0x00, 0x00, 0x00, 0x23],
    };
    let encoded = encode_zhex_header(&frame);
    let decoded = decode_zhex_header(&encoded).unwrap();
    assert_eq!(decoded.frame_type as u8, FrameType::ZRINIT as u8);
    assert_eq!(decoded.flags, [0x00, 0x00, 0x00, 0x23]);
}

#[test]
fn test_crc16_calculation() {
    let data = b"Hello";
    let crc = crc16(data);
    assert_ne!(crc, 0);
    assert_eq!(crc, crc16(b"Hello"));
}

#[test]
fn test_decode_invalid_hex_returns_none() {
    let result = decode_zhex_header(b"not a valid frame");
    assert!(result.is_none());
}
