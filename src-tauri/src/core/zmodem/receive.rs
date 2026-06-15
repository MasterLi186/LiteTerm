/// ZMODEM receive state machine.
///
/// Drives the receiver side of a ZMODEM file transfer. The caller feeds
/// incoming data and retrieves outgoing response frames to send back to the
/// remote sender.

use std::path::PathBuf;

use super::frame::{encode_zhex_header, FrameType, ZmodemFrame};

/// Events emitted by the receiver during a transfer.
#[derive(Debug)]
pub enum ReceiveEvent {
    /// A new file transfer has started.
    FileStart {
        name: String,
        size: u64,
    },
    /// Progress update for the current file.
    Progress {
        filename: String,
        bytes_received: u64,
        total_size: u64,
    },
    /// The current file has been fully received.
    FileComplete {
        filename: String,
        path: PathBuf,
    },
    /// All files have been received — session is finished.
    AllComplete,
    /// An error occurred.
    Error(String),
}

/// State of the file currently being received.
struct CurrentFile {
    name: String,
    size: u64,
    bytes_written: u64,
    data: Vec<u8>,
}

/// ZMODEM receive state machine.
pub struct ZmodemReceiver {
    download_dir: PathBuf,
    current: Option<CurrentFile>,
    files_received: u32,
    complete: bool,
}

impl ZmodemReceiver {
    /// Create a new receiver that will save files under `download_dir`.
    ///
    /// The path is expanded with `shellexpand::tilde` so that `~/Downloads`
    /// resolves correctly.
    pub fn new(download_dir: &str) -> Self {
        let expanded = shellexpand::tilde(download_dir).into_owned();
        Self {
            download_dir: PathBuf::from(expanded),
            current: None,
            files_received: 0,
            complete: false,
        }
    }

    /// Whether the entire transfer session has completed.
    pub fn is_complete(&self) -> bool {
        self.complete
    }

    /// Number of files received so far.
    pub fn files_received(&self) -> u32 {
        self.files_received
    }

    // -----------------------------------------------------------------
    // Frame generators
    // -----------------------------------------------------------------

    /// Generate a ZRINIT frame to send to the remote sender.
    ///
    /// Flags advertise receiver capabilities; for now we use a minimal set.
    pub fn make_zrinit(&self) -> Vec<u8> {
        let frame = ZmodemFrame {
            frame_type: FrameType::ZRINIT,
            flags: [0, 0, 0, 0x23], // CANFDX | CANOVIO | CANFC32
        };
        encode_zhex_header(&frame)
    }

    /// Generate a ZRPOS frame requesting data starting at `offset`.
    pub fn make_zrpos(&self, offset: u32) -> Vec<u8> {
        let frame = ZmodemFrame {
            frame_type: FrameType::ZRPOS,
            flags: offset.to_le_bytes(),
        };
        encode_zhex_header(&frame)
    }

    /// Generate a ZACK frame acknowledging receipt up to `offset`.
    pub fn make_zack(&self, offset: u32) -> Vec<u8> {
        let frame = ZmodemFrame {
            frame_type: FrameType::ZACK,
            flags: offset.to_le_bytes(),
        };
        encode_zhex_header(&frame)
    }

    /// Generate a ZFIN frame indicating the receiver is done.
    pub fn make_zfin(&self) -> Vec<u8> {
        let frame = ZmodemFrame {
            frame_type: FrameType::ZFIN,
            flags: [0; 4],
        };
        encode_zhex_header(&frame)
    }

    // -----------------------------------------------------------------
    // Subpacket parsing
    // -----------------------------------------------------------------

    /// Parse a ZFILE sub-packet to extract the filename and file size.
    ///
    /// The sub-packet format is:
    /// `filename\0size modification_time ...\0`
    ///
    /// Returns `(filename, size)` where `size` is 0 if not present.
    pub fn parse_zfile_subpacket(&self, data: &[u8]) -> Option<(String, u64)> {
        // Find the first NUL — everything before it is the filename.
        let nul_pos = data.iter().position(|&b| b == 0)?;
        let name = std::str::from_utf8(&data[..nul_pos]).ok()?;
        if name.is_empty() {
            return None;
        }

        // After the NUL comes the file metadata as space-separated ASCII
        // fields. The first field is the file size in decimal.
        let meta_start = nul_pos + 1;
        let size = if meta_start < data.len() {
            // Find the extent of the metadata string (up to next NUL or end).
            let meta_end = data[meta_start..]
                .iter()
                .position(|&b| b == 0)
                .map_or(data.len(), |p| meta_start + p);
            let meta_str = std::str::from_utf8(&data[meta_start..meta_end]).unwrap_or("");
            // First space-separated token is the size.
            meta_str
                .split_whitespace()
                .next()
                .and_then(|s| s.parse::<u64>().ok())
                .unwrap_or(0)
        } else {
            0
        };

        Some((name.to_string(), size))
    }

    // -----------------------------------------------------------------
    // File lifecycle
    // -----------------------------------------------------------------

    /// Begin receiving a new file.
    pub fn begin_file(&mut self, name: &str, size: u64) -> ReceiveEvent {
        self.current = Some(CurrentFile {
            name: name.to_string(),
            size,
            bytes_written: 0,
            data: Vec::with_capacity(size as usize),
        });
        ReceiveEvent::FileStart {
            name: name.to_string(),
            size,
        }
    }

    /// Append a chunk of data to the current file.
    pub fn write_data(&mut self, data: &[u8]) -> Result<ReceiveEvent, String> {
        if let Some(ref mut file) = self.current {
            file.data.extend_from_slice(data);
            file.bytes_written += data.len() as u64;
            Ok(ReceiveEvent::Progress {
                filename: file.name.clone(),
                bytes_received: file.bytes_written,
                total_size: file.size,
            })
        } else {
            Err("no file in progress".to_string())
        }
    }

    /// Finalize the current file and flush data to disk.
    pub fn end_file(&mut self) -> ReceiveEvent {
        if let Some(file) = self.current.take() {
            let dest = self.download_dir.join(&file.name);
            // Ensure download directory exists
            let _ = std::fs::create_dir_all(&self.download_dir);
            if let Err(e) = std::fs::write(&dest, &file.data) {
                return ReceiveEvent::Error(format!("failed to write {}: {}", dest.display(), e));
            }
            self.files_received += 1;
            ReceiveEvent::FileComplete {
                filename: file.name,
                path: dest,
            }
        } else {
            ReceiveEvent::Error("no file in progress".to_string())
        }
    }

    /// Mark the entire transfer as finished.
    pub fn finish(&mut self) -> ReceiveEvent {
        self.complete = true;
        ReceiveEvent::AllComplete
    }

    /// Path where received files would be written.
    pub fn download_dir(&self) -> &PathBuf {
        &self.download_dir
    }
}
