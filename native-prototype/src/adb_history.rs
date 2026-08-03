use openssl::sha::sha256;
use serde::{Deserialize, Serialize};
use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
#[cfg(unix)]
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::{mpsc, Mutex, OnceLock};
use std::thread::JoinHandle;
use std::time::Duration;

use crate::smart_completion::{MAX_HISTORY_BYTES, MAX_HISTORY_ITEMS};

const STORE_VERSION: u8 = 1;
const MAX_COMMAND_BYTES: usize = 16 * 1024;
const STORE_DIR: &str = "completion-history/adb";
const MAX_QUEUED_REQUESTS: usize = 256;

static PROCESS_STORE_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

#[derive(Clone, PartialEq, Eq)]
pub enum HostScope {
    Local,
    Ssh {
        user: String,
        host: String,
        port: u16,
    },
}

impl fmt::Debug for HostScope {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Local => formatter.write_str("Local"),
            Self::Ssh { .. } => formatter.write_str("Ssh(<redacted>)"),
        }
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct AdbHistoryIdentity {
    scope: HostScope,
    serial: String,
}

impl AdbHistoryIdentity {
    pub fn new(scope: HostScope, serial: impl Into<String>) -> Option<Self> {
        let serial = serial.into();
        let invalid_scope = match &scope {
            HostScope::Local => false,
            HostScope::Ssh { user, host, port } => {
                *port == 0 || !safe_identity_part(user, 1_024) || !safe_identity_part(host, 1_024)
            }
        };
        if invalid_scope
            || serial.is_empty()
            || serial.len() > 512
            || serial.starts_with('-')
            || serial.chars().any(|character| {
                character.is_control() || character.is_whitespace() || character == '\0'
            })
        {
            return None;
        }
        Some(Self { scope, serial })
    }

    fn digest(&self) -> String {
        let mut framed = Vec::new();
        push_frame(&mut framed, b"liteterm-adb-completion-history-v1");
        match &self.scope {
            HostScope::Local => push_frame(&mut framed, b"local"),
            HostScope::Ssh { user, host, port } => {
                push_frame(&mut framed, b"ssh");
                push_frame(&mut framed, user.as_bytes());
                push_frame(&mut framed, host.to_ascii_lowercase().as_bytes());
                push_frame(&mut framed, &port.to_be_bytes());
            }
        }
        push_frame(&mut framed, self.serial.as_bytes());
        hex::encode(sha256(&framed))
    }
}

fn safe_identity_part(value: &str, max_bytes: usize) -> bool {
    !value.is_empty()
        && value.len() <= max_bytes
        && !value.chars().any(|character| character.is_control())
}

impl fmt::Debug for AdbHistoryIdentity {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AdbHistoryIdentity")
            .field("scope", &self.scope)
            .field("serial", &"<redacted>")
            .finish()
    }
}

#[derive(Serialize, Deserialize)]
struct StoreDocument {
    version: u8,
    entries: Vec<String>,
}

struct AdvisoryLock(File);

impl AdvisoryLock {
    fn exclusive(path: &Path) -> Result<Self, String> {
        if path
            .symlink_metadata()
            .is_ok_and(|metadata| metadata.file_type().is_symlink())
        {
            return Err("ADB 历史锁类型不安全".into());
        }
        let mut options = OpenOptions::new();
        options.read(true).write(true).create(true);
        #[cfg(unix)]
        options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
        let file = options
            .open(path)
            .map_err(|_| "无法打开 ADB 历史锁".to_string())?;
        if !file
            .metadata()
            .is_ok_and(|metadata| metadata.file_type().is_file())
        {
            return Err("ADB 历史锁类型不安全".into());
        }
        set_private_file_permissions(&file)?;
        for _ in 0..100 {
            match fs2::FileExt::try_lock_exclusive(&file) {
                Ok(()) => return Ok(Self(file)),
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(10));
                }
                Err(_) => return Err("无法锁定 ADB 历史".into()),
            }
        }
        Err("ADB 历史正被另一个进程占用".into())
    }
}

impl Drop for AdvisoryLock {
    fn drop(&mut self) {
        let _ = fs2::FileExt::unlock(&self.0);
    }
}

fn push_frame(output: &mut Vec<u8>, value: &[u8]) {
    output.extend_from_slice(&(value.len() as u64).to_be_bytes());
    output.extend_from_slice(value);
}

