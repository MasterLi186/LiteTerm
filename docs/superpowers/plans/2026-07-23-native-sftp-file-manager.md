# Native SFTP File Manager Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a bottom, collapsible local/remote SFTP file manager to `native-prototype/` with directory navigation, upload/download, progress, and per-SSH-tab isolation.

**Architecture:** Each SSH tab owns a separate SFTP worker thread and command channel. The worker creates its own authenticated `ssh2::Session`, performs all filesystem/network I/O off the winit thread, and returns tagged events through `EventLoopProxy`; egui renders per-tab reducer state.

**Tech Stack:** Rust 2021, ssh2/libssh2, std threads and mpsc, winit 0.30 user events, egui 0.31, tempfile for unit tests.

---

## Working Tree Safety

The target Rust files already contain user-owned, uncommitted work. During execution, do not stage or commit overlapping source files automatically: each task ends with a diff checkpoint instead. A source commit may be created only after the user reviews the combined diff or explicitly authorizes including the pre-existing changes. The plan document itself can be committed independently because it is a new file.

## File Structure

- Create `native-prototype/src/sftp.rs`: file models, path helpers, authenticated SFTP worker, streaming transfer.
- Create `native-prototype/src/file_browser.rs`: per-tab state reducer, egui bottom panel, UI actions.
- Modify `native-prototype/src/ssh.rs`: reusable connection parameters and authenticated Session creation.
- Modify `native-prototype/src/tab_manager.rs`: retain complete SSH parameters on each SSH tab.
- Modify `native-prototype/src/main.rs`: module registration, worker lifecycle, event routing, action dispatch.
- Modify `native-prototype/Cargo.toml`: add `tempfile` as a development dependency.
- Modify `native-prototype/TODO.md`: mark only the SFTP P0 item complete after verification.

### Task 1: Share SSH Connection Parameters and Authentication

**Files:**
- Modify: `native-prototype/src/ssh.rs:1-118`
- Modify: `native-prototype/src/tab_manager.rs:1-95`
- Modify: `native-prototype/src/main.rs:169-188, 614-633, 876-918`

- [ ] **Step 1: Add a failing connection-parameter preservation test**

Append to `native-prototype/src/ssh.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::ConnectionParams;
    use crate::sidebar::SshConnection;

    #[test]
    fn connection_params_preserve_all_auth_fields() {
        let source = SshConnection {
            label: "生产机".into(),
            host: "server.example.com".into(),
            port: 2222,
            user: "deploy".into(),
            auth: "key".into(),
            key_path: "~/.ssh/id_ed25519".into(),
            password: "passphrase".into(),
            group: "prod".into(),
            group_color: [1, 2, 3],
        };

        let params = ConnectionParams::from(&source);
        assert_eq!(params.host, "server.example.com");
        assert_eq!(params.port, 2222);
        assert_eq!(params.user, "deploy");
        assert_eq!(params.auth, "key");
        assert_eq!(params.key_path, "~/.ssh/id_ed25519");
        assert_eq!(params.password, "passphrase");
    }
}
```

- [ ] **Step 2: Run the test and verify it fails**

Run:

```bash
cargo test --manifest-path native-prototype/Cargo.toml connection_params_preserve_all_auth_fields
```

Expected: compilation fails because `ConnectionParams` does not exist.

- [ ] **Step 3: Extract connection and authentication primitives**

Add near the top of `native-prototype/src/ssh.rs`:

```rust
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConnectionParams {
    pub host: String,
    pub port: u16,
    pub user: String,
    pub auth: String,
    pub key_path: String,
    pub password: String,
}

impl From<&crate::sidebar::SshConnection> for ConnectionParams {
    fn from(conn: &crate::sidebar::SshConnection) -> Self {
        Self {
            host: conn.host.clone(),
            port: conn.port,
            user: conn.user.clone(),
            auth: conn.auth.clone(),
            key_path: conn.key_path.clone(),
            password: conn.password.clone(),
        }
    }
}

pub(crate) fn connect_authenticated(
    params: &ConnectionParams,
) -> Result<ssh2::Session, String> {
    use std::net::ToSocketAddrs;

    let address = format!("{}:{}", params.host, params.port);
    let socket = address
        .to_socket_addrs()
        .map_err(|e| format!("DNS 解析失败: {e} ({address})"))?
        .next()
        .ok_or_else(|| format!("DNS 无结果: {address}"))?;
    let tcp = TcpStream::connect_timeout(&socket, Duration::from_secs(10))
        .map_err(|e| format!("TCP 连接失败: {e}"))?;

    let mut session =
        ssh2::Session::new().map_err(|e| format!("SSH session 创建失败: {e}"))?;
    session.set_tcp_stream(tcp);
    session
        .handshake()
        .map_err(|e| format!("SSH 握手失败: {e}"))?;

    let key_path = (!params.key_path.is_empty()).then_some(params.key_path.as_str());
    let secret = (!params.password.is_empty()).then_some(params.password.as_str());
    let mut authenticated = false;

    if params.auth == "key" || params.auth == "keyring" || params.auth.is_empty() {
        if let Some(path) = key_path {
            let expanded = shellexpand::tilde(path);
            authenticated = session
                .userauth_pubkey_file(
                    &params.user,
                    None,
                    std::path::Path::new(expanded.as_ref()),
                    secret,
                )
                .is_ok();
        }
    }
    if !authenticated {
        authenticated = session.userauth_agent(&params.user).is_ok();
    }
    if !authenticated {
        for path in ["~/.ssh/id_rsa", "~/.ssh/id_ed25519", "~/.ssh/id_ecdsa"] {
            let expanded = shellexpand::tilde(path);
            let expanded = std::path::Path::new(expanded.as_ref());
            if expanded.exists()
                && session
                    .userauth_pubkey_file(&params.user, None, expanded, secret)
                    .is_ok()
            {
                authenticated = true;
                break;
            }
        }
    }
    if !authenticated {
        if let Some(password) = secret {
            authenticated = session
                .userauth_password(&params.user, password)
                .is_ok();
        }
    }
    if !authenticated || !session.authenticated() {
        return Err(format!(
            "SSH 认证失败 (auth={}, user={})",
            params.auth, params.user
        ));
    }
    session.set_keepalive(true, 30);
    Ok(session)
}
```

