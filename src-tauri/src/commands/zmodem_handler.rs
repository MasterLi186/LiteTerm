/// ZMODEM protocol integration for terminal byte streams.
///
/// Wraps the core ZMODEM detector and receiver to filter terminal output,
/// intercept ZMODEM transfers, and emit progress events to the frontend.

use crate::core::zmodem::detect::{DetectResult, ZmodemDetector};
use crate::core::zmodem::frame::{decode_zhex_header, FrameType};
use crate::core::zmodem::receive::{ReceiveEvent, ZmodemReceiver};
use tauri::{AppHandle, Emitter};

/// Filters terminal output bytes, detecting and handling ZMODEM transfers.
pub struct ZmodemFilter {
    detector: ZmodemDetector,
    receiver: Option<ZmodemReceiver>,
    download_dir: String,
    in_transfer: bool,
}

impl ZmodemFilter {
    pub fn new(download_dir: &str) -> Self {
        Self {
            detector: ZmodemDetector::new(),
            receiver: None,
            download_dir: download_dir.to_string(),
            in_transfer: false,
        }
    }

    /// Feed incoming bytes from terminal output.
    ///
    /// Returns `(bytes_for_xterm, response_bytes_to_write_back)`.
    /// During a ZMODEM transfer, `bytes_for_xterm` will be empty and
    /// `response_bytes` contains protocol responses to send back to the PTY/SSH.
    pub fn process_incoming(
        &mut self,
        data: &[u8],
        app: &AppHandle,
        terminal_id: &str,
    ) -> (Vec<u8>, Vec<u8>) {
        if self.in_transfer {
            return self.handle_zmodem_data(data, app, terminal_id);
        }

        // Normal mode — scan for ZMODEM start
        let result = self.detector.feed(data);
        match result {
            DetectResult::Normal(bytes) => (bytes, Vec::new()),
            DetectResult::ZmodemStart {
                preceding,
                frame: _,
                remaining: _,
            } => {
                self.in_transfer = true;
                let receiver = ZmodemReceiver::new(&self.download_dir);
                let zrinit = receiver.make_zrinit();
                self.receiver = Some(receiver);

                let _ = app.emit(
                    "zmodem-start",
                    serde_json::json!({
                        "id": terminal_id,
                        "direction": "download",
                    }),
                );

                (preceding, zrinit)
            }
        }
    }

    fn handle_zmodem_data(
        &mut self,
        data: &[u8],
        app: &AppHandle,
        terminal_id: &str,
    ) -> (Vec<u8>, Vec<u8>) {
        let receiver = match self.receiver.as_mut() {
            Some(r) => r,
            None => {
                self.in_transfer = false;
                return (data.to_vec(), Vec::new());
            }
        };

        let mut response = Vec::new();

        // Try to decode a ZHEX header from the data
        if let Some(frame) = decode_zhex_header(data) {
            match frame.frame_type {
                FrameType::ZFILE => {
                    // Extract filename and size from the subpacket after the header.
                    // The header format is: preamble(4) + hex(14) + optional CRLF
                    let marker = b"**\x18B";
                    if let Some(pos) = data.windows(marker.len()).position(|w| w == marker) {
                        let after_header = pos + marker.len() + 14;
                        // Skip optional CR LF
                        let mut skip = after_header;
                        if skip < data.len() && data[skip] == b'\r' {
                            skip += 1;
                        }
                        if skip < data.len() && data[skip] == b'\n' {
                            skip += 1;
                        }
                        if skip < data.len() {
                            let subpacket = &data[skip..];
                            if let Some((name, size)) = receiver.parse_zfile_subpacket(subpacket) {
                                let _ = receiver.begin_file(&name, size);
                                let _ = app.emit(
                                    "zmodem-progress",
                                    serde_json::json!({
                                        "id": terminal_id,
                                        "filename": name,
                                        "bytes_received": 0,
                                        "total_size": size,
                                        "status": "receiving",
                                    }),
                                );
                                response = receiver.make_zrpos(0);
                            }
                        }
                    }
                }
                FrameType::ZEOF => {
                    let event = receiver.end_file();
                    if let ReceiveEvent::FileComplete {
                        filename, path, ..
                    } = event
                    {
                        let _ = app.emit(
                            "zmodem-progress",
                            serde_json::json!({
                                "id": terminal_id,
                                "filename": filename,
                                "bytes_received": 0,
                                "total_size": 0,
                                "status": "complete",
                                "path": path.to_string_lossy(),
                            }),
                        );
                    }
                    response = receiver.make_zrinit();
                }
                FrameType::ZFIN => {
                    response = receiver.make_zfin();
                    self.in_transfer = false;
                    self.receiver = None;
                    self.detector.reset();
                    let _ = app.emit(
                        "zmodem-end",
                        serde_json::json!({
                            "id": terminal_id,
                        }),
                    );
                }
                _ => {}
            }
        } else {
            // Not a header frame — treat as data payload for the current file
            if let Ok(event) = receiver.write_data(data) {
                if let ReceiveEvent::Progress {
                    filename,
                    bytes_received,
                    total_size,
                } = event
                {
                    let _ = app.emit(
                        "zmodem-progress",
                        serde_json::json!({
                            "id": terminal_id,
                            "filename": filename,
                            "bytes_received": bytes_received,
                            "total_size": total_size,
                            "status": "receiving",
                        }),
                    );
                }
                response = receiver.make_zack(0);
            }
        }

        // Don't forward ZMODEM protocol data to xterm.js
        (Vec::new(), response)
    }
}
