use std::fs::File;
use std::io::{ErrorKind, Write};
use std::path::{Component, Path, PathBuf};

use super::decode::{DataEnd, DataSubpacket};
use super::encode::{encode_cancel, encode_zhex_header};
use super::{DecodedFrame, FrameType, ZmodemError, CANFC32, CANFDX, CANOVIO, MAX_ZMODEM_FILE_SIZE};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReceiverEvent {
    Progress {
        bytes_received: u64,
        total: u64,
        filename: String,
    },
    FileComplete {
        path: PathBuf,
        size: u64,
    },
    SessionComplete,
    Error(ZmodemError),
    Cancelled,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ReceiverOutput {
    pub writes: Vec<Vec<u8>>,
    pub events: Vec<ReceiverEvent>,
}

impl ReceiverOutput {
    fn send(bytes: Vec<u8>) -> Self {
        Self {
            writes: vec![bytes],
            events: Vec::new(),
        }
    }

    fn error(error: ZmodemError) -> Self {
        Self {
            writes: Vec::new(),
            events: vec![ReceiverEvent::Error(error)],
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    AwaitHeader,
    AwaitMetadata,
    AwaitDataHeader,
    ReceivingData,
    AwaitEof,
    Finishing,
    Done,
}

struct IncomingFile {
    name: String,
    declared_size: u64,
    bytes_written: u64,
    target_name: String,
    temporary_name: String,
    file: File,
}

pub struct ZmodemReceiver {
    destination: PathBuf,
    #[cfg(target_os = "linux")]
    directory: File,
    state: State,
    incoming: Option<IncomingFile>,
}

impl ZmodemReceiver {
    pub fn new(destination: impl AsRef<Path>) -> Result<Self, ZmodemError> {
        let destination = destination.as_ref();
        #[cfg(not(target_os = "linux"))]
        {
            let _ = destination;
            return Err(unsupported_secure_file_access("接收"));
        }
        #[cfg(target_os = "linux")]
        {
            let directory = open_receive_directory(destination)?;
            let destination =
                current_directory_path(&directory).unwrap_or_else(|| destination.to_path_buf());
            Ok(Self {
                destination,
                directory,
                state: State::AwaitHeader,
                incoming: None,
            })
        }
    }

    pub fn start(&self) -> ReceiverOutput {
        ReceiverOutput::send(zrinit())
    }

    pub fn is_done(&self) -> bool {
        self.state == State::Done
    }

    pub fn is_finishing(&self) -> bool {
        self.state == State::Finishing
    }

    pub fn expects_data_subpacket(&self) -> bool {
        matches!(self.state, State::AwaitMetadata | State::ReceivingData)
    }

    pub fn has_partial_file(&self) -> bool {
        self.incoming.is_some()
    }

    pub fn handle_header(&mut self, frame: DecodedFrame) -> ReceiverOutput {
        if matches!(frame.frame_type, FrameType::Zcan | FrameType::Zabort) {
            self.cleanup_partial();
            self.state = State::Done;
            return ReceiverOutput {
                writes: Vec::new(),
                events: vec![ReceiverEvent::Cancelled],
            };
        }
        match frame.frame_type {
            FrameType::Zrqinit => ReceiverOutput::send(zrinit()),
            FrameType::Zfile if self.state == State::AwaitHeader => {
                self.state = State::AwaitMetadata;
                ReceiverOutput::default()
            }
            FrameType::Zdata if matches!(self.state, State::AwaitDataHeader | State::AwaitEof) => {
                let expected = self
                    .incoming
                    .as_ref()
                    .map_or(0, |incoming| incoming.bytes_written);
                if u64::from(frame.offset()) != expected {
                    return ReceiverOutput::send(zrpos(expected as u32));
                }
                self.state = State::ReceivingData;
                ReceiverOutput::default()
            }
            FrameType::Zeof if matches!(self.state, State::AwaitEof | State::AwaitDataHeader) => {
                self.handle_eof(frame.offset())
            }
            FrameType::Zfin if self.state == State::AwaitHeader => {
                self.state = State::Finishing;
                ReceiverOutput::send(encode_zhex_header(FrameType::Zfin, [0; 4]))
            }
            FrameType::Zskip => {
                self.cleanup_partial();
                self.state = State::AwaitHeader;
                ReceiverOutput::default()
            }
            _ => ReceiverOutput::default(),
        }
    }

    pub fn handle_data(&mut self, packet: DataSubpacket) -> ReceiverOutput {
        match self.state {
            State::AwaitMetadata => self.handle_metadata(packet),
            State::ReceivingData => self.handle_file_data(packet),
            _ => ReceiverOutput::error(ZmodemError::Protocol(
                "当前状态不接受 ZMODEM 数据子包".into(),
            )),
        }
    }

    pub fn handle_over_and_out(&mut self, bytes: &[u8]) -> ReceiverOutput {
        if self.state == State::Finishing && bytes.starts_with(b"OO") {
            self.state = State::Done;
            return ReceiverOutput {
                writes: Vec::new(),
                events: vec![ReceiverEvent::SessionComplete],
            };
        }
        ReceiverOutput::default()
    }

    pub fn cancel(&mut self) -> ReceiverOutput {
        self.cleanup_partial();
        self.state = State::Done;
        ReceiverOutput {
            writes: vec![encode_cancel()],
            events: vec![ReceiverEvent::Cancelled],
        }
    }

    fn handle_metadata(&mut self, packet: DataSubpacket) -> ReceiverOutput {
        if !matches!(packet.end, DataEnd::End | DataEnd::EndAck) {
            self.state = State::AwaitHeader;
            return ReceiverOutput::error(ZmodemError::Protocol("ZFILE 元数据未正确结束".into()));
        }
        let (name, size) = match parse_metadata(&packet.data) {
            Ok(metadata) => metadata,
            Err(error) => {
                self.state = State::AwaitHeader;
                return ReceiverOutput {
                    writes: vec![encode_zhex_header(FrameType::Zskip, [0; 4])],
                    events: vec![ReceiverEvent::Error(error)],
                };
            }
        };
        let incoming = match self.create_incoming(name, size) {
            Ok(incoming) => incoming,
            Err(error) => {
                self.state = State::AwaitHeader;
                return ReceiverOutput {
                    writes: vec![encode_zhex_header(FrameType::Zskip, [0; 4])],
                    events: vec![ReceiverEvent::Error(error)],
                };
            }
        };
        self.incoming = Some(incoming);
        self.state = State::AwaitDataHeader;
        ReceiverOutput::send(zrpos(0))
    }

    fn handle_file_data(&mut self, packet: DataSubpacket) -> ReceiverOutput {
        let Some(incoming) = &mut self.incoming else {
            self.state = State::AwaitHeader;
            return ReceiverOutput::error(ZmodemError::Protocol(
                "收到数据但没有打开的接收文件".into(),
            ));
        };
        let Some(next_offset) = incoming.bytes_written.checked_add(packet.data.len() as u64) else {
            self.cleanup_partial();
            self.state = State::AwaitHeader;
            return ReceiverOutput::error(ZmodemError::FileTooLarge(u64::MAX));
        };
        if next_offset > incoming.declared_size || next_offset > MAX_ZMODEM_FILE_SIZE {
            let error = ZmodemError::Protocol(format!(
                "接收数据超过声明大小: {next_offset} > {}",
                incoming.declared_size
            ));
            self.cleanup_partial();
            self.state = State::AwaitHeader;
            return ReceiverOutput {
                writes: vec![encode_zhex_header(FrameType::Zferr, [0; 4])],
                events: vec![ReceiverEvent::Error(error)],
            };
        }
        if let Err(error) = incoming.file.write_all(&packet.data) {
            self.cleanup_partial();
            self.state = State::AwaitHeader;
            return ReceiverOutput {
                writes: vec![encode_zhex_header(FrameType::Zferr, [0; 4])],
                events: vec![ReceiverEvent::Error(error.into())],
            };
        }
        incoming.bytes_written = next_offset;
        let progress = ReceiverEvent::Progress {
            bytes_received: next_offset,
            total: incoming.declared_size,
            filename: incoming.name.clone(),
        };
        let mut output = ReceiverOutput {
            writes: Vec::new(),
            events: vec![progress],
        };
        match packet.end {
            DataEnd::Continue => {}
            DataEnd::ContinueAck => {
                output.writes.push(zack(next_offset as u32));
            }
            DataEnd::End => self.state = State::AwaitEof,
            DataEnd::EndAck => {
                output.writes.push(zack(next_offset as u32));
                self.state = State::AwaitDataHeader;
            }
        }
        output
    }

    fn handle_eof(&mut self, offset: u32) -> ReceiverOutput {
        let Some(incoming) = self.incoming.as_ref() else {
            self.state = State::AwaitHeader;
            return ReceiverOutput::send(zrinit());
        };
        if u64::from(offset) != incoming.bytes_written {
            return ReceiverOutput::send(zrpos(incoming.bytes_written as u32));
        }
        if incoming.bytes_written != incoming.declared_size {
            let error = ZmodemError::Protocol(format!(
                "文件大小不匹配: 声明 {}，实际 {}",
                incoming.declared_size, incoming.bytes_written
            ));
            self.cleanup_partial();
            self.state = State::AwaitHeader;
            return ReceiverOutput {
                writes: vec![encode_zhex_header(FrameType::Zferr, [0; 4])],
                events: vec![ReceiverEvent::Error(error)],
            };
        }
        match self.finish_file() {
            Ok((path, size)) => {
                self.state = State::AwaitHeader;
                ReceiverOutput {
                    writes: vec![zrinit()],
                    events: vec![ReceiverEvent::FileComplete { path, size }],
                }
            }
            Err(error) => {
                self.cleanup_partial();
                self.state = State::AwaitHeader;
                ReceiverOutput {
                    writes: vec![encode_zhex_header(FrameType::Zferr, [0; 4])],
                    events: vec![ReceiverEvent::Error(error)],
                }
            }
        }
    }

    fn create_incoming(&self, name: String, size: u64) -> Result<IncomingFile, ZmodemError> {
        validate_basename(&name)?;
        if size > MAX_ZMODEM_FILE_SIZE {
            return Err(ZmodemError::FileTooLarge(size));
        }
        #[cfg(not(target_os = "linux"))]
        {
            let _ = (name, size);
            return Err(unsupported_secure_file_access("接收"));
        }
        #[cfg(target_os = "linux")]
        {
            if entry_exists_at(&self.directory, &name)? {
                return Err(ZmodemError::DestinationExists(name));
            }
            for _ in 0..32 {
                let temporary_name = format!(".{name}.{}.part", uuid::Uuid::new_v4());
                match openat_new_file(&self.directory, &temporary_name) {
                    Ok(file) => {
                        return Ok(IncomingFile {
                            target_name: name.clone(),
                            name,
                            declared_size: size,
                            bytes_written: 0,
                            temporary_name,
                            file,
                        });
                    }
                    Err(error) if error.kind() == ErrorKind::AlreadyExists => continue,
                    Err(error) => return Err(error.into()),
                }
            }
            Err(ZmodemError::Io("无法创建唯一的 ZMODEM 临时文件".into()))
        }
    }

    fn finish_file(&mut self) -> Result<(PathBuf, u64), ZmodemError> {
        #[cfg(not(target_os = "linux"))]
        {
            self.cleanup_partial();
            return Err(unsupported_secure_file_access("接收"));
        }
        #[cfg(target_os = "linux")]
        {
            let mut incoming = self
                .incoming
                .take()
                .ok_or_else(|| ZmodemError::Protocol("没有待完成的接收文件".into()))?;
            if let Err(error) = incoming.file.flush() {
                drop(incoming.file);
                self.remove_temporary(&incoming.temporary_name);
                return Err(error.into());
            }
            if let Err(error) = incoming.file.sync_all() {
                drop(incoming.file);
                self.remove_temporary(&incoming.temporary_name);
                return Err(error.into());
            }
            let size = incoming.bytes_written;
            drop(incoming.file);
            let rename_result: Result<(), ZmodemError> = rename_noreplace_at(
                &self.directory,
                &incoming.temporary_name,
                &incoming.target_name,
            )
            .map_err(|error| {
                if error.kind() == ErrorKind::AlreadyExists {
                    ZmodemError::DestinationExists(incoming.name.clone())
                } else {
                    error.into()
                }
            });
            if let Err(error) = rename_result {
                self.remove_temporary(&incoming.temporary_name);
                return Err(error);
            }
            let _ = self.directory.sync_all();
            let destination =
                current_directory_path(&self.directory).unwrap_or_else(|| self.destination.clone());
            Ok((destination.join(&incoming.target_name), size))
        }
    }

    fn cleanup_partial(&mut self) {
        if let Some(incoming) = self.incoming.take() {
            drop(incoming.file);
            self.remove_temporary(&incoming.temporary_name);
        }
    }

    fn remove_temporary(&self, temporary_name: &str) {
        #[cfg(target_os = "linux")]
        let _ = unlinkat_file(&self.directory, temporary_name);
        #[cfg(not(target_os = "linux"))]
        let _ = temporary_name;
    }
}

impl Drop for ZmodemReceiver {
    fn drop(&mut self) {
        self.cleanup_partial();
    }
}

pub fn validate_basename(name: &str) -> Result<(), ZmodemError> {
    let path = Path::new(name);
    let valid_component = matches!(
        path.components().collect::<Vec<_>>().as_slice(),
        [Component::Normal(_)]
    );
    if name.is_empty()
        || name == "."
        || name == ".."
        || name.contains(['/', '\\', '\0'])
        || Path::new(name).is_absolute()
        || !valid_component
        || name.contains(':')
        || name.chars().any(|character| {
            character <= '\u{1f}' || matches!(character, '<' | '>' | '"' | '|' | '?' | '*')
        })
        || name.ends_with(['.', ' '])
        || is_windows_reserved_name(name)
    {
        return Err(ZmodemError::UnsafeFilename(name.into()));
    }
    Ok(())
}

fn is_windows_reserved_name(name: &str) -> bool {
    let stem = name.split('.').next().unwrap_or(name);
    let uppercase = stem.to_ascii_uppercase();
    matches!(uppercase.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || uppercase.strip_prefix("COM").is_some_and(|suffix| {
            matches!(
                suffix,
                "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "¹" | "²" | "³"
            )
        })
        || uppercase.strip_prefix("LPT").is_some_and(|suffix| {
            matches!(
                suffix,
                "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "¹" | "²" | "³"
            )
        })
}

#[cfg(not(target_os = "linux"))]
fn unsupported_secure_file_access(operation: &str) -> ZmodemError {
    ZmodemError::Protocol(format!(
        "当前平台不支持安全的 ZMODEM 文件{operation}，已拒绝不安全降级"
    ))
}

fn parse_metadata(data: &[u8]) -> Result<(String, u64), ZmodemError> {
    let separator = data
        .iter()
        .position(|byte| *byte == 0)
        .ok_or_else(|| ZmodemError::Protocol("ZFILE 元数据缺少文件名终止符".into()))?;
    let name = std::str::from_utf8(&data[..separator])
        .map_err(|_| ZmodemError::UnsafeFilename("非 UTF-8 文件名".into()))?
        .to_string();
    validate_basename(&name)?;
    let info_end = data[separator + 1..]
        .iter()
        .position(|byte| *byte == 0)
        .map_or(data.len(), |offset| separator + 1 + offset);
    let info = std::str::from_utf8(&data[separator + 1..info_end])
        .map_err(|_| ZmodemError::Protocol("ZFILE 大小字段不是 UTF-8".into()))?;
    let size = info
        .split_ascii_whitespace()
        .next()
        .ok_or_else(|| ZmodemError::Protocol("ZFILE 元数据缺少大小".into()))?
        .parse::<u64>()
        .map_err(|_| ZmodemError::Protocol("ZFILE 文件大小无效".into()))?;
    if size > MAX_ZMODEM_FILE_SIZE {
        return Err(ZmodemError::FileTooLarge(size));
    }
    Ok((name, size))
}

fn zrinit() -> Vec<u8> {
    encode_zhex_header(FrameType::Zrinit, [0, 0, 0, CANFDX | CANOVIO | CANFC32])
}

fn zrpos(offset: u32) -> Vec<u8> {
    encode_zhex_header(FrameType::Zrpos, offset.to_le_bytes())
}

fn zack(offset: u32) -> Vec<u8> {
    encode_zhex_header(FrameType::Zack, offset.to_le_bytes())
}

#[cfg(target_os = "linux")]
fn rename_noreplace_at(directory: &File, source: &str, destination: &str) -> std::io::Result<()> {
    use std::ffi::CString;
    use std::os::fd::AsRawFd;

    let source = CString::new(source)
        .map_err(|_| std::io::Error::new(ErrorKind::InvalidInput, "NUL in source name"))?;
    let destination = CString::new(destination)
        .map_err(|_| std::io::Error::new(ErrorKind::InvalidInput, "NUL in destination name"))?;
    // SAFETY: both pointers come from live CStrings and flags/dirfds follow
    // renameat2(2). No Rust memory is aliased or retained by the syscall.
    let result = unsafe {
        libc::renameat2(
            directory.as_raw_fd(),
            source.as_ptr(),
            directory.as_raw_fd(),
            destination.as_ptr(),
            libc::RENAME_NOREPLACE,
        )
    };
    if result == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

#[cfg(target_os = "linux")]
fn openat_new_file(directory: &File, name: &str) -> std::io::Result<File> {
    use std::ffi::CString;
    use std::os::fd::{AsRawFd, FromRawFd};

    let name = CString::new(name)
        .map_err(|_| std::io::Error::new(ErrorKind::InvalidInput, "NUL in temporary name"))?;
    // SAFETY: `name` remains live for the syscall; successful `openat` returns
    // a uniquely-owned descriptor which is immediately transferred to `File`.
    let fd = unsafe {
        libc::openat(
            directory.as_raw_fd(),
            name.as_ptr(),
            libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL | libc::O_CLOEXEC | libc::O_NOFOLLOW,
            0o600,
        )
    };
    if fd < 0 {
        Err(std::io::Error::last_os_error())
    } else {
        // SAFETY: `fd` was just returned by `openat` and has no other owner.
        Ok(unsafe { File::from_raw_fd(fd) })
    }
}

#[cfg(target_os = "linux")]
fn open_receive_directory(path: &Path) -> Result<File, ZmodemError> {
    use std::ffi::CString;
    use std::os::fd::FromRawFd;
    use std::os::unix::ffi::OsStrExt;

    let path_bytes = path.as_os_str().as_bytes();
    let path =
        CString::new(path_bytes).map_err(|_| ZmodemError::Io("接收目录路径包含 NUL".into()))?;
    // SAFETY: `path` is a live NUL-terminated pathname. A successful descriptor
    // is transferred exactly once to `File` below.
    let fd = unsafe {
        libc::open(
            path.as_ptr(),
            libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
        )
    };
    if fd < 0 {
        return Err(ZmodemError::Io(format!(
            "无法安全打开 ZMODEM 接收目录: {}",
            std::io::Error::last_os_error()
        )));
    }
    // SAFETY: `fd` was just returned by `open` and has no other owner.
    let directory = unsafe { File::from_raw_fd(fd) };
    if !directory.metadata()?.is_dir() {
        return Err(ZmodemError::Io("ZMODEM 接收目标不是目录".into()));
    }
    Ok(directory)
}

#[cfg(target_os = "linux")]
fn current_directory_path(directory: &File) -> Option<PathBuf> {
    use std::os::fd::AsRawFd;

    let path = std::fs::read_link(format!("/proc/self/fd/{}", directory.as_raw_fd())).ok()?;
    if path.as_os_str().to_string_lossy().ends_with(" (deleted)") {
        None
    } else {
        Some(path)
    }
}

#[cfg(target_os = "linux")]
fn entry_exists_at(directory: &File, name: &str) -> std::io::Result<bool> {
    use std::ffi::CString;
    use std::mem::MaybeUninit;
    use std::os::fd::AsRawFd;

    let name = CString::new(name)
        .map_err(|_| std::io::Error::new(ErrorKind::InvalidInput, "NUL in target name"))?;
    let mut stat = MaybeUninit::<libc::stat>::uninit();
    // SAFETY: `stat` points to writable storage and `name` remains live for
    // the duration of the syscall.
    let result = unsafe {
        libc::fstatat(
            directory.as_raw_fd(),
            name.as_ptr(),
            stat.as_mut_ptr(),
            libc::AT_SYMLINK_NOFOLLOW,
        )
    };
    if result == 0 {
        Ok(true)
    } else {
        let error = std::io::Error::last_os_error();
        if error.kind() == ErrorKind::NotFound {
            Ok(false)
        } else {
            Err(error)
        }
    }
}

#[cfg(target_os = "linux")]
fn unlinkat_file(directory: &File, name: &str) -> std::io::Result<()> {
    use std::ffi::CString;
    use std::os::fd::AsRawFd;

    let name = CString::new(name)
        .map_err(|_| std::io::Error::new(ErrorKind::InvalidInput, "NUL in temporary name"))?;
    // SAFETY: `name` remains live for the syscall and the operation is scoped
    // to the already-open receive directory.
    let result = unsafe { libc::unlinkat(directory.as_raw_fd(), name.as_ptr(), 0) };
    if result == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn metadata(name: &str, size: u64) -> DataSubpacket {
        DataSubpacket {
            data: format!("{name}\0{size} 0 0 0 0 1 0\0").into_bytes(),
            end: DataEnd::EndAck,
        }
    }

    #[test]
    fn rejects_unsafe_names_and_oversized_declarations_without_part_files() {
        let directory = tempfile::tempdir().unwrap();
        for name in [
            "",
            ".",
            "..",
            "../escape",
            "/absolute",
            "a/b",
            "a\\b",
            "a\0b",
            "stream:ads",
            "trailing.",
            "trailing ",
            "CON",
            "nul.txt",
            "COM1.log",
            "lpt9",
        ] {
            assert!(validate_basename(name).is_err(), "{name:?}");
        }
        let mut receiver = ZmodemReceiver::new(directory.path()).unwrap();
        receiver.handle_header(DecodedFrame::new(FrameType::Zfile, [0; 4]));
        let output = receiver.handle_data(metadata("../escape", 1));
        assert!(matches!(
            output.events.as_slice(),
            [ReceiverEvent::Error(ZmodemError::UnsafeFilename(_))]
        ));
        assert!(std::fs::read_dir(directory.path())
            .unwrap()
            .next()
            .is_none());

        receiver.handle_header(DecodedFrame::new(FrameType::Zfile, [0; 4]));
        let output = receiver.handle_data(metadata("huge", MAX_ZMODEM_FILE_SIZE + 1));
        assert!(matches!(
            output.events.as_slice(),
            [ReceiverEvent::Error(ZmodemError::FileTooLarge(_))]
        ));
    }

    #[test]
    fn rejects_windows_invalid_characters_controls_and_superscript_devices() {
        for name in [
            "less<than",
            "greater>than",
            "double\"quote",
            "pipe|name",
            "question?",
            "star*name",
            "control\u{1f}name",
            "COM¹",
            "com².txt",
            "COM³.log",
            "LPT¹",
            "lpt².txt",
            "LPT³.log",
        ] {
            assert!(validate_basename(name).is_err(), "{name:?}");
        }
    }

    #[test]
    fn streams_data_and_atomically_finishes_empty_and_nonempty_files() {
        let directory = tempfile::tempdir().unwrap();
        let mut receiver = ZmodemReceiver::new(directory.path()).unwrap();
        receiver.handle_header(DecodedFrame::new(FrameType::Zfile, [0; 4]));
        receiver.handle_data(metadata("file.bin", 6));
        receiver.handle_header(DecodedFrame::new(FrameType::Zdata, 0u32.to_le_bytes()));
        receiver.handle_data(DataSubpacket {
            data: b"abc".to_vec(),
            end: DataEnd::Continue,
        });
        receiver.handle_data(DataSubpacket {
            data: b"def".to_vec(),
            end: DataEnd::End,
        });
        let output = receiver.handle_header(DecodedFrame::new(FrameType::Zeof, 6u32.to_le_bytes()));
        assert!(matches!(
            output.events.as_slice(),
            [ReceiverEvent::FileComplete { size: 6, .. }]
        ));
        assert_eq!(
            std::fs::read(directory.path().join("file.bin")).unwrap(),
            b"abcdef"
        );

        receiver.handle_header(DecodedFrame::new(FrameType::Zfile, [0; 4]));
        receiver.handle_data(metadata("empty", 0));
        receiver.handle_header(DecodedFrame::new(FrameType::Zdata, [0; 4]));
        receiver.handle_data(DataSubpacket {
            data: Vec::new(),
            end: DataEnd::End,
        });
        receiver.handle_header(DecodedFrame::new(FrameType::Zeof, [0; 4]));
        assert_eq!(
            std::fs::metadata(directory.path().join("empty"))
                .unwrap()
                .len(),
            0
        );
    }

    #[test]
    fn cancellation_and_drop_remove_partial_files() {
        let directory = tempfile::tempdir().unwrap();
        {
            let mut receiver = ZmodemReceiver::new(directory.path()).unwrap();
            receiver.handle_header(DecodedFrame::new(FrameType::Zfile, [0; 4]));
            receiver.handle_data(metadata("partial", 5));
            assert!(receiver.has_partial_file());
            receiver.cancel();
        }
        assert!(std::fs::read_dir(directory.path())
            .unwrap()
            .all(|entry| !entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .ends_with(".part")));
    }

    #[test]
    fn drop_without_explicit_cancel_removes_partial_file() {
        let directory = tempfile::tempdir().unwrap();
        {
            let mut receiver = ZmodemReceiver::new(directory.path()).unwrap();
            receiver.handle_header(DecodedFrame::new(FrameType::Zfile, [0; 4]));
            receiver.handle_data(metadata("drop-partial", 5));
            assert!(receiver.has_partial_file());
            assert!(std::fs::read_dir(directory.path())
                .unwrap()
                .any(|entry| entry
                    .unwrap()
                    .file_name()
                    .to_string_lossy()
                    .ends_with(".part")));
        }
        assert!(std::fs::read_dir(directory.path())
            .unwrap()
            .all(|entry| !entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .ends_with(".part")));
    }

    #[test]
    fn existing_target_is_not_overwritten() {
        let directory = tempfile::tempdir().unwrap();
        std::fs::write(directory.path().join("same"), b"old").unwrap();
        let mut receiver = ZmodemReceiver::new(directory.path()).unwrap();
        receiver.handle_header(DecodedFrame::new(FrameType::Zfile, [0; 4]));
        let output = receiver.handle_data(metadata("same", 3));
        assert!(matches!(
            output.events.as_slice(),
            [ReceiverEvent::Error(ZmodemError::DestinationExists(_))]
        ));
        assert_eq!(
            std::fs::read(directory.path().join("same")).unwrap(),
            b"old"
        );
    }

    #[cfg(unix)]
    #[test]
    fn rejects_symlink_receive_directory() {
        use std::os::unix::fs::symlink;

        let directory = tempfile::tempdir().unwrap();
        let target = directory.path().join("target");
        let link = directory.path().join("link");
        std::fs::create_dir(&target).unwrap();
        symlink(&target, &link).unwrap();
        assert!(ZmodemReceiver::new(&link).is_err());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn held_directory_fd_prevents_path_swap_escape() {
        let root = tempfile::tempdir().unwrap();
        let receive = root.path().join("receive");
        let held = root.path().join("held");
        std::fs::create_dir(&receive).unwrap();
        let mut receiver = ZmodemReceiver::new(&receive).unwrap();
        std::fs::rename(&receive, &held).unwrap();
        std::fs::create_dir(&receive).unwrap();

        receiver.handle_header(DecodedFrame::new(FrameType::Zfile, [0; 4]));
        receiver.handle_data(metadata("safe", 1));
        receiver.handle_header(DecodedFrame::new(FrameType::Zdata, [0; 4]));
        receiver.handle_data(DataSubpacket {
            data: b"x".to_vec(),
            end: DataEnd::End,
        });
        let output = receiver.handle_header(DecodedFrame::new(FrameType::Zeof, 1u32.to_le_bytes()));

        assert_eq!(std::fs::read(held.join("safe")).unwrap(), b"x");
        assert!(!receive.join("safe").exists());
        assert!(matches!(
            output.events.as_slice(),
            [ReceiverEvent::FileComplete { path, size: 1 }] if path == &held.join("safe")
        ));
    }
}