fn safe_command(command: &str) -> bool {
    !command.is_empty()
        && command.len() <= MAX_COMMAND_BYTES
        && !command.chars().any(char::is_control)
}

fn normalize(entries: impl IntoIterator<Item = String>) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    entries
        .into_iter()
        .filter(|entry| safe_command(entry))
        .filter(|entry| seen.insert(entry.clone()))
        .take(MAX_HISTORY_ITEMS)
        .collect()
}

fn config_store_dir() -> Result<PathBuf, String> {
    let config = dirs::config_dir()
        .filter(|path| path.is_absolute())
        .ok_or_else(|| {
            "系统没有提供可安全使用的配置目录，ADB 补全历史仅保留在内存中".to_string()
        })?;
    let app_root = config.join("guishell");
    ensure_directory_without_following_symlink(&app_root, false)?;
    let history_root = app_root.join("completion-history");
    ensure_private_dir(&history_root)?;
    let store = app_root.join(STORE_DIR);
    ensure_private_dir(&store)?;
    Ok(store)
}

fn ensure_directory_without_following_symlink(
    path: &Path,
    force_private: bool,
) -> Result<(), String> {
    match path.symlink_metadata() {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
            return Err("ADB 历史目录类型不安全".into());
        }
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            let mut builder = fs::DirBuilder::new();
            #[cfg(unix)]
            builder.mode(0o700);
            match builder.create(path) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(_) => return Err("无法创建 ADB 历史目录".into()),
            }
            if !matches!(
                path.symlink_metadata(),
                Ok(metadata) if !metadata.file_type().is_symlink() && metadata.is_dir()
            ) {
                return Err("ADB 历史目录类型不安全".into());
            }
        }
        Err(_) => return Err("无法检查 ADB 历史目录".into()),
    }
    if force_private {
        #[cfg(unix)]
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))
            .map_err(|_| "无法保护 ADB 历史目录".to_string())?;
    }
    Ok(())
}

fn ensure_private_dir(path: &Path) -> Result<(), String> {
    if path
        .symlink_metadata()
        .is_ok_and(|metadata| metadata.file_type().is_symlink() || !metadata.is_dir())
    {
        return Err("ADB 历史目录类型不安全".into());
    }
    ensure_directory_without_following_symlink(path, true)
}

fn prepare_store_dir(base: &Path) -> Result<(), String> {
    ensure_private_dir(base)?;
    if base
        .symlink_metadata()
        .is_ok_and(|metadata| metadata.file_type().is_symlink() || !metadata.is_dir())
    {
        return Err("ADB 历史目录类型不安全".into());
    }
    Ok(())
}

fn set_private_file_permissions(file: &File) -> Result<(), String> {
    #[cfg(unix)]
    {
        file.set_permissions(fs::Permissions::from_mode(0o600))
            .map_err(|_| "无法保护 ADB 历史文件".to_string())?;
    }
    #[cfg(not(unix))]
    let _ = file;
    Ok(())
}

fn history_path(base: &Path, identity: &AdbHistoryIdentity) -> PathBuf {
    base.join(format!("{}.json", identity.digest()))
}

fn read_document(path: &Path) -> Result<Vec<String>, String> {
    if path
        .symlink_metadata()
        .is_ok_and(|metadata| metadata.file_type().is_symlink())
    {
        return Err("ADB 补全历史文件类型不安全".into());
    }
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    let mut file = match options.open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(_) => return Err("无法读取 ADB 补全历史".into()),
    };
    if !file
        .metadata()
        .is_ok_and(|metadata| metadata.file_type().is_file())
    {
        return Err("ADB 补全历史文件类型不安全".into());
    }
    set_private_file_permissions(&file)?;
    let mut bytes = Vec::new();
    Read::by_ref(&mut file)
        .take(MAX_HISTORY_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "无法读取 ADB 补全历史".to_string())?;
    if bytes.len() as u64 > MAX_HISTORY_BYTES {
        return Err("ADB 补全历史文件超过大小限制".into());
    }
    let document: StoreDocument =
        serde_json::from_slice(&bytes).map_err(|_| "ADB 补全历史格式无效".to_string())?;
    if document.version != STORE_VERSION {
        return Err("ADB 补全历史版本不受支持".into());
    }
    Ok(normalize(document.entries))
}