Change the terminal connector signature and replace its duplicated TCP/handshake/authentication block:

```rust
pub fn connect(
    params: &ConnectionParams,
    cols: u16,
    rows: u16,
) -> Result<SshHandle, String> {
    let mut session = connect_authenticated(params)?;
    let mut channel = session
        .channel_session()
        .map_err(|e| format!("打开 channel 失败: {e}"))?;
    channel
        .request_pty(
            "xterm-256color",
            None,
            Some((cols as u32, rows as u32, 0, 0)),
        )
        .map_err(|e| format!("PTY 请求失败: {e}"))?;
    channel.shell().map_err(|e| format!("Shell 请求失败: {e}"))?;
    let (write_tx, write_rx) = mpsc::channel::<Vec<u8>>();
    let (resize_tx, resize_rx) = mpsc::channel::<(u16, u16)>();
    let (pipe_read, mut pipe_write) =
        os_pipe::pipe().map_err(|e| format!("创建管道失败: {e}"))?;

    std::thread::spawn(move || {
        session.set_blocking(false);
        let mut buffer = [0_u8; 4096];
        loop {
            match channel.read(&mut buffer) {
                Ok(0) if channel.eof() => break,
                Ok(0) => {}
                Ok(count) => {
                    if pipe_write.write_all(&buffer[..count]).is_err() {
                        break;
                    }
                    let _ = pipe_write.flush();
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {}
                Err(_) => break,
            }
            while let Ok(data) = write_rx.try_recv() {
                session.set_blocking(true);
                let _ = channel.write_all(&data);
                let _ = channel.flush();
                session.set_blocking(false);
            }
            while let Ok((new_cols, new_rows)) = resize_rx.try_recv() {
                session.set_blocking(true);
                let _ = channel.request_pty_size(
                    new_cols as u32,
                    new_rows as u32,
                    None,
                    None,
                );
                session.set_blocking(false);
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    });

    Ok(SshHandle {
        reader: Box::new(pipe_read),
        write_tx,
        resize_tx,
    })
}
```

Change `TabType` and `new_ssh_placeholder` in `native-prototype/src/tab_manager.rs`:

```rust
#[derive(Clone, Debug)]
pub enum TabType {
    Local { shell_path: String },
    Ssh {
        label: String,
        params: crate::ssh::ConnectionParams,
    },
}

// Inside new_ssh_placeholder:
tab_type: TabType::Ssh {
    label: conn.label.clone(),
    params: crate::ssh::ConnectionParams::from(conn),
},
```

Update terminal connection startup:

```rust
let params = crate::ssh::ConnectionParams::from(conn);
let result = crate::ssh::connect(&params, cols, rows);
```

Use this shape for duplicate/reconnect actions:

```rust
if let TabType::Ssh { label, params } = &tab.tab_type {
    let conn = sidebar::SshConnection {
        label: label.clone(),
        host: params.host.clone(),
        port: params.port,
        user: params.user.clone(),
        auth: params.auth.clone(),
        key_path: params.key_path.clone(),
        password: params.password.clone(),
        group: String::new(),
        group_color: [0, 0, 0],
    };
    self.new_ssh_tab(&conn);
}
```

Use this shape for keyring persistence and password retry:

```rust
if let TabType::Ssh { label, params } = &tab.tab_type {
    if !params.password.is_empty() {
        let entry =
            crate::keyring::KeyringEntry::new(&params.user, &params.host, params.port);
        let _ = entry.store_password(&params.password);
    }
    self.sidebar.password_prompt = Some(sidebar::SshConnection {
        label: label.clone(),
        host: params.host.clone(),
        port: params.port,
        user: params.user.clone(),
        auth: "password".to_string(),
        key_path: String::new(),
        password: String::new(),
        group: String::new(),
        group_color: [0, 0, 0],
    });
}
```

- [ ] **Step 4: Run the focused and full native tests**

Run:

```bash
cargo test --manifest-path native-prototype/Cargo.toml connection_params_preserve_all_auth_fields
cargo test --manifest-path native-prototype/Cargo.toml
```

Expected: the focused test and full native test suite pass.

- [ ] **Step 5: Record the authentication checkpoint**

```bash
git diff --check -- native-prototype/src/ssh.rs native-prototype/src/tab_manager.rs native-prototype/src/main.rs
git diff --stat -- native-prototype/src/ssh.rs native-prototype/src/tab_manager.rs native-prototype/src/main.rs
```

Expected: no whitespace errors. Do not stage these overlapping files.

### Task 2: Add File Models, Paths, and Local Listing

**Files:**
- Create: `native-prototype/src/sftp.rs`
- Modify: `native-prototype/src/main.rs:11-24`
- Modify: `native-prototype/Cargo.toml:39`

- [ ] **Step 1: Add failing path and local-listing tests**

Create `native-prototype/src/sftp.rs` with the models and tests first:

```rust
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileEntry {
    pub name: String,
    pub path: String,
    pub is_dir: bool,
    pub size: u64,
    pub mtime: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn path_helpers_handle_root_and_nested_paths() {
        assert_eq!(join_path("/", "etc"), "/etc");
        assert_eq!(join_path("/var/log/", "app.log"), "/var/log/app.log");
        assert_eq!(parent_path("/var/log"), "/var");
        assert_eq!(parent_path("/"), "/");
    }

    #[test]
    fn local_listing_puts_directories_before_files() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(temp.path().join("z.txt"), b"123").unwrap();
        std::fs::create_dir(temp.path().join("a-dir")).unwrap();

        let entries = list_local_dir(temp.path().to_str().unwrap()).unwrap();
        assert_eq!(entries[0].name, "a-dir");
        assert!(entries[0].is_dir);
        assert_eq!(entries[1].name, "z.txt");
        assert_eq!(entries[1].size, 3);
    }
}
```

Add to `native-prototype/Cargo.toml`:

