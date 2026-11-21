use super::{
    ChecksumMode, FrameType, ZBIN, ZBIN32, ZCRCE, ZCRCG, ZCRCQ, ZCRCW, ZDLE, ZHEX, ZPAD, ZRUB0,
    ZRUB1,
};

pub fn crc16(data: &[u8]) -> u16 {
    let mut crc = 0u16;
    for &byte in data {
        crc ^= u16::from(byte) << 8;
        for _ in 0..8 {
            crc = if crc & 0x8000 != 0 {
                (crc << 1) ^ 0x1021
            } else {
                crc << 1
            };
        }
    }
    crc
}

pub fn crc32(data: &[u8]) -> u32 {
    crc32fast::hash(data)
}

pub fn crc32_multi(slices: &[&[u8]]) -> u32 {
    let mut hasher = crc32fast::Hasher::new();
    for slice in slices {
        hasher.update(slice);
    }
    hasher.finalize()
}

fn hex_byte(byte: u8) -> [u8; 2] {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    [HEX[usize::from(byte >> 4)], HEX[usize::from(byte & 0x0f)]]
}

pub(crate) fn zdle_encode_byte(byte: u8, escape_control: bool, output: &mut Vec<u8>) {
    let escaped = match byte {
        0x7f => Some(ZRUB0),
        0xff => Some(ZRUB1),
        ZDLE => Some(ZDLE ^ 0x40),
        0x10 | 0x11 | 0x13 | 0x90 | 0x91 | 0x93 => Some(byte ^ 0x40),
        0x00..=0x1f | 0x80..=0x9f if escape_control => Some(byte ^ 0x40),
        _ => None,
    };
    if let Some(escaped) = escaped {
        output.extend_from_slice(&[ZDLE, escaped]);
    } else {
        output.push(byte);
    }
}

pub fn zdle_encode(data: &[u8], escape_control: bool) -> Vec<u8> {
    let mut output = Vec::with_capacity(data.len().saturating_mul(2));
    for &byte in data {
        zdle_encode_byte(byte, escape_control, &mut output);
    }
    output
}

pub fn encode_zhex_header(frame_type: FrameType, flags: [u8; 4]) -> Vec<u8> {
    let payload = [frame_type as u8, flags[0], flags[1], flags[2], flags[3]];
    let mut output = Vec::with_capacity(21);
    output.extend_from_slice(&[ZPAD, ZPAD, ZDLE, ZHEX]);
    for byte in payload {
        output.extend_from_slice(&hex_byte(byte));
    }
    let checksum = crc16(&payload).to_be_bytes();
    output.extend_from_slice(&hex_byte(checksum[0]));
    output.extend_from_slice(&hex_byte(checksum[1]));
    output.extend_from_slice(b"\r\n");
    if !matches!(frame_type, FrameType::Zfin | FrameType::Zack) {
        output.push(super::XON);
    }
    output
}

pub fn encode_zbin32_header(frame_type: FrameType, flags: [u8; 4]) -> Vec<u8> {
    let payload = [frame_type as u8, flags[0], flags[1], flags[2], flags[3]];
    let mut output = Vec::with_capacity(24);
    output.extend_from_slice(&[ZPAD, ZDLE, ZBIN32]);
    for byte in payload.into_iter().chain(crc32(&payload).to_le_bytes()) {
        zdle_encode_byte(byte, true, &mut output);
    }
    output
}

pub fn encode_zbin16_header(frame_type: FrameType, flags: [u8; 4]) -> Vec<u8> {
    let payload = [frame_type as u8, flags[0], flags[1], flags[2], flags[3]];
    let mut output = Vec::with_capacity(20);
    output.extend_from_slice(&[ZPAD, ZDLE, ZBIN]);
    for byte in payload.into_iter().chain(crc16(&payload).to_be_bytes()) {
        zdle_encode_byte(byte, true, &mut output);
    }
    output
}

pub fn encode_data_subpacket(
    data: &[u8],
    end_type: u8,
    escape_control: bool,
) -> Result<Vec<u8>, &'static str> {
    encode_data_subpacket_with_checksum(data, end_type, escape_control, ChecksumMode::Crc32)
}

pub fn encode_data_subpacket_with_checksum(
    data: &[u8],
    end_type: u8,
    escape_control: bool,
    checksum_mode: ChecksumMode,
) -> Result<Vec<u8>, &'static str> {
    if !matches!(end_type, ZCRCE | ZCRCG | ZCRCQ | ZCRCW) {
        return Err("invalid ZMODEM data subpacket terminator");
    }
    let mut output = Vec::with_capacity(data.len().saturating_mul(2).saturating_add(10));
    for &byte in data {
        zdle_encode_byte(byte, escape_control, &mut output);
    }
    output.extend_from_slice(&[ZDLE, end_type]);
    let checksum: Vec<u8> = match checksum_mode {
        ChecksumMode::Crc16 => {
            let mut payload = Vec::with_capacity(data.len().saturating_add(1));
            payload.extend_from_slice(data);
            payload.push(end_type);
            crc16(&payload).to_be_bytes().to_vec()
        }
        ChecksumMode::Crc32 => crc32_multi(&[data, &[end_type]]).to_le_bytes().to_vec(),
    };
    for byte in checksum {
        zdle_encode_byte(byte, true, &mut output);
    }
    Ok(output)
}

pub fn encode_zfile_metadata(name: &str, size: u32, mtime: u64, files_remaining: u32) -> Vec<u8> {
    format!("{name}\0{size} {mtime} 0 0 0 {files_remaining} 0\0").into_bytes()
}

pub fn encode_cancel() -> Vec<u8> {
    let mut output = Vec::with_capacity(16);
    output.extend_from_slice(&[ZDLE; 8]);
    output.extend_from_slice(&[0x08; 8]);
    output
}

pub fn encode_over_and_out() -> Vec<u8> {
    b"OO".to_vec()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn standard_crc_vectors_match() {
        assert_eq!(crc16(b"123456789"), 0x31c3);
        assert_eq!(crc32(b"123456789"), 0xcbf4_3926);
    }

    #[test]
    fn zdle_encodes_boundaries_and_rubout_codes() {
        assert_eq!(
            zdle_encode(&[0, ZDLE, 0x7f, 0xff], true),
            vec![ZDLE, 0x40, ZDLE, 0x58, ZDLE, ZRUB0, ZDLE, ZRUB1]
        );
        assert_eq!(zdle_encode(&[0x01], false), vec![0x01]);
    }

    #[test]
    fn rejects_invalid_subpacket_terminator() {
        assert!(encode_data_subpacket(b"x", b'?', false).is_err());
    }

    #[test]
    fn empty_and_boundary_metadata_are_explicit() {
        assert_eq!(
            encode_zfile_metadata("empty", 0, 0, 1),
            b"empty\0\x30 0 0 0 0 1 0\0".to_vec()
        );
        let metadata = encode_zfile_metadata("max", u32::MAX, 1, 2);
        assert!(String::from_utf8(metadata)
            .unwrap()
            .contains("4294967295 1"));
    }
}
