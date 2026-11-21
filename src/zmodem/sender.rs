use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use super::encode::{
    encode_cancel, encode_data_subpacket_with_checksum, encode_over_and_out, encode_zbin16_header,
    encode_zbin32_header, encode_zfile_metadata, encode_zhex_header,
};
use super::receiver::validate_basename;
use super::{
    ChecksumMode, DecodedFrame, FrameType, ZmodemError, CANFC32, ESCCTL, MAX_ZMODEM_FILE_SIZE,
    ZCRCE, ZCRCG, ZCRCW,
};

const SUBPACKET_SIZE: usize = 8192;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct FileIdentity {
    device: u64,
    inode: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileInfo {
    pub path: PathBuf,
    pub name: String,
    pub size: u64,
    pub mtime: u64,
    identity: FileIdentity,
}

impl FileInfo {
    pub fn from_path(path: impl AsRef<Path>) -> Result<Self, ZmodemError> {
        let path = path.as_ref();
        #[cfg(not(unix))]
        {
            let _ = path;
            return Err(unsupported_secure_file_access());
        }
        #[cfg(unix)]
        {
            let file = open_source_file(path)?;
            let metadata = file.metadata()?;
            if metadata.len() > MAX_ZMODEM_FILE_SIZE {
                return Err(ZmodemError::FileTooLarge(metadata.len()));
            }
            let name = path
                .file_name()
                .and_then(|name| name.to_str())
                .filter(|name| !name.is_empty())
                .ok_or_else(|| ZmodemError::UnsafeFilename(path.display().to_string()))?
                .to_string();
            validate_basename(&name)?;
            let mtime = metadata
                .modified()
                .ok()
                .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
                .map_or(0, |duration| duration.as_secs());
            Ok(Self {
                path: path.to_path_buf(),
                name,
                size: metadata.len(),
                mtime,
                identity: file_identity(&metadata),
            })
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SenderAction {
    Send(Vec<u8>),
    Progress {
        bytes_sent: u64,
        total: u64,
        filename: String,
    },
    FileComplete(String),
    AllComplete,
    Error(ZmodemError),
    None,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    Init,
    WaitZrinit,
    WaitFileAccept,
    SendData,
    SentEof,
    WaitZfin,
    Done,
}

pub struct ZmodemSender {
    state: State,
    files: Vec<FileInfo>,
    current_index: usize,
    file: Option<File>,
    file_offset: u64,
    file_size: u64,
    escape_control: bool,
    checksum_mode: ChecksumMode,
    opened_modified: Option<SystemTime>,
}

impl ZmodemSender {
    pub fn new(files: Vec<FileInfo>) -> Result<Self, ZmodemError> {
        if let Some(file) = files.iter().find(|file| file.size > MAX_ZMODEM_FILE_SIZE) {
            return Err(ZmodemError::FileTooLarge(file.size));
        }
        for file in &files {
            validate_basename(&file.name)?;
        }
        #[cfg(not(unix))]
        if !files.is_empty() {
            return Err(unsupported_secure_file_access());
        }
        Ok(Self {
            state: State::Init,
            files,
            current_index: 0,
            file: None,
            file_offset: 0,
            file_size: 0,
            escape_control: false,
            checksum_mode: ChecksumMode::Crc16,
            opened_modified: None,
        })
    }

    pub fn start(&mut self) -> SenderAction {
        if self.state != State::Init {
            return SenderAction::None;
        }
        self.state = State::WaitZrinit;
        SenderAction::Send(encode_zhex_header(FrameType::Zrqinit, [0; 4]))
    }

    pub fn is_done(&self) -> bool {
        self.state == State::Done
    }

    pub fn in_send_data(&self) -> bool {
        self.state == State::SendData
    }

    pub fn current_filename(&self) -> Option<&str> {
        self.files
            .get(self.current_index)
            .map(|file| file.name.as_str())
    }

    pub fn progress(&self) -> Option<SenderAction> {
        self.files
            .get(self.current_index)
            .map(|file| SenderAction::Progress {
                bytes_sent: self.file_offset,
                total: self.file_size,
                filename: file.name.clone(),
            })
    }

    pub fn cancel(&mut self) -> SenderAction {
        self.file = None;
        self.opened_modified = None;
        self.state = State::Done;
        SenderAction::Send(encode_cancel())
    }

    pub fn handle_frame(&mut self, frame: DecodedFrame) -> SenderAction {
        if matches!(frame.frame_type, FrameType::Zabort | FrameType::Zcan) {
            self.file = None;
            self.state = State::Done;
            return SenderAction::Error(ZmodemError::Cancelled);
        }
        match self.state {
            State::WaitZrinit if frame.frame_type == FrameType::Zrinit => {
                self.escape_control = frame.flags[3] & ESCCTL != 0;
                self.checksum_mode = if frame.flags[3] & CANFC32 != 0 {
                    ChecksumMode::Crc32
                } else {
                    ChecksumMode::Crc16
                };
                self.send_zfile()
            }
            State::WaitFileAccept => match frame.frame_type {
                FrameType::Zrpos => self.seek_and_start(frame.offset()),
                FrameType::Zskip => self.advance_file(),
                FrameType::Znak => self.send_zfile(),
                _ => SenderAction::None,
            },
            State::SendData => match frame.frame_type {
                FrameType::Zrpos => self.seek_and_start(frame.offset()),
                FrameType::Zskip => self.advance_file(),
                FrameType::Zack => SenderAction::None,
                _ => SenderAction::None,
            },
            State::SentEof => match frame.frame_type {
                FrameType::Zrinit => self.advance_file(),
                FrameType::Zrpos => self.seek_and_start(frame.offset()),
                FrameType::Zskip => self.advance_file(),
                _ => SenderAction::None,
            },
            State::WaitZfin if frame.frame_type == FrameType::Zfin => {
                self.state = State::Done;
                SenderAction::Send(encode_over_and_out())
            }
            _ => SenderAction::None,
        }
    }

    pub fn next_data_chunk(&mut self) -> Option<SenderAction> {
        if self.state != State::SendData {
            return None;
        }
        let file = self.file.as_mut()?;
        let remaining = match self.file_size.checked_sub(self.file_offset) {
            Some(remaining) => remaining,
            None => {
                return Some(self.fail(ZmodemError::Protocol("发送偏移超过固定文件大小".into())));
            }
        };
        if remaining == 0 {
            if let Err(error) =
                verify_open_file_unchanged(file, self.file_size, self.opened_modified)
            {
                return Some(self.fail(error));
            }
            self.state = State::SentEof;
            let offset = match u32::try_from(self.file_offset) {
                Ok(offset) => offset,
                Err(_) => {
                    self.state = State::Done;
                    return Some(SenderAction::Error(ZmodemError::FileTooLarge(
                        self.file_offset,
                    )));
                }
            };
            let mut output = encode_data_subpacket_with_checksum(
                &[],
                ZCRCE,
                self.escape_control,
                self.checksum_mode,
            )
            .expect("valid terminator");
            output.extend(self.encode_binary_header(FrameType::Zeof, offset.to_le_bytes()));
            return Some(SenderAction::Send(output));
        }
        let mut buffer = [0u8; SUBPACKET_SIZE];
        let read_limit = usize::try_from(remaining.min(SUBPACKET_SIZE as u64))
            .expect("subpacket size always fits usize");
        let read = match file.read(&mut buffer[..read_limit]) {
            Ok(read) => read,
            Err(error) => {
                self.state = State::Done;
                return Some(SenderAction::Error(error.into()));
            }
        };
        if read == 0 {
            return Some(self.fail(ZmodemError::Protocol(format!(
                "文件在发送完成前提前结束: {} / {}",
                self.file_offset, self.file_size
            ))));
        }
        self.file_offset += read as u64;
        Some(SenderAction::Send(
            encode_data_subpacket_with_checksum(
                &buffer[..read],
                ZCRCG,
                self.escape_control,
                self.checksum_mode,
            )
            .expect("valid terminator"),
        ))
    }

    fn send_zfile(&mut self) -> SenderAction {
        let Some(info) = self.files.get(self.current_index) else {
            return self.send_zfin();
        };
        let file = match open_source_file(&info.path) {
            Ok(file) => file,
            Err(error) => return self.fail(error),
        };
        let metadata = match file.metadata() {
            Ok(metadata) => metadata,
            Err(error) => return self.fail(error.into()),
        };
        if file_identity(&metadata) != info.identity {
            return self.fail(ZmodemError::Protocol(format!(
                "发送源在打开期间被替换: {}",
                info.path.display()
            )));
        }
        if metadata.len() > MAX_ZMODEM_FILE_SIZE {
            return self.fail(ZmodemError::FileTooLarge(metadata.len()));
        }
        let opened_mtime = metadata.modified().ok();
        let opened_mtime_secs = opened_mtime
            .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
            .map_or(0, |duration| duration.as_secs());
        if metadata.len() != info.size || opened_mtime_secs != info.mtime {
            return self.fail(ZmodemError::Protocol(format!(
                "发送源自选择后已发生变化: {}",
                info.path.display()
            )));
        }
        self.file_size = metadata.len();
        self.opened_modified = opened_mtime;
        self.file_offset = 0;
        self.file = Some(file);
        self.state = State::WaitFileAccept;

        let mut output = self.encode_binary_header(FrameType::Zfile, [0; 4]);
        let remaining = u32::try_from(self.files.len() - self.current_index).unwrap_or(u32::MAX);
        let metadata = encode_zfile_metadata(
            &info.name,
            self.file_size as u32,
            opened_mtime_secs,
            remaining,
        );
        output.extend(
            encode_data_subpacket_with_checksum(
                &metadata,
                ZCRCW,
                self.escape_control,
                self.checksum_mode,
            )
            .expect("valid terminator"),
        );
        SenderAction::Send(output)
    }

    fn seek_and_start(&mut self, offset: u32) -> SenderAction {
        let offset = u64::from(offset);
        if offset > self.file_size {
            return self.fail(ZmodemError::Protocol(format!(
                "远端请求偏移 {offset} 超过文件大小 {}",
                self.file_size
            )));
        }
        let Some(file) = &mut self.file else {
            return self.fail(ZmodemError::Protocol("发送文件未打开".into()));
        };
        if let Err(error) = file.seek(SeekFrom::Start(offset)) {
            return self.fail(error.into());
        }
        self.file_offset = offset;
        self.state = State::SendData;
        SenderAction::Send(
            self.encode_binary_header(FrameType::Zdata, (offset as u32).to_le_bytes()),
        )
    }

    fn advance_file(&mut self) -> SenderAction {
        self.file = None;
        self.opened_modified = None;
        self.current_index += 1;
        self.send_zfile()
    }

    fn send_zfin(&mut self) -> SenderAction {
        self.state = State::WaitZfin;
        SenderAction::Send(encode_zhex_header(FrameType::Zfin, [0; 4]))
    }

    fn encode_binary_header(&self, frame_type: FrameType, flags: [u8; 4]) -> Vec<u8> {
        match self.checksum_mode {
            ChecksumMode::Crc16 => encode_zbin16_header(frame_type, flags),
            ChecksumMode::Crc32 => encode_zbin32_header(frame_type, flags),
        }
    }

    fn fail(&mut self, error: ZmodemError) -> SenderAction {
        self.file = None;
        self.opened_modified = None;
        self.state = State::Done;
        SenderAction::Error(error)
    }
}

fn verify_open_file_unchanged(
    file: &File,
    expected_size: u64,
    expected_modified: Option<SystemTime>,
) -> Result<(), ZmodemError> {
    // This detects ordinary truncation, growth, and mtime changes. It is not an
    // immutable content snapshot: a same-inode writer that can also restore the
    // exact mtime can still alter bytes during transfer. Callers must not claim
    // cryptographic snapshot semantics for this check.
    let metadata = file.metadata()?;
    if !metadata.is_file()
        || metadata.len() != expected_size
        || metadata.modified().ok() != expected_modified
    {
        return Err(ZmodemError::Protocol(
            "文件在传输期间发生变化，拒绝发送不一致的 ZEOF".into(),
        ));
    }
    Ok(())
}

#[cfg(unix)]
fn file_identity(metadata: &std::fs::Metadata) -> FileIdentity {
    use std::os::unix::fs::MetadataExt;

    FileIdentity {
        device: metadata.dev(),
        inode: metadata.ino(),
    }
}

#[cfg(not(unix))]
fn file_identity(_metadata: &std::fs::Metadata) -> FileIdentity {
    FileIdentity {
        device: 0,
        inode: 0,
    }
}

#[cfg(unix)]
fn open_source_file(path: &Path) -> Result<File, ZmodemError> {
    use std::ffi::CString;
    use std::os::fd::FromRawFd;
    use std::os::unix::ffi::OsStrExt;

    let path = CString::new(path.as_os_str().as_bytes())
        .map_err(|_| ZmodemError::Protocol("发送源路径包含 NUL".into()))?;
    // O_NOFOLLOW closes the final-component symlink race. O_NONBLOCK ensures a
    // raced FIFO/device cannot block before the descriptor is verified by fstat.
    // SAFETY: `path` is live for the syscall; a successful fd is transferred
    // exactly once into `File`.
    let fd = unsafe {
        libc::open(
            path.as_ptr(),
            libc::O_RDONLY | libc::O_CLOEXEC | libc::O_NOFOLLOW | libc::O_NONBLOCK,
        )
    };
    if fd < 0 {
        return Err(ZmodemError::Io(format!(
            "无法安全打开 ZMODEM 发送源: {}",
            std::io::Error::last_os_error()
        )));
    }
    // SAFETY: `fd` was just returned by `open` and has no other owner.
    let file = unsafe { File::from_raw_fd(fd) };
    if !file.metadata()?.is_file() {
        return Err(ZmodemError::Protocol(
            "ZMODEM 发送源不是普通文件，已拒绝设备、FIFO 或目录".into(),
        ));
    }
    Ok(file)
}

#[cfg(not(unix))]
fn open_source_file(_path: &Path) -> Result<File, ZmodemError> {
    Err(unsupported_secure_file_access())
}

#[cfg(not(unix))]
fn unsupported_secure_file_access() -> ZmodemError {
    ZmodemError::Protocol(
        "当前平台不支持安全的 ZMODEM 发送源打开与身份校验，已拒绝不安全降级".into(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::zmodem::decode::{parse_header_prefix, DataSubpacketDecoder, HeaderParse};
    use crate::zmodem::{ChecksumMode, HeaderFormat, ZBIN, ZDLE, ZPAD};
    use std::io::Write;

    fn frame(frame_type: FrameType, offset: u32) -> DecodedFrame {
        DecodedFrame::new(frame_type, offset.to_le_bytes())
    }

    #[test]
    fn retransmit_seeks_and_multiple_files_advance() {
        let directory = tempfile::tempdir().unwrap();
        let first_path = directory.path().join("first.bin");
        let second_path = directory.path().join("second.bin");
        std::fs::write(&first_path, b"abcdefghij").unwrap();
        std::fs::write(&second_path, b"XY").unwrap();
        let mut sender = ZmodemSender::new(vec![
            FileInfo::from_path(&first_path).unwrap(),
            FileInfo::from_path(&second_path).unwrap(),
        ])
        .unwrap();

        assert!(matches!(sender.start(), SenderAction::Send(_)));
        assert!(matches!(
            sender.handle_frame(frame(FrameType::Zrinit, 0)),
            SenderAction::Send(_)
        ));
        assert!(matches!(
            sender.handle_frame(frame(FrameType::Zrpos, 0)),
            SenderAction::Send(_)
        ));
        assert!(matches!(
            sender.next_data_chunk(),
            Some(SenderAction::Send(_))
        ));
        assert!(matches!(
            sender.handle_frame(frame(FrameType::Zrpos, 3)),
            SenderAction::Send(_)
        ));
        assert_eq!(
            sender.progress(),
            Some(SenderAction::Progress {
                bytes_sent: 3,
                total: 10,
                filename: "first.bin".into(),
            })
        );
        assert!(matches!(
            sender.next_data_chunk(),
            Some(SenderAction::Send(_))
        ));
        assert!(matches!(
            sender.next_data_chunk(),
            Some(SenderAction::Send(_))
        ));
        assert!(matches!(
            sender.handle_frame(frame(FrameType::Zrinit, 0)),
            SenderAction::Send(_)
        ));
        assert_eq!(sender.current_filename(), Some("second.bin"));
    }

    #[test]
    fn empty_file_reaches_eof_without_offset_overflow() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("empty");
        std::fs::write(&path, []).unwrap();
        let mut sender = ZmodemSender::new(vec![FileInfo::from_path(&path).unwrap()]).unwrap();
        sender.start();
        sender.handle_frame(frame(FrameType::Zrinit, 0));
        sender.handle_frame(frame(FrameType::Zrpos, 0));
        assert!(matches!(
            sender.next_data_chunk(),
            Some(SenderAction::Send(_))
        ));
    }

    #[test]
    fn constructor_rejects_files_above_wire_offset_limit() {
        let file = FileInfo {
            path: "unused".into(),
            name: "large".into(),
            size: MAX_ZMODEM_FILE_SIZE + 1,
            mtime: 0,
            identity: FileIdentity {
                device: 0,
                inode: 0,
            },
        };
        assert!(matches!(
            ZmodemSender::new(vec![file]),
            Err(ZmodemError::FileTooLarge(_))
        ));
    }

    #[test]
    fn peer_without_canfc32_gets_crc16_headers_and_data() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("crc16.bin");
        std::fs::write(&path, b"abc").unwrap();
        let mut sender = ZmodemSender::new(vec![FileInfo::from_path(&path).unwrap()]).unwrap();
        sender.start();

        let SenderAction::Send(zfile) =
            sender.handle_frame(DecodedFrame::new(FrameType::Zrinit, [0; 4]))
        else {
            panic!("ZRINIT should start ZFILE");
        };
        assert!(zfile.starts_with(&[ZPAD, ZDLE, ZBIN]));
        let HeaderParse::Complete { frame, consumed } = parse_header_prefix(&zfile) else {
            panic!("CRC16 ZFILE header should decode");
        };
        assert_eq!(frame.format, HeaderFormat::Binary16);
        assert_eq!(frame.frame_type, FrameType::Zfile);
        let mut data = DataSubpacketDecoder::default();
        data.set_checksum_mode(ChecksumMode::Crc16);
        assert!(data.feed_one(&zfile[consumed..]).result.unwrap().is_ok());

        let SenderAction::Send(zdata) =
            sender.handle_frame(DecodedFrame::new(FrameType::Zrpos, [0; 4]))
        else {
            panic!("ZRPOS should start ZDATA");
        };
        let HeaderParse::Complete { frame, .. } = parse_header_prefix(&zdata) else {
            panic!("CRC16 ZDATA header should decode");
        };
        assert_eq!(frame.format, HeaderFormat::Binary16);

        let Some(SenderAction::Send(chunk)) = sender.next_data_chunk() else {
            panic!("sender should emit a CRC16 data packet");
        };
        let mut data = DataSubpacketDecoder::default();
        data.set_checksum_mode(ChecksumMode::Crc16);
        assert_eq!(data.feed_one(&chunk).result.unwrap().unwrap().data, b"abc");
    }

    #[cfg(unix)]
    #[test]
    fn rejects_symlink_device_and_fifo_sources() {
        use std::ffi::CString;
        use std::os::unix::ffi::OsStrExt;
        use std::os::unix::fs::symlink;

        let directory = tempfile::tempdir().unwrap();
        let regular = directory.path().join("regular");
        let link = directory.path().join("link");
        let fifo = directory.path().join("fifo");
        std::fs::write(&regular, b"x").unwrap();
        symlink(&regular, &link).unwrap();
        let fifo_c = CString::new(fifo.as_os_str().as_bytes()).unwrap();
        // SAFETY: `fifo_c` is a live NUL-terminated path and the mode is valid.
        assert_eq!(unsafe { libc::mkfifo(fifo_c.as_ptr(), 0o600) }, 0);

        assert!(FileInfo::from_path(&link).is_err());
        assert!(FileInfo::from_path(&fifo).is_err());
        assert!(FileInfo::from_path("/dev/null").is_err());
    }

    #[cfg(unix)]
    #[test]
    fn rejects_source_replaced_by_symlink_after_selection() {
        use std::os::unix::fs::symlink;

        let directory = tempfile::tempdir().unwrap();
        let selected_path = directory.path().join("selected");
        let replacement_path = directory.path().join("replacement");
        std::fs::write(&selected_path, b"selected").unwrap();
        std::fs::write(&replacement_path, b"private!").unwrap();
        let info = FileInfo::from_path(&selected_path).unwrap();
        std::fs::remove_file(&selected_path).unwrap();
        symlink(&replacement_path, &selected_path).unwrap();

        let mut sender = ZmodemSender::new(vec![info]).unwrap();
        sender.start();
        assert!(matches!(
            sender.handle_frame(frame(FrameType::Zrinit, 0)),
            SenderAction::Error(ZmodemError::Io(_))
        ));
        assert!(sender.is_done());
    }

    #[test]
    fn truncation_and_growth_never_emit_inconsistent_zeof() {
        let directory = tempfile::tempdir().unwrap();

        let truncated_path = directory.path().join("truncated");
        std::fs::write(&truncated_path, b"abc").unwrap();
        let mut truncated =
            ZmodemSender::new(vec![FileInfo::from_path(&truncated_path).unwrap()]).unwrap();
        truncated.start();
        truncated.handle_frame(DecodedFrame::new(FrameType::Zrinit, [0; 4]));
        truncated.handle_frame(DecodedFrame::new(FrameType::Zrpos, [0; 4]));
        std::fs::OpenOptions::new()
            .write(true)
            .open(&truncated_path)
            .unwrap()
            .set_len(1)
            .unwrap();
        assert!(matches!(
            truncated.next_data_chunk(),
            Some(SenderAction::Send(_))
        ));
        assert!(matches!(
            truncated.next_data_chunk(),
            Some(SenderAction::Error(ZmodemError::Protocol(_)))
        ));

        let grown_path = directory.path().join("grown");
        std::fs::write(&grown_path, b"abc").unwrap();
        let mut grown = ZmodemSender::new(vec![FileInfo::from_path(&grown_path).unwrap()]).unwrap();
        grown.start();
        grown.handle_frame(DecodedFrame::new(FrameType::Zrinit, [0; 4]));
        grown.handle_frame(DecodedFrame::new(FrameType::Zrpos, [0; 4]));
        assert!(matches!(
            grown.next_data_chunk(),
            Some(SenderAction::Send(_))
        ));
        std::fs::OpenOptions::new()
            .append(true)
            .open(&grown_path)
            .unwrap()
            .write_all(b"d")
            .unwrap();
        assert!(matches!(
            grown.next_data_chunk(),
            Some(SenderAction::Error(ZmodemError::Protocol(_)))
        ));
    }
}
