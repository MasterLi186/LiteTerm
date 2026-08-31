use crate::sftp::{CreateKind, FileEntry, FileSide, SftpEvent, TransferDirection};
use std::time::{Duration, Instant};

pub const TOGGLE_HEIGHT: f32 = 22.0;
pub const PANEL_HEIGHT: f32 = 256.0;
const PANEL_BG: egui::Color32 = egui::Color32::from_rgb(0x0d, 0x11, 0x17);
const HEADER_BG: egui::Color32 = egui::Color32::from_rgb(0x16, 0x1b, 0x22);
const BORDER: egui::Color32 = egui::Color32::from_rgb(0x21, 0x26, 0x2d);
const MUTED: egui::Color32 = egui::Color32::from_rgb(0x8b, 0x94, 0x9e);
const DIM: egui::Color32 = egui::Color32::from_rgb(0x6e, 0x76, 0x81);
const TEXT: egui::Color32 = egui::Color32::from_rgb(0xc9, 0xd1, 0xd9);
const CYAN: egui::Color32 = egui::Color32::from_rgb(0x00, 0xd4, 0xff);
const GREEN: egui::Color32 = egui::Color32::from_rgb(0x3f, 0xb9, 0x50);
const YELLOW: egui::Color32 = egui::Color32::from_rgb(0xd2, 0x99, 0x22);
const RED: egui::Color32 = egui::Color32::from_rgb(0xf8, 0x51, 0x49);
const ROW_HEIGHT: f32 = 20.0;

pub fn reserved_height(open: bool) -> f32 {
    TOGGLE_HEIGHT + if open { PANEL_HEIGHT } else { 0.0 }
}

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
    context_menu: Option<FileContextMenu>,
    rename_dialog: Option<RenameDialogState>,
    create_dialog: Option<CreateDialogState>,
    delete_dialog: Option<DeleteDialogState>,
}

