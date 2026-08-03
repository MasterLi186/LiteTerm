use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::{Duration, Instant};
use winit::event_loop::EventLoopProxy;

use crate::smart_completion::{CompletionSessionKey, HistoryLoadRequest};

pub type SftpWorkerId = u128;

fn new_worker_id() -> SftpWorkerId {
    // UUID v4 includes a fixed, non-zero version nibble, so it cannot produce
    // the sentinel value 0 and does not suffer from counter wraparound reuse.
    uuid::Uuid::new_v4().as_u128()
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileEntry {
    pub name: String,
    pub path: String,
    pub is_dir: bool,
    pub size: u64,
    pub mtime: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileSide {
    Local,
    Remote,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CreateKind {
    File,
    Directory,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TransferDirection {
    Upload,
    Download,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileOperation {
    Create,
    Rename,
    Delete,
}

pub enum SftpCommand {
    ListLocal {
        request_id: u64,
        path: String,
    },
    ListRemote {
        request_id: u64,
        path: String,
    },
    Upload {
        transfer_id: String,
        local_path: String,
        remote_path: String,
    },
    UploadBatch {
        uploads: Vec<SftpUploadRequest>,
    },
    Download {
        transfer_id: String,
        remote_path: String,
        local_path: String,
    },
    Create {
        side: FileSide,
        path: String,
        kind: CreateKind,
    },
    Rename {
        side: FileSide,
        old_path: String,
        new_path: String,
    },
    Delete {
        side: FileSide,
        path: String,
        is_dir: bool,
    },
    ReadCompletionHistory {
        session: CompletionSessionKey,
        request: HistoryLoadRequest,
        path: String,
        max_bytes: u64,
    },
    WriteCompletionCandidate {
        session: CompletionSessionKey,
        request_id: u64,
        path: String,
        bytes: Vec<u8>,
    },
    Reconnect,
    Shutdown,
}

#[derive(Clone)]
pub struct SftpUploadRequest {
    pub transfer_id: String,
    pub local_path: String,
    pub remote_path: String,
}

impl std::fmt::Debug for SftpCommand {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ListLocal { request_id, .. } => formatter
                .debug_struct("ListLocal")
                .field("request_id", request_id)
                .finish_non_exhaustive(),
            Self::ListRemote { request_id, .. } => formatter
                .debug_struct("ListRemote")
                .field("request_id", request_id)
                .finish_non_exhaustive(),
            Self::Upload { .. } => formatter
                .debug_tuple("Upload")
                .field(&"<redacted>")
                .finish(),
            Self::UploadBatch { uploads } => formatter
                .debug_struct("UploadBatch")
                .field("uploads", &uploads.len())
                .finish(),
            Self::Download { .. } => formatter
                .debug_tuple("Download")
                .field(&"<redacted>")
                .finish(),
            Self::Create { side, kind, .. } => formatter
                .debug_struct("Create")
                .field("side", side)
                .field("kind", kind)
                .finish_non_exhaustive(),
            Self::Rename { side, .. } => formatter
                .debug_struct("Rename")
                .field("side", side)
                .finish_non_exhaustive(),
            Self::Delete { side, is_dir, .. } => formatter
                .debug_struct("Delete")
                .field("side", side)
                .field("is_dir", is_dir)
                .finish_non_exhaustive(),
            Self::ReadCompletionHistory {
                session, max_bytes, ..
            } => formatter
                .debug_struct("ReadCompletionHistory")
                .field("session", session)
                .field("max_bytes", max_bytes)
                .finish_non_exhaustive(),
            Self::WriteCompletionCandidate {
                session,
                request_id,
                bytes,
                ..
            } => formatter
                .debug_struct("WriteCompletionCandidate")
                .field("session", session)
                .field("request_id", request_id)
                .field("bytes_len", &bytes.len())
                .finish_non_exhaustive(),
            Self::Reconnect => formatter.write_str("Reconnect"),
            Self::Shutdown => formatter.write_str("Shutdown"),
        }
    }
}

pub enum SftpEvent {
    Ready {
        tab_id: String,
        home: String,
    },
    Listed {
        tab_id: String,
        request_id: u64,
        side: FileSide,
        path: String,
        result: Result<Vec<FileEntry>, String>,
    },
    TransferProgress {
        tab_id: String,
        transfer_id: String,
        direction: TransferDirection,
        transferred: u64,
        total: u64,
    },
    TransferFinished {
        tab_id: String,
        transfer_id: String,
        direction: TransferDirection,
        result: Result<(), String>,
    },
    MutationFinished {
        tab_id: String,
        side: FileSide,
        operation: FileOperation,
        result: Result<(), String>,
    },
    Failed {
        tab_id: String,
        error: String,
    },
    CompletionHistoryRead {
        tab_id: String,
        session: CompletionSessionKey,
        request: HistoryLoadRequest,
        path: String,
        result: Result<Vec<u8>, String>,
    },
    CompletionCandidateWritten {
        tab_id: String,
        session: CompletionSessionKey,
        request_id: u64,
        result: Result<(), String>,
    },
}

impl SftpEvent {
    fn debug_name(&self) -> &'static str {
        match self {
            Self::Ready { .. } => "Ready",
            Self::Listed { .. } => "Listed",
            Self::TransferProgress { .. } => "TransferProgress",
            Self::TransferFinished { .. } => "TransferFinished",
            Self::MutationFinished { .. } => "MutationFinished",
            Self::Failed { .. } => "Failed",
            Self::CompletionHistoryRead { .. } => "CompletionHistoryRead",
            Self::CompletionCandidateWritten { .. } => "CompletionCandidateWritten",
        }
    }

    pub fn completion_session(&self) -> Option<&CompletionSessionKey> {
        match self {
            Self::CompletionHistoryRead { session, .. }
            | Self::CompletionCandidateWritten { session, .. } => Some(session),
            _ => None,
        }
    }

    pub fn tab_id(&self) -> &str {
        match self {
            Self::Ready { tab_id, .. }
            | Self::Listed { tab_id, .. }
            | Self::TransferProgress { tab_id, .. }
            | Self::TransferFinished { tab_id, .. }
            | Self::MutationFinished { tab_id, .. }
            | Self::Failed { tab_id, .. }
            | Self::CompletionHistoryRead { tab_id, .. }
            | Self::CompletionCandidateWritten { tab_id, .. } => tab_id,
        }
    }
}

impl std::fmt::Debug for SftpEvent {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.debug_name())
    }
}

