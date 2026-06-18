// src-tauri/src/core/zmodem/sender.rs

use std::io::{Read, Seek, SeekFrom};
use std::path::PathBuf;

use super::{DecodedFrame, FrameType, ESCCTL};
use super::{ZCRCE, ZCRCG, ZCRCQ, ZCRCW};
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