fn serialized_document(mut entries: Vec<String>) -> Result<(Vec<String>, Vec<u8>), String> {
    loop {
        let bytes = serde_json::to_vec(&StoreDocument {
            version: STORE_VERSION,
            entries: entries.clone(),
        })
        .map_err(|_| "无法编码 ADB 补全历史".to_string())?;
        if bytes.len() as u64 <= MAX_HISTORY_BYTES {
            return Ok((entries, bytes));
        }
        if entries.pop().is_none() {
            return Err("ADB 补全历史超过大小限制".into());
        }
    }
}

fn write_document(base: &Path, path: &Path, entries: Vec<String>) -> Result<(), String> {
    let (_, bytes) = serialized_document(entries)?;
    let temp_path = base.join(format!(
        ".{}.{}.tmp",
        path.file_stem()
            .and_then(|value| value.to_str())
            .unwrap_or("history"),
        uuid::Uuid::new_v4().simple()
    ));
    let result = (|| {
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
        let mut file = options
            .open(&temp_path)
            .map_err(|_| "无法创建 ADB 历史临时文件".to_string())?;
        set_private_file_permissions(&file)?;
        file.write_all(&bytes)
            .and_then(|_| file.sync_all())
            .map_err(|_| "无法持久化 ADB 补全历史".to_string())?;
        fs::rename(&temp_path, path).map_err(|_| "无法替换 ADB 补全历史".to_string())?;
        #[cfg(unix)]
        File::open(base)
            .and_then(|directory| directory.sync_all())
            .map_err(|_| "无法同步 ADB 历史目录".to_string())?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temp_path);
    }
    result
}

fn load_from(base: &Path, identity: &AdbHistoryIdentity) -> Result<Vec<String>, String> {
    let _process_guard = PROCESS_STORE_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    prepare_store_dir(base)?;
    let digest = identity.digest();
    let _file_guard = AdvisoryLock::exclusive(&base.join(format!("{digest}.lock")))?;
    read_document(&history_path(base, identity))
}

fn merge_from(base: &Path, identity: &AdbHistoryIdentity, command: &str) -> Result<(), String> {
    if !safe_command(command) {
        return Err("ADB 补全命令不适合持久化".into());
    }
    let _process_guard = PROCESS_STORE_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    prepare_store_dir(base)?;
    let digest = identity.digest();
    let _file_guard = AdvisoryLock::exclusive(&base.join(format!("{digest}.lock")))?;
    let existing = read_document(&history_path(base, identity))?;
    let entries = normalize(std::iter::once(command.to_owned()).chain(existing));
    write_document(base, &history_path(base, identity), entries)
}

pub fn load(identity: &AdbHistoryIdentity) -> Result<Vec<String>, String> {
    load_from(&config_store_dir()?, identity)
}

pub fn merge(identity: &AdbHistoryIdentity, command: &str) -> Result<(), String> {
    merge_from(&config_store_dir()?, identity, command)
}

enum StoreRequest {
    Merge {
        identity: AdbHistoryIdentity,
        command: String,
    },
    Load {
        identity: AdbHistoryIdentity,
        reply: mpsc::Sender<Result<Vec<String>, String>>,
    },
}

pub struct AdbHistoryWriter {
    sender: Option<mpsc::SyncSender<StoreRequest>>,
    thread: Option<JoinHandle<()>>,
}

impl AdbHistoryWriter {
    pub fn start() -> Self {
        let (sender, receiver) = mpsc::sync_channel::<StoreRequest>(MAX_QUEUED_REQUESTS);
        let thread = std::thread::spawn(move || {
            while let Ok(request) = receiver.recv() {
                match request {
                    StoreRequest::Merge { identity, command } => {
                        if merge(&identity, &command).is_err() {
                            log::warn!("ADB 补全历史写入失败");
                        }
                    }
                    StoreRequest::Load { identity, reply } => {
                        let _ = reply.send(load(&identity));
                    }
                }
            }
        });
        Self {
            sender: Some(sender),
            thread: Some(thread),
        }
    }

    pub fn enqueue(&self, identity: AdbHistoryIdentity, command: String) -> Result<(), String> {
        if !safe_command(&command) {
            return Err("ADB 补全命令不适合持久化".into());
        }
        self.sender
            .as_ref()
            .ok_or_else(|| "ADB 历史写入器已关闭".to_string())?
            .try_send(StoreRequest::Merge { identity, command })
            .map_err(|_| "ADB 历史写入器不可用".to_string())
    }