pub struct SftpWorkerEvent {
    pub worker_id: SftpWorkerId,
    pub tab_id: String,
    pub pane_id: String,
    pub session: CompletionSessionKey,
    pub event: SftpEvent,
}

impl std::fmt::Debug for SftpWorkerEvent {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("SftpWorkerEvent")
            .field("tab_id", &self.tab_id)
            .field("pane_id", &self.pane_id)
            .field("session", &self.session)
            .field("event", &self.event.debug_name())
            .finish()
    }
}

pub struct SftpHandle {
    id: SftpWorkerId,
    tab_id: String,
    pane_id: String,
    session: CompletionSessionKey,
    tx: mpsc::Sender<SftpCommand>,
}

impl SftpHandle {
    pub fn id(&self) -> SftpWorkerId {
        self.id
    }

    pub fn tab_id(&self) -> &str {
        &self.tab_id
    }

    pub fn pane_id(&self) -> &str {
        &self.pane_id
    }

    pub fn session(&self) -> &CompletionSessionKey {
        &self.session
    }

    pub fn send(&self, command: SftpCommand) -> Result<(), String> {
        self.tx
            .send(command)
            .map_err(|_| "SFTP worker 已停止".to_string())
    }
}

#[cfg(test)]
pub fn test_handle() -> (SftpHandle, mpsc::Receiver<SftpCommand>) {
    test_handle_for("test-tab", "test-pane", CompletionSessionKey::new(1))
}

#[cfg(test)]
pub fn test_handle_for(
    tab_id: &str,
    pane_id: &str,
    session: CompletionSessionKey,
) -> (SftpHandle, mpsc::Receiver<SftpCommand>) {
    let (tx, rx) = mpsc::channel();
    (
        SftpHandle {
            id: new_worker_id(),
            tab_id: tab_id.into(),
            pane_id: pane_id.into(),
            session,
            tx,
        },
        rx,
    )
}

pub struct ProgressThrottle {
    last_emit: Instant,
}

impl ProgressThrottle {
    pub fn new(now: Instant) -> Self {
        Self { last_emit: now }
    }

    pub fn should_emit(&mut self, now: Instant, transferred: u64, total: u64) -> bool {
        if transferred >= total || now.duration_since(self.last_emit) >= Duration::from_millis(100)
        {
            self.last_emit = now;
            true
        } else {
            false
        }
    }
}