```toml
[dev-dependencies]
tempfile = "3"
```

Add `mod sftp;` beside the other module declarations in `main.rs`.

- [ ] **Step 2: Run the focused tests**

Run:

```bash
cargo test --manifest-path native-prototype/Cargo.toml sftp::tests
```

Expected: compilation fails because `join_path`, `parent_path`, and `list_local_dir` are undefined.

- [ ] **Step 3: Implement path helpers and local listing**

Add above the test module in `sftp.rs`:

```rust
use std::path::{Path, PathBuf};

pub fn parent_path(path: &str) -> String {
    let path = Path::new(path);
    path.parent()
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
                size: if metadata.is_file() { metadata.len() } else { 0 },
                mtime,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    sort_entries(&mut entries);
    Ok(entries)
}

pub fn sort_entries(entries: &mut [FileEntry]) {
    entries.sort_by(|left, right| {
        right
            .is_dir
            .cmp(&left.is_dir)
            .then_with(|| left.name.to_lowercase().cmp(&right.name.to_lowercase()))
    });
}
```

- [ ] **Step 4: Run, format, and record the file-domain checkpoint**

Run:

```bash
cargo test --manifest-path native-prototype/Cargo.toml sftp::tests
cargo fmt --manifest-path native-prototype/Cargo.toml -- --check
```

Expected: both SFTP helper tests pass and rustfmt reports no differences.

If rustfmt reports differences, run `cargo fmt --manifest-path native-prototype/Cargo.toml`, then rerun the check.

```bash
git diff --check -- native-prototype/Cargo.toml native-prototype/Cargo.lock native-prototype/src/sftp.rs native-prototype/src/main.rs
```

Expected: the file-domain diff has no whitespace errors. Do not stage the overlapping manifest or `main.rs`.

### Task 3: Implement the Per-Tab SFTP Worker

**Files:**
- Modify: `native-prototype/src/sftp.rs`
- Test: inline tests in `native-prototype/src/sftp.rs`

- [ ] **Step 1: Add a failing progress-throttle test**

Add:

```rust
#[cfg(test)]
mod worker_tests {
    use super::ProgressThrottle;
    use std::time::{Duration, Instant};

    #[test]
    fn progress_throttle_reports_first_interval_and_completion() {
        let start = Instant::now();
        let mut throttle = ProgressThrottle::new(start);
        assert!(!throttle.should_emit(start + Duration::from_millis(50), 50, 100));
        assert!(throttle.should_emit(start + Duration::from_millis(100), 60, 100));
        assert!(throttle.should_emit(start + Duration::from_millis(101), 100, 100));
    }
}
```

- [ ] **Step 2: Run the test and verify it fails**

Run:

```bash
cargo test --manifest-path native-prototype/Cargo.toml progress_throttle_reports
```

Expected: compilation fails because `ProgressThrottle` is undefined.

- [ ] **Step 3: Add worker commands, events, and progress throttling**

Add to `sftp.rs`:

```rust
use std::io::{Read, Write};
use std::sync::mpsc;
use std::time::{Duration, Instant};
use winit::event_loop::EventLoopProxy;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileSide {
    Local,
    Remote,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TransferDirection {
    Upload,
    Download,
}

#[derive(Debug)]
pub enum SftpCommand {
    ListLocal { request_id: u64, path: String },
    ListRemote { request_id: u64, path: String },
    Upload {
        transfer_id: String,
        local_path: String,
        remote_path: String,
    },
    Download {
        transfer_id: String,
        remote_path: String,
        local_path: String,
    },
    Reconnect,
    Shutdown,
}

#[derive(Debug)]
pub enum SftpEvent {
    Ready { tab_id: String, home: String },
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
    Failed { tab_id: String, error: String },
}

pub struct SftpHandle {
    tx: mpsc::Sender<SftpCommand>,
}

impl SftpHandle {
    pub fn send(&self, command: SftpCommand) -> Result<(), String> {
        self.tx
            .send(command)
            .map_err(|_| "SFTP worker 已停止".to_string())
    }
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
```

- [ ] **Step 4: Add authenticated worker startup and remote listing**

Add these functions:

```rust
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

pub fn start_worker(
    tab_id: String,
    params: crate::ssh::ConnectionParams,
    proxy: EventLoopProxy<crate::UserEvent>,
) -> SftpHandle {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let mut connection = connect_sftp(&params);
        match &connection {
            Ok((_, _, home)) => {
                let _ = proxy.send_event(crate::UserEvent::Sftp(SftpEvent::Ready {
                    tab_id: tab_id.clone(),
                    home: home.clone(),
                }));
            }
            Err(error) => {
                let _ = proxy.send_event(crate::UserEvent::Sftp(SftpEvent::Failed {
                    tab_id: tab_id.clone(),
                    error: error.clone(),
                }));
            }
        }

        while let Ok(command) = rx.recv() {
            match command {
                SftpCommand::Shutdown => break,
                SftpCommand::Reconnect => {
                    connection = connect_sftp(&params);
                    match &connection {
                        Ok((_, _, home)) => {
                            let _ = proxy.send_event(crate::UserEvent::Sftp(SftpEvent::Ready {
                                tab_id: tab_id.clone(),
                                home: home.clone(),
                            }));
                        }
                        Err(error) => {
                            let _ = proxy.send_event(crate::UserEvent::Sftp(SftpEvent::Failed {
                                tab_id: tab_id.clone(),
                                error: error.clone(),
                            }));
                        }
                    }
                }
                SftpCommand::ListLocal { request_id, path } => {
                    let result = list_local_dir(&path);
                    let _ = proxy.send_event(crate::UserEvent::Sftp(SftpEvent::Listed {
                        tab_id: tab_id.clone(),
                        request_id,
                        side: FileSide::Local,
                        path,
                        result,
                    }));
                }
                SftpCommand::ListRemote { request_id, path } => {
                    let result = connection
                        .as_ref()
                        .map_err(|error| error.clone())
                        .and_then(|(_, sftp, _)| list_remote_dir(sftp, &path));
                    let _ = proxy.send_event(crate::UserEvent::Sftp(SftpEvent::Listed {
                        tab_id: tab_id.clone(),
                        request_id,
                        side: FileSide::Remote,
                        path,
                        result,
                    }));
                }
                SftpCommand::Upload { transfer_id, local_path, remote_path } => {
                    let result = connection
                        .as_ref()
                        .map_err(|error| error.clone())
                        .and_then(|(_, sftp, _)| {
                            transfer_upload(
                                sftp, &proxy, &tab_id, &transfer_id, &local_path, &remote_path,
                            )
                        });
                    let _ = proxy.send_event(crate::UserEvent::Sftp(
                        SftpEvent::TransferFinished {
                            tab_id: tab_id.clone(),
                            transfer_id,
                            direction: TransferDirection::Upload,
                            result,
                        },
                    ));
                }
                SftpCommand::Download { transfer_id, remote_path, local_path } => {
                    let result = connection
                        .as_ref()
                        .map_err(|error| error.clone())
                        .and_then(|(_, sftp, _)| {
                            transfer_download(
                                sftp, &proxy, &tab_id, &transfer_id, &remote_path, &local_path,
                            )
                        });
                    let _ = proxy.send_event(crate::UserEvent::Sftp(
                        SftpEvent::TransferFinished {
                            tab_id: tab_id.clone(),
                            transfer_id,
                            direction: TransferDirection::Download,
                            result,
                        },
                    ));
                }
            }
        }
    });
    SftpHandle { tx }
}
```

