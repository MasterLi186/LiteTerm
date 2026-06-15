/// ZMODEM byte stream detector.
///
/// Scans an incoming terminal byte stream for the ZMODEM initiation sequence
/// (`**\x18B` preamble followed by a valid ZHEX header). Normal data is passed
/// through unchanged; when a ZMODEM session start is detected the preceding
/// bytes, decoded frame, and any remaining bytes are returned separately.

use super::frame::{decode_zhex_header, ZmodemFrame};

/// Result of feeding bytes into the detector.
#[derive(Debug)]
pub enum DetectResult {
    /// No ZMODEM activity — the data is plain terminal output.
    Normal(Vec<u8>),
    /// A ZMODEM session initiation was detected.
    ZmodemStart {
        /// Bytes that appeared before the ZMODEM preamble.
        preceding: Vec<u8>,
        /// The decoded ZHEX header frame.
        frame: ZmodemFrame,
        /// Bytes that appeared after the header (not yet consumed).
        remaining: Vec<u8>,
    },
}

/// Scans a byte stream for the ZMODEM `**\x18B` preamble and attempts to
/// decode the following ZHEX header.
pub struct ZmodemDetector {
    buf: Vec<u8>,
}

/// The four-byte preamble that starts every ZHEX header.
const PREAMBLE: [u8; 4] = [b'*', b'*', 0x18, b'B'];

/// Minimum number of bytes after the preamble required for a complete ZHEX
/// header: 14 hex chars (type[2] + flags[8] + crc[4]) + CR LF = 16.
const HEADER_HEX_LEN: usize = 14;

impl ZmodemDetector {
    /// Create a new detector with an empty internal buffer.
    pub fn new() -> Self {
        Self { buf: Vec::new() }
    }

    /// Feed raw bytes from the terminal stream into the detector.
    ///
    /// The detector appends `data` to its internal buffer, then searches for
    /// the ZMODEM preamble. Three outcomes are possible:
    ///
    /// 1. No preamble found and no partial match at the tail — all buffered
    ///    data is returned as `Normal`.
    /// 2. A complete preamble + valid ZHEX header is found — the buffer is
    ///    split into `preceding`, the decoded `frame`, and `remaining`.
    /// 3. The buffer ends with a partial preamble match — the safe prefix is
    ///    returned as `Normal` and the potential match stays buffered.
    pub fn feed(&mut self, data: &[u8]) -> DetectResult {
        self.buf.extend_from_slice(data);

        // Try to find the full preamble.
        if let Some(pos) = find_preamble(&self.buf) {
            let after_preamble = pos + PREAMBLE.len();

            // Do we have enough bytes for the hex header?
            if self.buf.len() >= after_preamble + HEADER_HEX_LEN {
                // Attempt to decode starting from the preamble position.
                if let Some(frame) = decode_zhex_header(&self.buf[pos..]) {
                    let preceding = self.buf[..pos].to_vec();
                    // The full header is preamble(4) + hex(14) + CRLF(2) = 20
                    // but the terminator (CR LF) may or may not be present in
                    // the remaining data — skip past everything decode consumed.
                    let header_end = after_preamble + HEADER_HEX_LEN;
                    // Skip optional CR LF terminator.
                    let mut skip = header_end;
                    if skip < self.buf.len() && self.buf[skip] == b'\r' {
                        skip += 1;
                    }
                    if skip < self.buf.len() && self.buf[skip] == b'\n' {
                        skip += 1;
                    }
                    let remaining = self.buf[skip..].to_vec();
                    self.buf.clear();
                    return DetectResult::ZmodemStart {
                        preceding,
                        frame,
                        remaining,
                    };
                }
                // Preamble found but decode failed — not a real header.
                // Skip past this false match and continue searching.
                // Return everything up to and including the false preamble as
                // normal data, keep the rest buffered.
                let flush_end = pos + 1; // advance past first '*'
                let normal = self.buf[..flush_end].to_vec();
                self.buf = self.buf[flush_end..].to_vec();
                return DetectResult::Normal(normal);
            }
            // We have the preamble but not enough trailing bytes yet.
            // Return everything before the preamble as Normal, keep the rest.
            if pos > 0 {
                let normal = self.buf[..pos].to_vec();
                self.buf = self.buf[pos..].to_vec();
                return DetectResult::Normal(normal);
            }
            // The preamble is at position 0 but incomplete — wait for more data.
            return DetectResult::Normal(Vec::new());
        }

        // No full preamble found. Check if the tail could be the start of one.
        let keep = partial_preamble_tail(&self.buf);
        if keep > 0 {
            let safe = self.buf.len() - keep;
            if safe > 0 {
                let normal = self.buf[..safe].to_vec();
                self.buf = self.buf[safe..].to_vec();
                return DetectResult::Normal(normal);
            }
            // Entire buffer is a partial match — nothing to emit yet.
            return DetectResult::Normal(Vec::new());
        }

        // No match at all — flush entire buffer.
        let normal = std::mem::take(&mut self.buf);
        DetectResult::Normal(normal)
    }

    /// Clear the internal buffer and reset the detector state.
    pub fn reset(&mut self) {
        self.buf.clear();
    }
}

/// Find the first occurrence of the 4-byte ZMODEM preamble in `data`.
fn find_preamble(data: &[u8]) -> Option<usize> {
    data.windows(PREAMBLE.len())
        .position(|w| w == PREAMBLE)
}

/// Determine how many bytes at the tail of `data` could be the start of the
/// preamble `**\x18B`. Returns 0 if the tail does not match any prefix of the
/// preamble.
fn partial_preamble_tail(data: &[u8]) -> usize {
    // Check lengths 3, 2, 1 (longest first).
    for len in (1..PREAMBLE.len()).rev() {
        if data.len() >= len && data[data.len() - len..] == PREAMBLE[..len] {
            return len;
        }
    }
    0
}