pub fn parent_path(path: &str) -> String {
    Path::new(path)
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("/"))
        .to_string_lossy()
        .into_owned()
}

pub fn join_path(base: &str, name: &str) -> String {
    if base == "/" {
        format!("/{name}")
    } else {
        format!("{}/{}", base.trim_end_matches('/'), name)
    }
}

pub fn expand_local_path(path: &str) -> PathBuf {
    PathBuf::from(shellexpand::tilde(path).into_owned())
}

pub fn list_local_dir(path: &str) -> Result<Vec<FileEntry>, String> {
    let expanded = expand_local_path(path);
    let mut entries = std::fs::read_dir(&expanded)
        .map_err(|e| format!("无法读取本地目录 {}: {e}", expanded.display()))?
        .map(|entry| {
            let entry = entry.map_err(|e| e.to_string())?;
            let metadata = entry.metadata().map_err(|e| e.to_string())?;
            let mtime = metadata
                .modified()
                .ok()
                .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
                .map_or(0, |duration| duration.as_secs());
            Ok(FileEntry {
                name: entry.file_name().to_string_lossy().into_owned(),
                path: entry.path().to_string_lossy().into_owned(),
                is_dir: metadata.is_dir(),
                size: if metadata.is_file() {
                    metadata.len()
                } else {
                    0
                },
                mtime,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    sort_entries(&mut entries);
    Ok(entries)
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct LocalUploadEntry {
    source: PathBuf,
    relative: PathBuf,
    is_dir: bool,
    size: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct LocalUploadPlan {
    entries: Vec<LocalUploadEntry>,
    total_bytes: u64,
}

fn build_local_upload_plan(root: &Path) -> Result<LocalUploadPlan, String> {
    let metadata = std::fs::symlink_metadata(root)
        .map_err(|error| format!("无法读取本地路径 {}: {error}", root.display()))?;
    if metadata.file_type().is_symlink() {
        return Err(format!("不支持上传符号链接: {}", root.display()));
    }
    if !metadata.is_dir() {
        return Err(format!("本地路径不是目录: {}", root.display()));
    }

    let mut plan = LocalUploadPlan {
        entries: Vec::new(),
        total_bytes: 0,
    };
    collect_local_upload_entries(root, root, &mut plan)?;
    Ok(plan)
}

fn collect_local_upload_entries(
    root: &Path,
    directory: &Path,
    plan: &mut LocalUploadPlan,
) -> Result<(), String> {
    let mut entries = std::fs::read_dir(directory)
        .map_err(|error| format!("无法读取本地目录 {}: {error}", directory.display()))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("无法读取本地目录项 {}: {error}", directory.display()))?;
    entries.sort_by_key(std::fs::DirEntry::file_name);

    for entry in entries {
        let source = entry.path();
        let metadata = std::fs::symlink_metadata(&source)
            .map_err(|error| format!("无法读取本地路径 {}: {error}", source.display()))?;
        if metadata.file_type().is_symlink() {
            return Err(format!("不支持上传符号链接: {}", source.display()));
        }
        let relative = source
            .strip_prefix(root)
            .map_err(|error| format!("无法计算本地相对路径 {}: {error}", source.display()))?
            .to_path_buf();
        if metadata.is_dir() {
            plan.entries.push(LocalUploadEntry {
                source: source.clone(),
                relative,
                is_dir: true,
                size: 0,
            });
            collect_local_upload_entries(root, &source, plan)?;
        } else if metadata.is_file() {
            let size = metadata.len();
            plan.total_bytes = plan.total_bytes.saturating_add(size);
            plan.entries.push(LocalUploadEntry {
                source,
                relative,
                is_dir: false,
                size,
            });
        } else {
            return Err(format!("不支持上传特殊文件: {}", source.display()));
        }
    }
    Ok(())
}

fn remote_plan_path(root: &str, relative: &Path) -> Result<String, String> {
    let mut remote = if root.is_empty() {
        "/".to_string()
    } else {
        root.to_string()
    };
    for component in relative.components() {
        match component {
            std::path::Component::Normal(name) => {
                remote = join_path(&remote, &name.to_string_lossy());
            }
            _ => {
                return Err(format!("本地相对路径包含非法分量: {}", relative.display()));
            }
        }
    }
    Ok(remote)
}

pub fn rename_local(old_path: &Path, new_path: &Path) -> Result<(), String> {
    std::fs::rename(old_path, new_path).map_err(|error| {
        format!(
            "无法将 {} 重命名为 {}: {error}",
            old_path.display(),
            new_path.display()
        )
    })
}

fn create_local(path: &Path, kind: CreateKind) -> Result<(), String> {
    match kind {
        CreateKind::File => std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)
            .map(|_| ())
            .map_err(|error| format!("无法创建本地文件 {}: {error}", path.display())),
        CreateKind::Directory => std::fs::create_dir(path)
            .map_err(|error| format!("无法创建本地目录 {}: {error}", path.display())),
    }
}

fn remote_file_create_flags() -> ssh2::OpenFlags {
    ssh2::OpenFlags::WRITE | ssh2::OpenFlags::CREATE | ssh2::OpenFlags::EXCLUSIVE
}

const REMOTE_CANDIDATE_MODE: i32 = 0o600;

fn remote_candidate_create_flags() -> ssh2::OpenFlags {
    ssh2::OpenFlags::WRITE | ssh2::OpenFlags::CREATE | ssh2::OpenFlags::EXCLUSIVE
}

fn history_tail_start(length: u64, max_bytes: u64) -> u64 {
    length.saturating_sub(max_bytes)
}

fn read_bounded_history_tail(
    reader: &mut dyn Read,
    max_bytes: u64,
    start: u64,
) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    reader
        .take(max_bytes)
        .read_to_end(&mut bytes)
        .map_err(|error| error.to_string())?;
    if start > 0 {
        if let Some(newline) = bytes.iter().position(|byte| *byte == b'\n') {
            bytes.drain(..=newline);
        } else {
            bytes.clear();
        }
    }
    Ok(bytes)
}

fn read_remote_history_tail(
    sftp: &ssh2::Sftp,
    path: &Path,
    max_bytes: u64,
) -> Result<Vec<u8>, String> {
    let length = sftp
        .stat(path)
        .map_err(|error| format!("无法读取远端历史属性: {error}"))?
        .size
        .unwrap_or(0);
    let start = history_tail_start(length, max_bytes);
    let mut file = sftp
        .open(path)
        .map_err(|error| format!("无法打开远端历史: {error}"))?;
    file.seek(SeekFrom::Start(start))
        .map_err(|error| error.to_string())?;
    read_bounded_history_tail(&mut file, max_bytes, start)
}

fn candidate_temporary_path(path: &str, request_id: u64) -> Result<String, String> {
    let path = Path::new(path);
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .ok_or_else(|| "候选路径缺少父目录".to_string())?;
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| "候选文件名无效".to_string())?;
    Ok(parent
        .join(format!(".{name}.{request_id}.tmp"))
        .to_string_lossy()
        .into_owned())
}

trait RemoteCandidateOps {
    fn open(&mut self, path: &Path, flags: ssh2::OpenFlags, mode: i32) -> Result<(), String>;
    fn write_all(&mut self, bytes: &[u8]) -> Result<(), String>;
    fn close(&mut self) -> Result<(), String>;
    fn rename(&mut self, from: &Path, to: &Path) -> Result<(), String>;
    fn unlink(&mut self, path: &Path) -> Result<(), String>;
}

struct Ssh2RemoteCandidateOps<'a> {
    sftp: &'a ssh2::Sftp,
    file: Option<ssh2::File>,
}

impl<'a> Ssh2RemoteCandidateOps<'a> {
    fn new(sftp: &'a ssh2::Sftp) -> Self {
        Self { sftp, file: None }
    }
}

impl RemoteCandidateOps for Ssh2RemoteCandidateOps<'_> {
    fn open(&mut self, path: &Path, flags: ssh2::OpenFlags, mode: i32) -> Result<(), String> {
        self.file = Some(
            self.sftp
                .open_mode(path, flags, mode, ssh2::OpenType::File)
                .map_err(|error| error.to_string())?,
        );
        Ok(())
    }

    fn write_all(&mut self, bytes: &[u8]) -> Result<(), String> {
        self.file
            .as_mut()
            .ok_or_else(|| "远端候选临时文件未打开".to_string())?
            .write_all(bytes)
            .map_err(|error| error.to_string())
    }

    fn close(&mut self) -> Result<(), String> {
        self.file
            .take()
            .ok_or_else(|| "远端候选临时文件未打开".to_string())?
            .close()
            .map_err(|error| error.to_string())
    }

    fn rename(&mut self, from: &Path, to: &Path) -> Result<(), String> {
        self.sftp
            .rename(from, to, Some(ssh2::RenameFlags::OVERWRITE))
            .map_err(|error| error.to_string())
    }

    fn unlink(&mut self, path: &Path) -> Result<(), String> {
        drop(self.file.take());
        self.sftp.unlink(path).map_err(|error| error.to_string())
    }
}