- [ ] **Step 5: Add streaming upload and download**

Add:

```rust
fn emit_progress(
    proxy: &EventLoopProxy<crate::UserEvent>,
    tab_id: &str,
    transfer_id: &str,
    direction: TransferDirection,
    transferred: u64,
    total: u64,
) {
    let _ = proxy.send_event(crate::UserEvent::Sftp(SftpEvent::TransferProgress {
        tab_id: tab_id.to_string(),
        transfer_id: transfer_id.to_string(),
        direction,
        transferred,
        total,
    }));
}

fn transfer_upload(
    sftp: &ssh2::Sftp,
    proxy: &EventLoopProxy<crate::UserEvent>,
    tab_id: &str,
    transfer_id: &str,
    local_path: &str,
    remote_path: &str,
) -> Result<(), String> {
    let mut source = std::fs::File::open(local_path)
        .map_err(|e| format!("无法打开本地文件 {local_path}: {e}"))?;
    let total = source.metadata().map_err(|e| e.to_string())?.len();
    let mut destination = sftp
        .create(Path::new(remote_path))
        .map_err(|e| format!("无法创建远端文件 {remote_path}: {e}"))?;
    copy_with_progress(
        &mut source,
        &mut destination,
        total,
        proxy,
        tab_id,
        transfer_id,
        TransferDirection::Upload,
    )
}

fn transfer_download(
    sftp: &ssh2::Sftp,
    proxy: &EventLoopProxy<crate::UserEvent>,
    tab_id: &str,
    transfer_id: &str,
    remote_path: &str,
    local_path: &str,
) -> Result<(), String> {
    let mut source = sftp
        .open(Path::new(remote_path))
        .map_err(|e| format!("无法打开远端文件 {remote_path}: {e}"))?;
    let total = source.stat().map_err(|e| e.to_string())?.size.unwrap_or(0);
    let mut destination = std::fs::File::create(local_path)
        .map_err(|e| format!("无法创建本地文件 {local_path}: {e}"))?;
    copy_with_progress(
        &mut source,
        &mut destination,
        total,
        proxy,
        tab_id,
        transfer_id,
        TransferDirection::Download,
    )
}

fn copy_with_progress(
    source: &mut dyn Read,
    destination: &mut dyn Write,
    total: u64,
    proxy: &EventLoopProxy<crate::UserEvent>,
    tab_id: &str,
    transfer_id: &str,
    direction: TransferDirection,
) -> Result<(), String> {
    let mut buffer = [0_u8; 64 * 1024];
    let mut transferred = 0_u64;
    let mut throttle = ProgressThrottle::new(Instant::now());
    loop {
        let count = source.read(&mut buffer).map_err(|e| e.to_string())?;
        if count == 0 {
            break;
        }
        destination
            .write_all(&buffer[..count])
            .map_err(|e| e.to_string())?;
        transferred += count as u64;
        let now = Instant::now();
        if throttle.should_emit(now, transferred, total) {
            emit_progress(
                proxy,
                tab_id,
                transfer_id,
                direction,
                transferred,
                total,
            );
        }
    }
    destination.flush().map_err(|e| e.to_string())?;
    emit_progress(
        proxy,
        tab_id,
        transfer_id,
        direction,
        transferred,
        total,
    );
    Ok(())
}
```

- [ ] **Step 6: Run native tests and record the worker checkpoint**

Run:

```bash
cargo test --manifest-path native-prototype/Cargo.toml
```

Expected: all native tests pass.

```bash
git diff --check -- native-prototype/src/sftp.rs
```

Expected: the worker diff has no whitespace errors.

### Task 4: Build Per-Tab File Browser State

**Files:**
- Create: `native-prototype/src/file_browser.rs`
- Modify: `native-prototype/src/main.rs:11-24`

- [ ] **Step 1: Write failing reducer tests**

Create `native-prototype/src/file_browser.rs`:

