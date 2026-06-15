/// ZMODEM send state machine.
///
/// Drives the sender side of a ZMODEM file transfer. The caller queues files,
/// generates protocol frames, and reads file data in chunks.

use std::path::PathBuf;

use super::frame::{encode_zhex_header, FrameType, ZmodemFrame};

/// Events emitted by the sender during a transfer.
#[derive(Debug)]
pub enum SendEvent {
    /// The sender is ready to begin.
    Ready,
    /// Progress update for the current file.
    Progress {
        bytes_sent: u64,
        total: u64,
    },
    /// The current file has been fully sent.
    FileComplete {
        name: String,
    },
    /// All queued files have been sent — session is finished.
    AllComplete,
    /// An error occurred.
    Error(String),
}

/// Metadata and read state for a queued file.
struct QueuedFile {
    path: PathBuf,
    name: String,
    size: u64,
    bytes_sent: u64,
    data: Vec<u8>,
    opened: bool,
}

/// ZMODEM send state machine.
pub struct ZmodemSender {
    files: Vec<QueuedFile>,
    current_index: usize,
    complete: bool,
}

impl ZmodemSender {
    /// Create a new sender with an empty file queue.
    pub fn new() -> Self {
        Self {
            files: Vec::new(),
            current_index: 0,
            complete: false,
        }
    }

    /// Add a file to the send queue.
    ///
    /// The file is not opened immediately — call `open_next_file` to begin
    /// sending.
    pub fn add_file(&mut self, path: &str) {
        let p = PathBuf::from(path);
        let name = p
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.to_string());
        self.files.push(QueuedFile {
            path: p,
            name,
            size: 0,
            bytes_sent: 0,
            data: Vec::new(),
            opened: false,
        });
    }

    /// Whether the entire send session has completed.
    pub fn is_complete(&self) -> bool {
        self.complete
    }

    // -----------------------------------------------------------------
    // Frame generators
    // -----------------------------------------------------------------

    /// Generate a ZRQINIT frame to initiate a ZMODEM send session.
    pub fn make_zrqinit(&self) -> Vec<u8> {
        let frame = ZmodemFrame {
            frame_type: FrameType::ZRQINIT,
            flags: [0; 4],
        };
        encode_zhex_header(&frame)
    }

    /// Generate a ZFILE frame advertising the given file `name` and `size`.
    ///
    /// Note: the actual file metadata sub-packet that accompanies ZFILE in
    /// the protocol is separate; this generates only the ZHEX header frame.
    pub fn make_zfile(&self, _name: &str, _size: u64) -> Vec<u8> {
        let frame = ZmodemFrame {
            frame_type: FrameType::ZFILE,
            flags: [0; 4],
        };
        encode_zhex_header(&frame)
    }

    /// Generate a ZEOF frame indicating the end of a file at `offset`.
    pub fn make_zeof(&self, offset: u32) -> Vec<u8> {
        let frame = ZmodemFrame {
            frame_type: FrameType::ZEOF,
            flags: offset.to_le_bytes(),
        };
        encode_zhex_header(&frame)
    }

    /// Generate a ZFIN frame indicating the sender is done.
    pub fn make_zfin(&self) -> Vec<u8> {
        let frame = ZmodemFrame {
            frame_type: FrameType::ZFIN,
            flags: [0; 4],
        };
        encode_zhex_header(&frame)
    }

    // -----------------------------------------------------------------
    // File operations
    // -----------------------------------------------------------------

    /// Open the next queued file for reading.
    ///
    /// Returns `Some(SendEvent::Ready)` if a file was opened, or
    /// `Some(SendEvent::AllComplete)` if there are no more files, or
    /// `Some(SendEvent::Error)` on failure.
    pub fn open_next_file(&mut self) -> SendEvent {
        if self.current_index >= self.files.len() {
            self.complete = true;
            return SendEvent::AllComplete;
        }
        let file = &mut self.files[self.current_index];
        match std::fs::read(&file.path) {
            Ok(data) => {
                file.size = data.len() as u64;
                file.data = data;
                file.opened = true;
                SendEvent::Ready
            }
            Err(e) => SendEvent::Error(format!("failed to open {}: {}", file.path.display(), e)),
        }
    }

    /// Read the next chunk of the current file into `buf`.
    ///
    /// Returns the number of bytes written into `buf`, or 0 if the file has
    /// been fully read.
    pub fn read_chunk(&mut self, buf: &mut [u8]) -> usize {
        if self.current_index >= self.files.len() {
            return 0;
        }
        let file = &mut self.files[self.current_index];
        if !file.opened {
            return 0;
        }
        let sent = file.bytes_sent as usize;
        let remaining = file.data.len().saturating_sub(sent);
        if remaining == 0 {
            return 0;
        }
        let n = buf.len().min(remaining);
        buf[..n].copy_from_slice(&file.data[sent..sent + n]);
        file.bytes_sent += n as u64;
        n
    }

    /// Get the current transfer progress.
    ///
    /// Returns `(bytes_sent, total_size, filename)` for the current file, or
    /// `None` if no file is active.
    pub fn progress(&self) -> Option<(u64, u64, &str)> {
        if self.current_index >= self.files.len() {
            return None;
        }
        let file = &self.files[self.current_index];
        Some((file.bytes_sent, file.size, &file.name))
    }

    /// Advance to the next file in the queue after completing the current one.
    pub fn advance(&mut self) -> SendEvent {
        if self.current_index < self.files.len() {
            let name = self.files[self.current_index].name.clone();
            self.current_index += 1;
            SendEvent::FileComplete { name }
        } else {
            self.complete = true;
            SendEvent::AllComplete
        }
    }
}
