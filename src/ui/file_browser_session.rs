use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex};

use gtk::glib;
use gtk::prelude::*;
use gtk::{Box as GtkBox, Label, ListBox, ListBoxRow, Orientation};

use crate::core::sftp::SftpEntry;
use crate::ui::file_browser::FileBrowserPanel;

/// Messages from the SFTP background thread to the UI thread.
enum SftpResult {
    /// A directory listing completed successfully.
    RemoteListing {
        path: String,
        entries: Vec<SftpEntry>,
    },
    /// An error occurred.
    Error(String),
}

/// A live file browser session that connects the `FileBrowserPanel` to an
/// SSH/SFTP session. Handles directory navigation for both local and remote
/// panes.
pub struct FileBrowserSession {
    stop_flag: Arc<AtomicBool>,
}

impl FileBrowserSession {
    /// Start a file browser session.
    ///
    /// Opens a dedicated SSH connection for SFTP, populates the local pane
    /// from the filesystem, and wires double-click navigation for both panes.
    pub fn start(
        panel: &FileBrowserPanel,
        host: &str,
        port: u16,
        user: &str,
        password: Option<&str>,
        key_path: Option<&str>,
    ) -> Self {
        let stop_flag = Arc::new(AtomicBool::new(false));

        // --- Populate local pane ---
        let home = dirs::home_dir()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_else(|| "/tmp".to_string());
        panel.local_path_entry().set_text(&home);
        populate_local_list(panel.local_list(), &home);

        // Wire local list double-click for directory navigation
        {
            let local_list = panel.local_list().clone();
            let path_entry = panel.local_path_entry().clone();
            local_list.connect_row_activated(move |_listbox, row| {
                let is_dir: bool = unsafe {
                    row.data::<bool>("is-dir")
                        .map(|p| *p.as_ref())
                        .unwrap_or(false)
                };
                if !is_dir {
                    return;
                }
                let name: String = unsafe {
                    row.data::<String>("entry-name")
                        .map(|p| p.as_ref().clone())
                        .unwrap_or_default()
                };
                if name.is_empty() {
                    return;
                }

                let current = path_entry.text().to_string();
                let new_path = if name == ".." {
                    std::path::Path::new(&current)
                        .parent()
                        .map(|p| p.to_string_lossy().to_string())
                        .unwrap_or_else(|| "/".to_string())
                } else {
                    let p = std::path::Path::new(&current).join(&name);
                    p.to_string_lossy().to_string()
                };

                path_entry.set_text(&new_path);
                // Re-use the listbox reference from the row's parent
                if let Some(parent_list) = row.parent().and_then(|p| p.downcast::<ListBox>().ok()) {
                    populate_local_list(&parent_list, &new_path);
                }
            });
        }

        // Wire local path entry activation (Enter key)
        {
            let local_list = panel.local_list().clone();
            let path_entry = panel.local_path_entry().clone();
            path_entry.connect_activate(move |entry| {
                let path = entry.text().to_string();
                populate_local_list(&local_list, &path);
            });
        }

        // --- Set up SFTP background thread ---
        // Channel for sending listing requests to the background thread
        let (request_tx, request_rx) = mpsc::channel::<String>();
        // Channel for receiving results from the background thread
        let (result_tx, result_rx) = mpsc::channel::<SftpResult>();

        let stop = Arc::clone(&stop_flag);
        let addr = format!("{}:{}", host, port);
        let user_str = user.to_string();
        let password_str = password.map(|s| s.to_string());
        let key_path_str = key_path.map(|s| s.to_string());

        std::thread::spawn(move || {
            // Open a dedicated SSH session for SFTP
            let session = match open_sftp_session(
                &addr,
                &user_str,
                password_str.as_deref(),
                key_path_str.as_deref(),
            ) {
                Ok(s) => s,
                Err(e) => {
                    log::error!("SFTP SSH connect failed: {}", e);
                    let _ = result_tx.send(SftpResult::Error(format!("SFTP connect failed: {}", e)));
                    return;
                }
            };

            let sftp = match session.sftp() {
                Ok(s) => s,
                Err(e) => {
                    log::error!("SFTP init failed: {}", e);
                    let _ = result_tx.send(SftpResult::Error(format!("SFTP init failed: {}", e)));
                    return;
                }
            };

            // Initial listing: home directory of the remote user
            let initial_path = format!("/home/{}", user_str);
            let _ = list_and_send(&sftp, &initial_path, &result_tx);

            // Process requests until stopped
            while !stop.load(Ordering::Relaxed) {
                match request_rx.recv_timeout(std::time::Duration::from_millis(200)) {
                    Ok(path) => {
                        let _ = list_and_send(&sftp, &path, &result_tx);
                    }
                    Err(mpsc::RecvTimeoutError::Timeout) => continue,
                    Err(mpsc::RecvTimeoutError::Disconnected) => break,
                }
            }

            log::info!("SFTP thread stopped.");
        });

        // --- Wire remote list navigation ---
        let request_tx_nav = request_tx.clone();
        {
            let remote_list = panel.remote_list().clone();
            let path_entry = panel.remote_path_entry().clone();
            remote_list.connect_row_activated(move |_listbox, row| {
                let is_dir: bool = unsafe {
                    row.data::<bool>("is-dir")
                        .map(|p| *p.as_ref())
                        .unwrap_or(false)
                };
                if !is_dir {
                    return;
                }
                let name: String = unsafe {
                    row.data::<String>("entry-name")
                        .map(|p| p.as_ref().clone())
                        .unwrap_or_default()
                };
                if name.is_empty() {
                    return;
                }

                let current = path_entry.text().to_string();
                let new_path = if name == ".." {
                    let p = std::path::Path::new(&current);
                    p.parent()
                        .map(|par| par.to_string_lossy().to_string())
                        .unwrap_or_else(|| "/".to_string())
                } else if current == "/" {
                    format!("/{}", name)
                } else {
                    format!("{}/{}", current, name)
                };

                path_entry.set_text(&new_path);
                let _ = request_tx_nav.send(new_path);
            });
        }

        // Wire remote path entry activation (Enter key)
        {
            let path_entry = panel.remote_path_entry().clone();
            let request_tx_enter = request_tx.clone();
            path_entry.connect_activate(move |entry| {
                let path = entry.text().to_string();
                let _ = request_tx_enter.send(path);
            });
        }

        // --- Main-thread timer: drain SFTP results and update remote list ---
        let remote_list = panel.remote_list().clone();
        let remote_path_entry = panel.remote_path_entry().clone();
        let stop_ui = Arc::clone(&stop_flag);
        let result_rx = Mutex::new(result_rx);

        glib::timeout_add_local(std::time::Duration::from_millis(200), move || {
            if stop_ui.load(Ordering::Relaxed) {
                return glib::ControlFlow::Break;
            }

            let rx = result_rx.lock().unwrap();
            while let Ok(msg) = rx.try_recv() {
                match msg {
                    SftpResult::RemoteListing { path, entries } => {
                        remote_path_entry.set_text(&path);
                        populate_remote_list(&remote_list, &entries);
                    }
                    SftpResult::Error(e) => {
                        log::error!("SFTP error: {}", e);
                    }
                }
            }

            glib::ControlFlow::Continue
        });

        Self { stop_flag }
    }