```rust
use crate::sftp::{FileEntry, FileSide, SftpEvent, TransferDirection};
use std::time::{Duration, Instant};

#[derive(Clone, Debug)]
pub struct PaneState {
    pub path: String,
    pub input: String,
    pub entries: Vec<FileEntry>,
    pub selected: Option<String>,
    pub loading: bool,
    pub error: Option<String>,
    pub request_id: u64,
}

#[derive(Clone, Debug)]
pub struct TransferItem {
    pub id: String,
    pub filename: String,
    pub direction: TransferDirection,
    pub transferred: u64,
    pub total: u64,
    pub error: Option<String>,
    pub finished: bool,
    pub finished_at: Option<Instant>,
}

pub struct FileBrowserState {
    pub open: bool,
    pub ready: bool,
    pub local: PaneState,
    pub remote: PaneState,
    pub transfers: Vec<TransferItem>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(name: &str) -> FileEntry {
        FileEntry {
            name: name.into(),
            path: format!("/{name}"),
            is_dir: false,
            size: 1,
            mtime: 0,
        }
    }

    #[test]
    fn stale_list_result_does_not_replace_newer_request() {
        let mut state = FileBrowserState::new("/tmp".into());
        state.local.request_id = 2;
        state.apply_event(&SftpEvent::Listed {
            tab_id: "tab".into(),
            request_id: 1,
            side: FileSide::Local,
            path: "/old".into(),
            result: Ok(vec![entry("old")]),
        });
        assert!(state.local.entries.is_empty());
        assert_eq!(state.local.path, "/tmp");
    }

    #[test]
    fn ready_event_sets_remote_home_without_touching_local_path() {
        let mut state = FileBrowserState::new("/tmp".into());
        state.apply_event(&SftpEvent::Ready {
            tab_id: "tab".into(),
            home: "/home/deploy".into(),
        });
        assert!(state.ready);
        assert_eq!(state.remote.path, "/home/deploy");
        assert_eq!(state.local.path, "/tmp");
    }

    #[test]
    fn new_transfer_clears_old_failures_and_preserves_filename() {
        let mut state = FileBrowserState::new("/tmp".into());
        state.start_transfer(
            "old".into(),
            "old.bin".into(),
            TransferDirection::Upload,
        );
        state.transfers[0].error = Some("failed".into());
        state.start_transfer(
            "new".into(),
            "release.tar".into(),
            TransferDirection::Download,
        );
        assert_eq!(state.transfers.len(), 1);
        assert_eq!(state.transfers[0].filename, "release.tar");
    }
}
```

Add `mod file_browser;` to `main.rs`.

- [ ] **Step 2: Run the tests and verify they fail**

Run:

```bash
cargo test --manifest-path native-prototype/Cargo.toml file_browser::tests
```

Expected: compilation fails because `FileBrowserState::new` and `apply_event` are absent.

- [ ] **Step 3: Implement the reducer**

Add above the test module:

```rust
impl PaneState {
    fn new(path: String) -> Self {
        Self {
            input: path.clone(),
            path,
            entries: Vec::new(),
            selected: None,
            loading: false,
            error: None,
            request_id: 0,
        }
    }
}

impl FileBrowserState {
    pub fn new(local_path: String) -> Self {
        Self {
            open: true,
            ready: false,
            local: PaneState::new(local_path),
            remote: PaneState::new("/".into()),
            transfers: Vec::new(),
        }
    }

    pub fn next_request(&mut self, side: FileSide, path: String) -> u64 {
        let pane = match side {
            FileSide::Local => &mut self.local,
            FileSide::Remote => &mut self.remote,
        };
        pane.request_id += 1;
        pane.path = path.clone();
        pane.input = path;
        pane.loading = true;
        pane.error = None;
        pane.request_id
    }

    pub fn start_transfer(
        &mut self,
        id: String,
        filename: String,
        direction: TransferDirection,
    ) {
        self.transfers
            .retain(|item| !item.finished && item.error.is_none());
        self.transfers.push(TransferItem {
            id,
            filename,
            direction,
            transferred: 0,
            total: 0,
            error: None,
            finished: false,
            finished_at: None,
        });
    }

    pub fn prune_completed(&mut self, now: Instant) {
        self.transfers.retain(|item| {
            item.error.is_some()
                || item
                    .finished_at
                    .is_none_or(|finished| now.duration_since(finished) < Duration::from_secs(3))
        });
    }

    pub fn apply_event(&mut self, event: &SftpEvent) {
        match event {
            SftpEvent::Ready { home, .. } => {
                self.ready = true;
                self.remote.path = home.clone();
                self.remote.input = home.clone();
                self.remote.error = None;
            }
            SftpEvent::Failed { error, .. } => {
                self.ready = false;
                self.remote.loading = false;
                self.remote.error = Some(error.clone());
            }
            SftpEvent::Listed {
                request_id,
                side,
                path,
                result,
                ..
            } => {
                let pane = match side {
                    FileSide::Local => &mut self.local,
                    FileSide::Remote => &mut self.remote,
                };
                if *request_id != pane.request_id {
                    return;
                }
                pane.loading = false;
                match result {
                    Ok(entries) => {
                        pane.path = path.clone();
                        pane.input = path.clone();
                        pane.entries = entries.clone();
                        pane.selected = None;
                        pane.error = None;
                    }
                    Err(error) => pane.error = Some(error.clone()),
                }
            }
            SftpEvent::TransferProgress {
                transfer_id,
                direction,
                transferred,
                total,
                ..
            } => {
                if let Some(item) = self
                    .transfers
                    .iter_mut()
                    .find(|item| item.id == transfer_id.as_str())
                {
                    item.transferred = *transferred;
                    item.total = *total;
                } else {
                    self.transfers.push(TransferItem {
                        id: transfer_id.clone(),
                        filename: transfer_id.clone(),
                        direction: *direction,
                        transferred: *transferred,
                        total: *total,
                        error: None,
                        finished: false,
                        finished_at: None,
                    });
                }
            }
            SftpEvent::TransferFinished {
                transfer_id,
                result,
                ..
            } => {
                if let Some(item) = self
                    .transfers
                    .iter_mut()
                    .find(|item| item.id == transfer_id.as_str())
                {
                    item.finished = result.is_ok();
                    item.error = result.as_ref().err().cloned();
                    item.finished_at = result.is_ok().then(Instant::now);
                }
            }
        }
    }
}
```

- [ ] **Step 4: Run tests and record the state checkpoint**

Run:

```bash
cargo test --manifest-path native-prototype/Cargo.toml file_browser::tests
```

Expected: both reducer tests pass.

```bash
git diff --check -- native-prototype/src/file_browser.rs native-prototype/src/main.rs
```

Expected: the state diff has no whitespace errors.