    pub fn enqueue_load(
        &self,
        identity: AdbHistoryIdentity,
    ) -> Result<mpsc::Receiver<Result<Vec<String>, String>>, String> {
        let (reply, receiver) = mpsc::channel();
        self.sender
            .as_ref()
            .ok_or_else(|| "ADB 历史写入器已关闭".to_string())?
            .try_send(StoreRequest::Load { identity, reply })
            .map_err(|_| "ADB 历史写入器不可用".to_string())?;
        Ok(receiver)
    }

    pub fn shutdown(&mut self) {
        self.sender.take();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

impl Drop for AdbHistoryWriter {
    fn drop(&mut self) {
        self.shutdown();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn local(serial: &str) -> AdbHistoryIdentity {
        AdbHistoryIdentity::new(HostScope::Local, serial).unwrap()
    }

    #[test]
    fn identity_path_and_debug_do_not_disclose_inputs() {
        let identity = AdbHistoryIdentity::new(
            HostScope::Ssh {
                user: "secret-user".into(),
                host: "private.example".into(),
                port: 2222,
            },
            "SECRET-SERIAL",
        )
        .unwrap();
        let base = Path::new("/tmp/safe-base");
        let path = history_path(base, &identity).to_string_lossy().into_owned();
        let debug = format!("{identity:?}");
        for secret in ["secret-user", "private.example", "SECRET-SERIAL"] {
            assert!(!path.contains(secret));
            assert!(!debug.contains(secret));
        }
    }

    #[test]
    fn identities_are_stable_and_scoped() {
        let local_identity = local("SERIAL");
        let same = local("SERIAL");
        let remote = AdbHistoryIdentity::new(
            HostScope::Ssh {
                user: "user".into(),
                host: "host".into(),
                port: 22,
            },
            "SERIAL",
        )
        .unwrap();
        assert_eq!(local_identity.digest(), same.digest());
        assert_ne!(local_identity.digest(), remote.digest());
        assert_ne!(local("OTHER").digest(), local_identity.digest());
    }

    #[test]
    fn invalid_serials_are_rejected() {
        for serial in ["", "-bad", "has space", "has\nnewline"] {
            assert!(AdbHistoryIdentity::new(HostScope::Local, serial).is_none());
        }
        assert!(AdbHistoryIdentity::new(
            HostScope::Ssh {
                user: String::new(),
                host: "host".into(),
                port: 22,
            },
            "SERIAL"
        )
        .is_none());
    }

    #[test]
    fn merge_is_mru_deduplicated_and_private() {
        let directory = tempfile::tempdir().unwrap();
        let identity = local("PRIVATE-SERIAL");
        merge_from(directory.path(), &identity, "getprop").unwrap();
        merge_from(directory.path(), &identity, "logcat -d").unwrap();
        merge_from(directory.path(), &identity, "getprop").unwrap();

        assert_eq!(
            load_from(directory.path(), &identity).unwrap(),
            ["getprop", "logcat -d"]
        );
        let file = history_path(directory.path(), &identity);
        assert_eq!(
            fs::metadata(&file).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert_eq!(
            fs::metadata(directory.path()).unwrap().permissions().mode() & 0o777,
            0o700
        );
        let contents = fs::read_to_string(file).unwrap();
        assert!(!contents.contains("PRIVATE-SERIAL"));
        let lock = directory.path().join(format!("{}.lock", identity.digest()));
        assert_eq!(
            fs::metadata(lock).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert!(fs::read_dir(directory.path()).unwrap().all(|entry| !entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .ends_with(".tmp")));
    }

    #[test]
    fn concurrent_merges_do_not_lose_commands() {
        let directory = tempfile::tempdir().unwrap();
        let identity = local("SERIAL");
        std::thread::scope(|scope| {
            for index in 0..12 {
                let identity = identity.clone();
                let base = directory.path().to_owned();
                scope.spawn(move || {
                    merge_from(&base, &identity, &format!("echo {index}")).unwrap();
                });
            }
        });
        let entries = load_from(directory.path(), &identity).unwrap();
        assert_eq!(entries.len(), 12);
        for index in 0..12 {
            assert!(entries.contains(&format!("echo {index}")));
        }
    }

    #[test]
    fn writer_preserves_submission_order_and_drains_on_shutdown() {
        let directory = tempfile::tempdir().unwrap();
        let identity = local("SERIAL");
        let (sender, receiver) = mpsc::channel::<StoreRequest>();
        let base = directory.path().to_owned();
        let thread = std::thread::spawn(move || {
            while let Ok(request) = receiver.recv() {
                match request {
                    StoreRequest::Merge { identity, command } => {
                        merge_from(&base, &identity, &command).unwrap();
                    }
                    StoreRequest::Load { identity, reply } => {
                        let _ = reply.send(load_from(&base, &identity));
                    }
                }
            }
        });
        for command in ["first", "second", "third"] {
            sender
                .send(StoreRequest::Merge {
                    identity: identity.clone(),
                    command: command.into(),
                })
                .unwrap();
        }
        drop(sender);
        thread.join().unwrap();
        assert_eq!(
            load_from(directory.path(), &identity).unwrap(),
            ["third", "second", "first"]
        );
    }

    #[test]
    fn queued_load_observes_all_prior_merges() {
        let directory = tempfile::tempdir().unwrap();
        let identity = local("SERIAL");
        let (sender, receiver) = mpsc::channel::<StoreRequest>();
        let base = directory.path().to_owned();
        let thread = std::thread::spawn(move || {
            while let Ok(request) = receiver.recv() {
                match request {
                    StoreRequest::Merge { identity, command } => {
                        merge_from(&base, &identity, &command).unwrap();
                    }
                    StoreRequest::Load { identity, reply } => {
                        let _ = reply.send(load_from(&base, &identity));
                    }
                }
            }
        });
        sender
            .send(StoreRequest::Merge {
                identity: identity.clone(),
                command: "just executed".into(),
            })
            .unwrap();
        let (reply, result) = mpsc::channel();
        sender.send(StoreRequest::Load { identity, reply }).unwrap();
        assert_eq!(result.recv().unwrap().unwrap(), ["just executed"]);
        drop(sender);
        thread.join().unwrap();
    }

    #[test]
    fn writer_rejects_unsafe_commands_before_queueing() {
        let mut writer = AdbHistoryWriter::start();
        assert!(writer
            .enqueue(local("SERIAL"), "bad\ncommand".into())
            .is_err());
        writer.shutdown();
    }

    #[test]
    fn corrupt_history_is_not_overwritten_by_a_merge() {
        let directory = tempfile::tempdir().unwrap();
        let identity = local("SERIAL");
        prepare_store_dir(directory.path()).unwrap();
        let path = history_path(directory.path(), &identity);
        let original = b"{not valid json";
        fs::write(&path, original).unwrap();

        assert!(merge_from(directory.path(), &identity, "pwd").is_err());
        assert_eq!(fs::read(path).unwrap(), original);
    }

    #[cfg(unix)]
    #[test]
    fn fifo_history_is_rejected_without_blocking_or_replacement() {
        use std::os::unix::ffi::OsStrExt;
        use std::os::unix::fs::FileTypeExt;

        let directory = tempfile::tempdir().unwrap();
        let identity = local("SERIAL");
        prepare_store_dir(directory.path()).unwrap();
        let path = history_path(directory.path(), &identity);
        let path_bytes = std::ffi::CString::new(path.as_os_str().as_bytes()).unwrap();
        assert_eq!(unsafe { libc::mkfifo(path_bytes.as_ptr(), 0o600) }, 0);

        assert!(merge_from(directory.path(), &identity, "pwd").is_err());
        assert!(fs::symlink_metadata(path).unwrap().file_type().is_fifo());
    }

    #[cfg(unix)]
    #[test]
    fn symlink_store_directory_is_rejected() {
        use std::os::unix::fs::symlink;

        let parent = tempfile::tempdir().unwrap();
        let target = parent.path().join("target");
        fs::create_dir(&target).unwrap();
        let link = parent.path().join("link");
        symlink(&target, &link).unwrap();
        assert!(merge_from(&link, &local("SERIAL"), "pwd").is_err());
    }

    #[cfg(unix)]
    #[test]
    fn concurrent_private_directory_creation_is_idempotent() {
        let parent = tempfile::tempdir().unwrap();
        let directory = parent.path().join("history");
        std::thread::scope(|scope| {
            for _ in 0..8 {
                let directory = directory.clone();
                scope.spawn(move || ensure_private_dir(&directory).unwrap());
            }
        });
        assert_eq!(
            fs::metadata(directory).unwrap().permissions().mode() & 0o777,
            0o700
        );
    }
}