    /// Stop the file browser session.
    pub fn stop(&self) {
        self.stop_flag.store(true, Ordering::Relaxed);
    }
}

impl Drop for FileBrowserSession {
    fn drop(&mut self) {
        self.stop();
    }
}

// ---------------------------------------------------------------------------
// Local file listing
// ---------------------------------------------------------------------------

/// Populate a ListBox with entries from a local directory.
fn populate_local_list(list: &ListBox, path: &str) {
    // Clear existing rows
    while let Some(child) = list.first_child() {
        list.remove(&child);
    }

    // Parent directory entry
    add_file_row(list, "..", true, 0, 0);

    let entries = match std::fs::read_dir(path) {
        Ok(e) => e,
        Err(e) => {
            log::warn!("Failed to read local dir {}: {}", path, e);
            return;
        }
    };

    // Collect and sort: directories first, then alphabetical
    let mut items: Vec<(String, bool, u64, u64)> = Vec::new();
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if name.starts_with('.') {
            continue; // Skip hidden files by default
        }
        let meta = entry.metadata().ok();
        let is_dir = meta.as_ref().map(|m| m.is_dir()).unwrap_or(false);
        let size = meta.as_ref().map(|m| m.len()).unwrap_or(0);
        let mtime = meta
            .as_ref()
            .and_then(|m| m.modified().ok())
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_secs())
            .unwrap_or(0);
        items.push((name, is_dir, size, mtime));
    }

    items.sort_by(|a, b| {
        b.1.cmp(&a.1) // directories first
            .then_with(|| a.0.to_lowercase().cmp(&b.0.to_lowercase()))
    });

    for (name, is_dir, size, mtime) in items {
        add_file_row(list, &name, is_dir, size, mtime);
    }
}

// ---------------------------------------------------------------------------
// Remote file listing
// ---------------------------------------------------------------------------

/// Populate a ListBox with remote SFTP entries.
fn populate_remote_list(list: &ListBox, entries: &[SftpEntry]) {
    while let Some(child) = list.first_child() {
        list.remove(&child);
    }

    // Parent directory entry
    add_file_row(list, "..", true, 0, 0);

    // Sort: directories first, then alphabetical
    let mut sorted: Vec<&SftpEntry> = entries.iter().collect();
    sorted.sort_by(|a, b| {
        b.is_dir
            .cmp(&a.is_dir)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });

    for entry in sorted {
        if entry.name.starts_with('.') {
            continue;
        }
        add_file_row(list, &entry.name, entry.is_dir, entry.size, entry.mtime);
    }
}