### Task 5: Render the Bottom Dual-Pane UI

**Files:**
- Modify: `native-prototype/src/file_browser.rs`

- [ ] **Step 1: Define UI actions and add a failing formatting test**

Add:

```rust
#[derive(Debug, PartialEq, Eq)]
pub enum FileBrowserAction {
    Toggle,
    List { side: FileSide, path: String },
    Upload { local_path: String, remote_path: String },
    Download { remote_path: String, local_path: String },
    Reconnect,
}

#[cfg(test)]
mod ui_tests {
    use super::format_size;

    #[test]
    fn size_format_uses_readable_binary_units() {
        assert_eq!(format_size(512), "512 B");
        assert_eq!(format_size(1536), "1.5 KB");
        assert_eq!(format_size(2 * 1024 * 1024), "2.0 MB");
    }
}
```

- [ ] **Step 2: Run the formatting test**

Run:

```bash
cargo test --manifest-path native-prototype/Cargo.toml size_format_uses_readable
```

Expected: compilation fails because `format_size` is undefined.

- [ ] **Step 3: Implement the collapsible bottom panel**

Add deterministic size formatting:

```rust
pub fn format_size(bytes: u64) -> String {
    match bytes {
        0..=1023 => format!("{bytes} B"),
        1024..=1_048_575 => format!("{:.1} KB", bytes as f64 / 1024.0),
        1_048_576..=1_073_741_823 => format!("{:.1} MB", bytes as f64 / 1_048_576.0),
        _ => format!("{:.1} GB", bytes as f64 / 1_073_741_824.0),
    }
}
```

Then add a `render` function that returns actions instead of performing I/O:

```rust
pub fn render(
    ctx: &egui::Context,
    state: &mut FileBrowserState,
) -> Vec<FileBrowserAction> {
    let mut actions = Vec::new();
    state.prune_completed(Instant::now());
    egui::TopBottomPanel::bottom("file_browser_toggle")
        .exact_height(22.0)
        .show(ctx, |ui| {
            let label = if state.open {
                "▼ 隐藏文件管理器"
            } else {
                "▲ 显示文件管理器"
            };
            if ui
                .add_sized(ui.available_size(), egui::Button::new(label).frame(false))
                .clicked()
            {
                state.open = !state.open;
                actions.push(FileBrowserAction::Toggle);
            }
        });
    if !state.open {
        return actions;
    }

    egui::TopBottomPanel::bottom("file_browser")
        .exact_height(256.0)
        .show(ctx, |ui| {
            let local_destination = state.local.path.clone();
            let remote_destination = state.remote.path.clone();
            ui.columns(2, |columns| {
                render_pane(
                    &mut columns[0],
                    FileSide::Local,
                    &mut state.local,
                    &remote_destination,
                    &mut actions,
                );
                render_pane(
                    &mut columns[1],
                    FileSide::Remote,
                    &mut state.remote,
                    &local_destination,
                    &mut actions,
                );
            });
            for transfer in &state.transfers {
                let percent = if transfer.total == 0 {
                    0.0
                } else {
                    transfer.transferred as f32 / transfer.total as f32
                };
                ui.horizontal(|ui| {
                    ui.label(match transfer.direction {
                        TransferDirection::Upload => "↑",
                        TransferDirection::Download => "↓",
                    });
                    ui.label(&transfer.filename);
                    ui.add(egui::ProgressBar::new(percent).desired_width(180.0));
                    if let Some(error) = &transfer.error {
                        ui.colored_label(egui::Color32::LIGHT_RED, error);
                    }
                });
            }
        });
    actions
}
```

Add the reusable pane:

```rust
fn render_pane(
    ui: &mut egui::Ui,
    side: FileSide,
    pane: &mut PaneState,
    destination_path: &str,
    actions: &mut Vec<FileBrowserAction>,
) {
    ui.horizontal(|ui| {
        ui.strong(match side {
            FileSide::Local => "本地",
            FileSide::Remote => "远端",
        });
        if ui.button("↑").on_hover_text("上级目录").clicked() {
            actions.push(FileBrowserAction::List {
                side,
                path: crate::sftp::parent_path(&pane.path),
            });
        }
        if ui.button("⟳").on_hover_text("刷新").clicked() {
            actions.push(FileBrowserAction::List {
                side,
                path: pane.path.clone(),
            });
        }
        let response = ui.add(
            egui::TextEdit::singleline(&mut pane.input)
                .desired_width(f32::INFINITY),
        );
        if response.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter)) {
            actions.push(FileBrowserAction::List {
                side,
                path: pane.input.clone(),
            });
        }
    });

    if pane.loading {
        ui.spinner();
    }
    if let Some(error) = &pane.error {
        ui.colored_label(egui::Color32::LIGHT_RED, error);
        if side == FileSide::Remote && ui.button("重新连接").clicked() {
            actions.push(FileBrowserAction::Reconnect);
        }
    }

    egui::Grid::new(match side {
        FileSide::Local => "local_files",
        FileSide::Remote => "remote_files",
    })
    .num_columns(3)
    .striped(true)
    .show(ui, |ui| {
        ui.strong("名称");
        ui.strong("大小");
        ui.strong("修改时间");
        ui.end_row();
        for entry in &pane.entries {
            let icon = if entry.is_dir { "📁" } else { "📄" };
            let response = ui.selectable_label(
                pane.selected.as_deref() == Some(entry.name.as_str()),
                format!("{icon} {}", entry.name),
            );
            ui.label(if entry.is_dir {
                String::new()
            } else {
                format_size(entry.size)
            });
            ui.label(entry.mtime.to_string());
            ui.end_row();
            if response.clicked() {
                pane.selected = Some(entry.name.clone());
            }
            if response.double_clicked() {
                if entry.is_dir {
                    actions.push(FileBrowserAction::List {
                        side,
                        path: entry.path.clone(),
                    });
                } else {
                    match side {
                        FileSide::Local => actions.push(FileBrowserAction::Upload {
                            local_path: entry.path.clone(),
                            remote_path: crate::sftp::join_path(
                                destination_path,
                                &entry.name,
                            ),
                        }),
                        FileSide::Remote => actions.push(FileBrowserAction::Download {
                            remote_path: entry.path.clone(),
                            local_path: crate::sftp::join_path(
                                destination_path,
                                &entry.name,
                            ),
                        }),
                    }
                }
            }
        }
    });
}
```