fn write_remote_candidate_atomic(
    sftp: &ssh2::Sftp,
    path: &str,
    request_id: u64,
    bytes: &[u8],
) -> Result<(), String> {
    let mut ops = Ssh2RemoteCandidateOps::new(sftp);
    write_remote_candidate_with_ops(&mut ops, path, request_id, bytes)
}

fn write_remote_candidate_with_ops(
    ops: &mut impl RemoteCandidateOps,
    path: &str,
    request_id: u64,
    bytes: &[u8],
) -> Result<(), String> {
    crate::bash_integration::validate_candidate_bytes(bytes)?;
    let temporary = candidate_temporary_path(path, request_id)?;
    let temporary_path = Path::new(&temporary);
    let result = (|| {
        ops.open(
            temporary_path,
            remote_candidate_create_flags(),
            REMOTE_CANDIDATE_MODE,
        )
        .map_err(|error| format!("无法创建远端候选临时文件: {error}"))?;
        ops.write_all(bytes)
            .map_err(|error| format!("无法写入远端候选临时文件: {error}"))?;
        ops.close()
            .map_err(|error| format!("无法关闭远端候选临时文件: {error}"))?;
        ops.rename(temporary_path, Path::new(path))
            .map_err(|error| format!("无法替换远端候选文件: {error}"))
    })();
    if result.is_err() {
        let _ = ops.unlink(temporary_path);
    }
    result
}