#[derive(Clone, Debug)]
struct FileContextMenu {
    side: FileSide,
    entry: FileEntry,
    parent_path: String,
    destination_path: String,
    pointer: egui::Pos2,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct RenameDialogState {
    side: FileSide,
    old_path: String,
    parent_path: String,
    value: String,
    request_focus: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct CreateDialogState {
    side: FileSide,
    parent_path: String,
    kind: CreateKind,
    value: String,
    request_focus: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct DeleteDialogState {
    side: FileSide,
    name: String,
    path: String,
    is_dir: bool,
    just_opened: bool,
}

#[derive(Debug, PartialEq, Eq)]
pub enum FileBrowserAction {
    Toggle,
    List {
        side: FileSide,
        path: String,
    },
    Upload {
        local_path: String,
        remote_path: String,
    },
    Download {
        remote_path: String,
        local_path: String,
    },
    Rename {
        side: FileSide,
        old_path: String,
        new_path: String,
    },
    Create {
        side: FileSide,
        path: String,
        kind: CreateKind,
    },
    Delete {
        side: FileSide,
        path: String,
        is_dir: bool,
    },
    Reconnect,
}

pub fn format_size(bytes: u64) -> String {
    match bytes {
        0..=1023 => format!("{bytes} B"),
        1024..=1_048_575 => format!("{:.1} KB", bytes as f64 / 1024.0),
        1_048_576..=1_073_741_823 => format!("{:.1} MB", bytes as f64 / 1_048_576.0),
        _ => format!("{:.1} GB", bytes as f64 / 1_073_741_824.0),
    }
}

pub fn format_mtime(epoch: u64) -> String {
    use chrono::{Local, TimeZone};

    if epoch == 0 {
        return String::new();
    }
    Local
        .timestamp_opt(epoch as i64, 0)
        .single()
        .map(|time| time.format("%m-%d %H:%M").to_string())
        .unwrap_or_default()
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct FileColumns {
    name: f32,
    size: f32,
    mtime: f32,
}

fn file_columns(width: f32) -> FileColumns {
    const SIZE: f32 = 64.0;
    const MTIME: f32 = 94.0;
    FileColumns {
        name: (width - SIZE - MTIME).max(80.0),
        size: SIZE,
        mtime: MTIME,
    }
}

fn is_visible_entry(entry: &FileEntry) -> bool {
    !entry.name.starts_with('.')
}

fn visible_entries(entries: &[FileEntry]) -> impl Iterator<Item = &FileEntry> {
    entries.iter().filter(|entry| is_visible_entry(entry))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ContextCommand {
    Transfer,
    Rename,
    Delete,
}

#[derive(Debug, PartialEq, Eq)]
enum ContextOutcome {
    Action(FileBrowserAction),
    Rename(RenameDialogState),
    Delete(DeleteDialogState),
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ContextItemSpec {
    label: String,
    command: ContextCommand,
    enabled: bool,
    separator_before: bool,
}

fn popup_position(pointer: egui::Pos2, size: egui::Vec2, screen: egui::Rect) -> egui::Pos2 {
    let max_x = (screen.right() - size.x).max(screen.left());
    let x = pointer.x.clamp(screen.left(), max_x);
    let desired_y = if pointer.y + size.y > screen.bottom() {
        pointer.y - size.y
    } else {
        pointer.y
    };
    let max_y = (screen.bottom() - size.y).max(screen.top());
    egui::pos2(x, desired_y.clamp(screen.top(), max_y))
}

fn context_item_fill(hovered: bool, enabled: bool) -> egui::Color32 {
    if hovered && enabled {
        egui::Color32::from_rgb(0x30, 0x36, 0x3d)
    } else {
        egui::Color32::TRANSPARENT
    }
}

fn path_name(path: &str) -> String {
    std::path::Path::new(path)
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| path.to_string())
}

fn context_menu_items(
    side: FileSide,
    entry: &FileEntry,
    local_path: &str,
    remote_path: &str,
    remote_ready: bool,
) -> Vec<ContextItemSpec> {
    let transfer_label = match side {
        FileSide::Local => format!("上传到远程 ({})", path_name(remote_path)),
        FileSide::Remote => format!("下载到本地 ({})", path_name(local_path)),
    };
    let transfer_enabled = match side {
        FileSide::Local => remote_ready,
        FileSide::Remote => remote_ready && !entry.is_dir,
    };
    vec![
        ContextItemSpec {
            label: transfer_label,
            command: ContextCommand::Transfer,
            enabled: transfer_enabled,
            separator_before: false,
        },
        ContextItemSpec {
            label: "重命名".into(),
            command: ContextCommand::Rename,
            enabled: true,
            separator_before: true,
        },
        ContextItemSpec {
            label: "删除".into(),
            command: ContextCommand::Delete,
            enabled: true,
            separator_before: false,
        },
    ]
}

fn file_context_menu(
    side: FileSide,
    entry: FileEntry,
    parent_path: &str,
    destination_path: &str,
    pointer: egui::Pos2,
) -> FileContextMenu {
    FileContextMenu {
        side,
        entry,
        parent_path: parent_path.to_string(),
        destination_path: destination_path.to_string(),
        pointer,
    }
}

fn context_action(menu: &FileContextMenu, command: ContextCommand) -> ContextOutcome {
    match command {
        ContextCommand::Transfer => {
            let destination = crate::sftp::join_path(&menu.destination_path, &menu.entry.name);
            ContextOutcome::Action(match menu.side {
                FileSide::Local => FileBrowserAction::Upload {
                    local_path: menu.entry.path.clone(),
                    remote_path: destination,
                },
                FileSide::Remote => FileBrowserAction::Download {
                    remote_path: menu.entry.path.clone(),
                    local_path: destination,
                },
            })
        }
        ContextCommand::Rename => ContextOutcome::Rename(RenameDialogState {
            side: menu.side,
            old_path: menu.entry.path.clone(),
            parent_path: menu.parent_path.clone(),
            value: menu.entry.name.clone(),
            request_focus: true,
        }),
        ContextCommand::Delete => ContextOutcome::Delete(DeleteDialogState {
            side: menu.side,
            name: menu.entry.name.clone(),
            path: menu.entry.path.clone(),
            is_dir: menu.entry.is_dir,
            just_opened: true,
        }),
    }
}

fn delete_action(dialog: &DeleteDialogState) -> FileBrowserAction {
    FileBrowserAction::Delete {
        side: dialog.side,
        path: dialog.path.clone(),
        is_dir: dialog.is_dir,
    }
}

fn rename_action(
    side: FileSide,
    old_path: &str,
    parent_path: &str,
    value: &str,
) -> Option<FileBrowserAction> {
    let name = value.trim();
    if name.is_empty() {
        return None;
    }
    Some(FileBrowserAction::Rename {
        side,
        old_path: old_path.to_string(),
        new_path: crate::sftp::join_path(parent_path, name),
    })
}

fn create_action(
    side: FileSide,
    parent_path: &str,
    kind: CreateKind,
    value: &str,
) -> Option<FileBrowserAction> {
    let name = value.trim();
    if name.is_empty() || matches!(name, "." | "..") || name.contains('/') || name.contains('\\') {
        return None;
    }
    Some(FileBrowserAction::Create {
        side,
        path: crate::sftp::join_path(parent_path, name),
        kind,
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FileIconKind {
    Folder,
    Code,
    Text,
    Image,
    Archive,
    Binary,
    File,
}

fn file_icon_kind(entry: &FileEntry) -> FileIconKind {
    if entry.is_dir {
        return FileIconKind::Folder;
    }
    let extension = std::path::Path::new(&entry.name)
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    match extension.as_str() {
        "rs" | "c" | "cc" | "cpp" | "h" | "hpp" | "py" | "js" | "jsx" | "ts" | "tsx" | "go"
        | "java" | "kt" | "kts" | "lua" | "php" | "rb" | "sh" | "bash" | "zsh" | "fish" => {
            FileIconKind::Code
        }
        "txt" | "md" | "log" | "json" | "toml" | "yaml" | "yml" | "xml" | "ini" | "conf"
        | "cfg" | "env" | "properties" | "csv" => FileIconKind::Text,
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp" | "svg" | "ico" => FileIconKind::Image,
        "zip" | "tar" | "gz" | "tgz" | "bz2" | "xz" | "7z" | "rar" | "zst" => FileIconKind::Archive,
        "bin" | "elf" | "run" | "appimage" | "exe" | "so" | "dll" | "dylib" => FileIconKind::Binary,
        _ => FileIconKind::File,
    }
}

struct PaneOutputs<'a> {
    pending_menu: &'a mut Option<FileContextMenu>,
    pending_create: &'a mut Option<CreateDialogState>,
    actions: &'a mut Vec<FileBrowserAction>,
}

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
            // Every new/restored/reconnected SSH session starts collapsed.
            // The per-tab state remains mutable so ordinary tab switching keeps
            // the user's manual choice for the lifetime of that connection.
            open: false,
            ready: false,
            local: PaneState::new(local_path),
            remote: PaneState::new("/".into()),
            transfers: Vec::new(),
            context_menu: None,
            rename_dialog: None,
            create_dialog: None,
            delete_dialog: None,
        }
    }

    fn open_context_menu(&mut self, menu: FileContextMenu) {
        self.context_menu = Some(menu);
    }

    pub fn next_request(&mut self, side: FileSide, path: String) -> u64 {
        let pane = match side {
            FileSide::Local => &mut self.local,
            FileSide::Remote => &mut self.remote,
        };
        pane.request_id += 1;
        pane.input = path;
        pane.loading = true;
        pane.error = None;
        pane.request_id
    }

    pub fn start_transfer(&mut self, id: String, filename: String, direction: TransferDirection) {
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
                    Err(error) => {
                        pane.input = pane.path.clone();
                        pane.error = Some(error.clone());
                    }
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
            SftpEvent::MutationFinished {
                side,
                operation,
                result,
                ..
            } => {
                let pane = match side {
                    FileSide::Local => &mut self.local,
                    FileSide::Remote => &mut self.remote,
                };
                pane.error = result.as_ref().err().map(|error| {
                    let label = match operation {
                        crate::sftp::FileOperation::Create => "创建",
                        crate::sftp::FileOperation::Rename => "重命名",
                        crate::sftp::FileOperation::Delete => "删除",
                    };
                    format!("{label}失败: {error}")
                });
            }
            SftpEvent::CompletionHistoryRead { .. }
            | SftpEvent::CompletionCandidateWritten { .. } => {}
        }
    }
}

mod dialogs;
mod view;

#[cfg(test)]
use dialogs::{
    delete_dialog_width, render_delete_dialog, should_cancel_rename,
    DELETE_DIALOG_HORIZONTAL_CHROME, DELETE_DIALOG_MAX_WIDTH, DELETE_DIALOG_MIN_WIDTH,
};
pub use view::render;

#[cfg(test)]
#[path = "file_browser/tests.rs"]
mod tests;

#[cfg(test)]
#[path = "file_browser/ui_tests.rs"]
mod ui_tests;