- [ ] **Step 4: Run tests and record the UI checkpoint**

Run:

```bash
cargo test --manifest-path native-prototype/Cargo.toml
```

Expected: all native tests pass.

```bash
git diff --check -- native-prototype/src/file_browser.rs
```

Expected: the UI diff has no whitespace errors.

### Task 6: Wire Workers, Events, and Tab Lifecycle into App

**Files:**
- Modify: `native-prototype/src/main.rs:1-143, 195-500, 564-641, 864-925`
- Modify: `native-prototype/src/tab_manager.rs:105-125`

- [ ] **Step 1: Add app fields and event routing**

Import `HashMap`, register the modules, and extend `UserEvent`:

```rust
use std::collections::HashMap;

enum UserEvent {
    Redraw,
    SshReady {
        tab_id: String,
        result: Result<crate::ssh::SshHandle, String>,
    },
    Sftp(crate::sftp::SftpEvent),
    MonitorUpdate(Box<monitor::MonitorData>),
}
```

Add to `Debug`:

```rust
UserEvent::Sftp(event) => write!(f, "Sftp({event:?})"),
```

Add to `App` and initialize in `App::new`:

```rust
sftp_workers: HashMap<String, sftp::SftpHandle>,
file_browsers: HashMap<String, file_browser::FileBrowserState>,
```

```rust
sftp_workers: HashMap::new(),
file_browsers: HashMap::new(),
```

- [ ] **Step 2: Start SFTP only after terminal SSH succeeds**

In the successful `SshReady` branch, clone the tab parameters before mutably applying the terminal handle:

```rust
let sftp_params = self.tab_manager.tabs.iter().find_map(|tab| {
    if tab.id != tab_id {
        return None;
    }
    match &tab.tab_type {
        TabType::Ssh { params, .. } => Some(params.clone()),
        TabType::Local { .. } => None,
    }
});
let (cols, rows) = self.grid_size();
if let Some(terminal) = self.tab_manager.apply_ssh(&tab_id, handle, cols, rows) {
    self.start_read_loop(terminal);
}
if let Some(params) = sftp_params {
    let local_path = shellexpand::tilde("~/Downloads").into_owned();
    self.file_browsers.insert(
        tab_id.clone(),
        file_browser::FileBrowserState::new(local_path),
    );
    let worker = sftp::start_worker(tab_id.clone(), params, self.proxy.clone());
    self.sftp_workers.insert(tab_id.clone(), worker);
    self.request_listing(&tab_id, sftp::FileSide::Local, None);
}
```

- [ ] **Step 3: Add listing/action helpers**

Add to `impl App`:

```rust
fn request_listing(
    &mut self,
    tab_id: &str,
    side: sftp::FileSide,
    path: Option<String>,
) {
    let Some(state) = self.file_browsers.get_mut(tab_id) else {
        return;
    };
    let path = path.unwrap_or_else(|| match side {
        sftp::FileSide::Local => state.local.path.clone(),
        sftp::FileSide::Remote => state.remote.path.clone(),
    });
    let request_id = state.next_request(side, path.clone());
    let command = match side {
        sftp::FileSide::Local => sftp::SftpCommand::ListLocal { request_id, path },
        sftp::FileSide::Remote => sftp::SftpCommand::ListRemote { request_id, path },
    };
    if let Some(worker) = self.sftp_workers.get(tab_id) {
        if let Err(error) = worker.send(command) {
            let pane = match side {
                sftp::FileSide::Local => &mut state.local,
                sftp::FileSide::Remote => &mut state.remote,
            };
            pane.loading = false;
            pane.error = Some(error);
        }
    }
}

fn handle_file_browser_action(
    &mut self,
    tab_id: &str,
    action: file_browser::FileBrowserAction,
) {
    match action {
        file_browser::FileBrowserAction::Toggle => {}
        file_browser::FileBrowserAction::List { side, path } => {
            self.request_listing(tab_id, side, Some(path));
        }
        file_browser::FileBrowserAction::Reconnect => {
            if let Some(worker) = self.sftp_workers.get(tab_id) {
                let _ = worker.send(sftp::SftpCommand::Reconnect);
            }
        }
        file_browser::FileBrowserAction::Upload {
            local_path,
            remote_path,
        } => {
            let id = uuid::Uuid::new_v4().to_string();
            let filename = std::path::Path::new(&local_path)
                .file_name()
                .map_or_else(|| local_path.clone(), |name| name.to_string_lossy().into_owned());
            if let Some(state) = self.file_browsers.get_mut(tab_id) {
                state.start_transfer(
                    id.clone(),
                    filename,
                    sftp::TransferDirection::Upload,
                );
            }
            if let Some(worker) = self.sftp_workers.get(tab_id) {
                let _ = worker.send(sftp::SftpCommand::Upload {
                    transfer_id: id,
                    local_path,
                    remote_path,
                });
            }
        }
        file_browser::FileBrowserAction::Download {
            remote_path,
            local_path,
        } => {
            let id = uuid::Uuid::new_v4().to_string();
            let filename = std::path::Path::new(&remote_path)
                .file_name()
                .map_or_else(|| remote_path.clone(), |name| name.to_string_lossy().into_owned());
            if let Some(state) = self.file_browsers.get_mut(tab_id) {
                state.start_transfer(
                    id.clone(),
                    filename,
                    sftp::TransferDirection::Download,
                );
            }
            if let Some(worker) = self.sftp_workers.get(tab_id) {
                let _ = worker.send(sftp::SftpCommand::Download {
                    transfer_id: id,
                    remote_path,
                    local_path,
                });
            }
        }
    }
}
```

- [ ] **Step 4: Render only the active SSH tab’s browser**