/// Add a single file/directory row to a ListBox.
fn add_file_row(list: &ListBox, name: &str, is_dir: bool, size: u64, _mtime: u64) {
    let row_box = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(6)
        .margin_start(4)
        .margin_end(4)
        .margin_top(1)
        .margin_bottom(1)
        .build();

    // Icon
    let icon_name = if name == ".." {
        "go-up-symbolic"
    } else if is_dir {
        "folder-symbolic"
    } else {
        "text-x-generic-symbolic"
    };
    let icon = gtk::Image::from_icon_name(icon_name);
    row_box.append(&icon);

    // Name
    let name_label = Label::builder()
        .label(name)
        .halign(gtk::Align::Start)
        .hexpand(true)
        .ellipsize(gtk::pango::EllipsizeMode::End)
        .build();
    row_box.append(&name_label);

    // Size (skip for directories and parent)
    if !is_dir && name != ".." {
        let size_text = human_file_size(size);
        let size_label = Label::builder()
            .label(&size_text)
            .halign(gtk::Align::End)
            .build();
        size_label.add_css_class("dim-label");
        size_label.add_css_class("caption");
        row_box.append(&size_label);
    }

    let row = ListBoxRow::builder().child(&row_box).build();

    // Store metadata on the row for navigation callbacks
    unsafe {
        row.set_data::<String>("entry-name", name.to_string());
        row.set_data::<bool>("is-dir", is_dir);
    }

    list.append(&row);
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Open a dedicated SSH session for SFTP.
fn open_sftp_session(
    addr: &str,
    user: &str,
    password: Option<&str>,
    key_path: Option<&str>,
) -> Result<ssh2::Session, String> {
    use std::net::TcpStream;

    let tcp = TcpStream::connect(addr).map_err(|e| format!("SFTP TCP connect: {}", e))?;
    let mut sess = ssh2::Session::new().map_err(|e| format!("Session create: {}", e))?;
    sess.set_tcp_stream(tcp);
    sess.handshake().map_err(|e| format!("SSH handshake: {}", e))?;

    if let Some(pw) = password {
        sess.userauth_password(user, pw)
            .map_err(|e| format!("Password auth: {}", e))?;
    } else if let Some(kp) = key_path {
        let expanded = shellexpand::tilde(kp).to_string();
        let path = std::path::Path::new(&expanded);
        sess.userauth_pubkey_file(user, None, path, None)
            .map_err(|e| format!("Key auth: {}", e))?;
    } else {
        let mut agent = sess.agent().map_err(|e| format!("SSH agent: {}", e))?;
        agent.connect().map_err(|e| format!("Agent connect: {}", e))?;
        agent.list_identities().map_err(|e| format!("Agent list: {}", e))?;
        let mut authenticated = false;
        for identity in agent.identities().unwrap_or_default() {
            if agent.userauth(user, &identity).is_ok() {
                authenticated = true;
                break;
            }
        }
        if !authenticated {
            return Err("SSH agent authentication failed".to_string());
        }
    }

    if !sess.authenticated() {
        return Err("Authentication failed".to_string());
    }

    Ok(sess)
}

/// List a remote directory via SFTP and send the result.
fn list_and_send(
    sftp: &ssh2::Sftp,
    path: &str,
    tx: &mpsc::Sender<SftpResult>,
) -> Result<(), String> {
    use std::path::Path;

    match sftp.readdir(Path::new(path)) {
        Ok(dir) => {
            let entries: Vec<SftpEntry> = dir
                .into_iter()
                .filter_map(|(pathbuf, stat)| {
                    let name = pathbuf.file_name()?.to_string_lossy().into_owned();
                    Some(SftpEntry {
                        name,
                        is_dir: stat.is_dir(),
                        size: stat.size.unwrap_or(0),
                        mtime: stat.mtime.unwrap_or(0),
                        permissions: stat.perm.unwrap_or(0o644),
                    })
                })
                .collect();

            let _ = tx.send(SftpResult::RemoteListing {
                path: path.to_string(),
                entries,
            });
            Ok(())
        }
        Err(e) => {
            let msg = format!("SFTP readdir({}): {}", path, e);
            log::warn!("{}", msg);
            let _ = tx.send(SftpResult::Error(msg.clone()));
            Err(msg)
        }
    }
}

/// Format a file size as a human-readable string.
fn human_file_size(bytes: u64) -> String {
    if bytes < 1024 {
        format!("{} B", bytes)
    } else if bytes < 1024 * 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else if bytes < 1024 * 1024 * 1024 {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    } else {
        format!("{:.1} GB", bytes as f64 / (1024.0 * 1024.0 * 1024.0))
    }
}