fn create_remote(sftp: &ssh2::Sftp, path: &str, kind: CreateKind) -> Result<(), String> {
    match kind {
        CreateKind::File => sftp
            .open_mode(
                Path::new(path),
                remote_file_create_flags(),
                0o644,
                ssh2::OpenType::File,
            )
            .map(|_| ())
            .map_err(|error| format!("无法创建远端文件 {path}: {error}")),
        CreateKind::Directory => sftp
            .mkdir(Path::new(path), 0o755)
            .map_err(|error| format!("无法创建远端目录 {path}: {error}")),
    }
}

pub fn delete_local(path: &Path, is_dir: bool) -> Result<(), String> {
    let result = if is_dir {
        std::fs::remove_dir_all(path)
    } else {
        std::fs::remove_file(path)
    };
    result.map_err(|error| format!("无法删除 {}: {error}", path.display()))
}

pub fn sort_entries(entries: &mut [FileEntry]) {
    entries.sort_by(|left, right| {
        right
            .is_dir
            .cmp(&left.is_dir)
            .then_with(|| left.name.to_lowercase().cmp(&right.name.to_lowercase()))
    });
}

fn connect_sftp(
    params: &crate::ssh::ConnectionParams,
) -> Result<(ssh2::Session, ssh2::Sftp, String), String> {
    let session = crate::ssh::connect_authenticated(params)?;
    let sftp = session
        .sftp()
        .map_err(|e| format!("SFTP 子系统启动失败: {e}"))?;
    let home = sftp
        .realpath(Path::new("."))
        .map_err(|e| format!("无法获取远端主目录: {e}"))?
        .to_string_lossy()
        .into_owned();
    Ok((session, sftp, home))
}

fn list_remote_dir(sftp: &ssh2::Sftp, path: &str) -> Result<Vec<FileEntry>, String> {
    let mut entries = sftp
        .readdir(Path::new(path))
        .map_err(|e| format!("无法读取远端目录 {path}: {e}"))?
        .into_iter()
        .filter_map(|(path, stat)| {
            let name = path.file_name()?.to_string_lossy().into_owned();
            if name == "." || name == ".." {
                return None;
            }
            Some(FileEntry {
                name,
                path: path.to_string_lossy().into_owned(),
                is_dir: stat.is_dir(),
                size: stat.size.unwrap_or(0),
                mtime: stat.mtime.unwrap_or(0),
            })
        })
        .collect::<Vec<_>>();
    sort_entries(&mut entries);
    Ok(entries)
}

mod worker;

pub use worker::start_worker_for_pane;
#[cfg(test)]
use worker::{completion_command_session_is_current, with_current_completion_session};

#[cfg(test)]
#[path = "sftp/tests.rs"]
mod tests;

#[cfg(test)]
#[path = "sftp/worker_tests.rs"]
mod worker_tests;