Inside the egui run closure, after the command bar reserves its bottom area:

```rust
let active_sftp_tab = self.tab_manager.active().and_then(|tab| {
    matches!(&tab.tab_type, TabType::Ssh { .. }).then(|| tab.id.clone())
});
let mut file_actions = Vec::new();
if let Some(tab_id) = active_sftp_tab.as_ref() {
    if let Some(state) = self.file_browsers.get_mut(tab_id) {
        file_actions = file_browser::render(ctx, state);
    }
}
```

After the egui closure releases UI borrows:

```rust
if let Some(tab_id) = active_sftp_tab {
    for action in file_actions {
        self.handle_file_browser_action(&tab_id, action);
    }
}
```

- [ ] **Step 5: Apply SFTP events and refresh completed targets**

Add to `user_event`:

```rust
UserEvent::Sftp(event) => {
    let tab_id = match &event {
        sftp::SftpEvent::Ready { tab_id, .. }
        | sftp::SftpEvent::Listed { tab_id, .. }
        | sftp::SftpEvent::TransferProgress { tab_id, .. }
        | sftp::SftpEvent::TransferFinished { tab_id, .. }
        | sftp::SftpEvent::Failed { tab_id, .. } => tab_id.clone(),
    };
    if !self.file_browsers.contains_key(&tab_id) {
        return;
    }
    let refresh_side = match &event {
        sftp::SftpEvent::Ready { .. } => Some(sftp::FileSide::Remote),
        sftp::SftpEvent::TransferFinished {
            direction: sftp::TransferDirection::Upload,
            result: Ok(()),
            ..
        } => Some(sftp::FileSide::Remote),
        sftp::SftpEvent::TransferFinished {
            direction: sftp::TransferDirection::Download,
            result: Ok(()),
            ..
        } => Some(sftp::FileSide::Local),
        _ => None,
    };
    self.file_browsers.get_mut(&tab_id).unwrap().apply_event(&event);
    if let Some(side) = refresh_side {
        self.request_listing(&tab_id, side, None);
    }
}
```

- [ ] **Step 6: Clean up worker state when tabs close**

Add:

```rust
fn remove_sftp_tab(&mut self, tab_id: &str) {
    if let Some(worker) = self.sftp_workers.remove(tab_id) {
        let _ = worker.send(sftp::SftpCommand::Shutdown);
    }
    self.file_browsers.remove(tab_id);
}

fn close_tab(&mut self, index: usize) {
    let tab_id = self.tab_manager.tabs.get(index).map(|tab| tab.id.clone());
    self.tab_manager.close(index);
    if let Some(tab_id) = tab_id {
        self.remove_sftp_tab(&tab_id);
    }
    if self.tab_manager.is_empty() {
        self.new_local_tab();
    }
}

fn close_other_tabs(&mut self, keep_index: usize) {
    let keep_id = self
        .tab_manager
        .tabs
        .get(keep_index)
        .map(|tab| tab.id.clone());
    let removed_ids: Vec<String> = self
        .tab_manager
        .tabs
        .iter()
        .filter(|tab| Some(&tab.id) != keep_id.as_ref())
        .map(|tab| tab.id.clone())
        .collect();
    self.tab_manager.close_others(keep_index);
    for tab_id in removed_ids {
        self.remove_sftp_tab(&tab_id);
    }
}
```

Replace every direct `self.tab_manager.close(index)` call in `native-prototype/src/main.rs` with `self.close_tab(index)`. Replace the `TabBarAction::CloseOthers(idx)` arm body with `self.close_other_tabs(idx)`.

- [ ] **Step 7: Run the complete native suite and record the integration checkpoint**

Run:

```bash
cargo test --manifest-path native-prototype/Cargo.toml
cargo fmt --manifest-path native-prototype/Cargo.toml -- --check
```

Expected: all tests pass and formatting is clean.

```bash
git diff --check -- native-prototype/src/main.rs native-prototype/src/tab_manager.rs
```

Expected: the integration diff has no whitespace errors.

### Task 7: Verify End-to-End Behavior and Update the Checklist

**Files:**
- Modify: `native-prototype/TODO.md:64`

- [ ] **Step 1: Run automated verification**

Run:

```bash
cargo test --manifest-path native-prototype/Cargo.toml
cargo clippy --manifest-path native-prototype/Cargo.toml --all-targets -- -D warnings
./build.sh
```

Expected:

- All native unit tests pass.
- Native Clippy exits with no warnings.
- The repository-required full Tauri build and existing Rust tests pass.

- [ ] **Step 2: Run the native client**

Run:

```bash
cargo run --manifest-path native-prototype/Cargo.toml
```

Expected: LiteTerm Native opens with existing sidebar, tabs, terminal, and command bar intact.

- [ ] **Step 3: Perform the SFTP acceptance checks**

Using password, key, and agent-backed saved connections:

1. Connect an SSH tab and confirm the remote pane opens at the server home directory.
2. Navigate both panes using double-click, parent, refresh, and direct path entry.
3. Upload a small file and compare local/remote SHA-256 values.
4. Download the file to a different local name and compare SHA-256 values again.
5. Transfer a file larger than 100 MB while typing commands in the terminal; verify progress updates and terminal responsiveness.
6. Open an unauthorized remote path; verify the prior listing remains and a Chinese error appears.
7. Switch between two SSH tabs; verify paths, listings, and progress never cross tabs.
8. Close a tab during transfer; verify no crash or stale UI event appears.

- [ ] **Step 4: Mark the implemented P0 item complete**

Change only this line in `native-prototype/TODO.md`:

```markdown
- [x] **SFTP 文件管理器** — FileZilla 风格双栏（第一阶段：浏览、导航、上传、下载、进度）
```

- [ ] **Step 5: Verify the final diff**

Run:

```bash
git diff --check
git status --short
```

Confirm the diff contains only the planned SFTP files plus pre-existing user changes, with no `.superpowers/` visual-companion files staged.

Do not stage `native-prototype/TODO.md`: it is a pre-existing untracked user file. Report the final source diff and ask for explicit approval before creating any combined implementation commit.
