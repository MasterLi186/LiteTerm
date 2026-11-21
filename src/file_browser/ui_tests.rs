use super::{
    context_action, context_item_fill, context_menu_items, create_action, delete_action,
    delete_dialog_width, file_columns, file_context_menu, file_icon_kind, format_mtime,
    format_size, is_visible_entry, popup_position, rename_action, render, render_delete_dialog,
    reserved_height, should_cancel_rename, visible_entries, ContextCommand, ContextOutcome,
    DeleteDialogState, FileBrowserAction, FileBrowserState, FileEntry, FileIconKind,
    DELETE_DIALOG_HORIZONTAL_CHROME, DELETE_DIALOG_MAX_WIDTH, DELETE_DIALOG_MIN_WIDTH, TEXT,
};
use crate::sftp::CreateKind;

fn entry(name: &str, is_dir: bool) -> FileEntry {
    FileEntry {
        name: name.into(),
        path: format!("/tmp/{name}"),
        is_dir,
        size: 0,
        mtime: 0,
    }
}

fn collect_text(shape: &egui::epaint::Shape, output: &mut Vec<String>) {
    match shape {
        egui::epaint::Shape::Text(text) => output.push(text.galley.job.text.clone()),
        egui::epaint::Shape::Vec(shapes) => {
            for shape in shapes {
                collect_text(shape, output);
            }
        }
        _ => {}
    }
}

fn render_text(mut state: FileBrowserState) -> Vec<String> {
    let ctx = egui::Context::default();
    let mut painted_text = Vec::new();
    for _ in 0..2 {
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1280.0, 800.0),
            )),
            ..Default::default()
        };
        let output = ctx.run(input, |ctx| {
            let _ = render(ctx, &mut state);
        });
        painted_text.clear();
        for clipped in &output.shapes {
            collect_text(&clipped.shape, &mut painted_text);
        }
    }
    painted_text
}

fn rendered_delete_dialog_size(name: &str, path: &str) -> egui::Vec2 {
    let ctx = egui::Context::default();
    let mut state = FileBrowserState::new("/tmp".into());
    state.delete_dialog = Some(DeleteDialogState {
        side: crate::sftp::FileSide::Local,
        name: name.into(),
        path: path.into(),
        is_dir: false,
        just_opened: false,
    });
    let mut size = egui::Vec2::ZERO;
    for _ in 0..3 {
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1280.0, 800.0),
            )),
            ..Default::default()
        };
        let _ = ctx.run(input, |ctx| {
            let mut actions = Vec::new();
            size = render_delete_dialog(ctx, &mut state, &mut actions)
                .expect("delete dialog should render")
                .size();
        });
    }
    size
}

#[test]
fn context_menu_opens_above_when_bottom_space_is_insufficient() {
    let screen = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1280.0, 800.0));
    assert_eq!(
        popup_position(egui::pos2(900.0, 760.0), egui::vec2(160.0, 90.0), screen),
        egui::pos2(900.0, 670.0)
    );
}

#[test]
fn context_menu_is_clamped_to_screen_edges() {
    let screen = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1280.0, 800.0));
    assert_eq!(
        popup_position(egui::pos2(1270.0, 20.0), egui::vec2(160.0, 90.0), screen),
        egui::pos2(1120.0, 20.0)
    );
}

#[test]
fn enabled_context_item_has_visible_hover_fill() {
    assert_eq!(context_item_fill(false, true), egui::Color32::TRANSPARENT);
    assert_ne!(context_item_fill(true, true), egui::Color32::TRANSPARENT);
    assert_eq!(context_item_fill(true, false), egui::Color32::TRANSPARENT);
}

#[test]
fn local_directories_can_upload_but_remote_directories_cannot_download() {
    let local = context_menu_items(
        crate::sftp::FileSide::Local,
        &entry("src", true),
        "/home/local",
        "/srv/remote",
        true,
    );
    let remote = context_menu_items(
        crate::sftp::FileSide::Remote,
        &entry("logs", true),
        "/home/local",
        "/srv/remote",
        true,
    );

    assert_eq!(local[0].label, "上传到远程 (remote)");
    assert!(local[0].enabled);
    assert!(!remote[0].enabled);
    assert!(local
        .iter()
        .any(|item| item.label == "重命名" && item.enabled));
    assert!(local
        .iter()
        .any(|item| item.label == "删除" && item.enabled));
}

#[test]
fn blank_rename_is_rejected() {
    assert!(rename_action(crate::sftp::FileSide::Local, "/tmp/a", "/tmp", "   ").is_none());
}

#[test]
fn rename_action_keeps_the_entry_in_its_parent_directory() {
    assert_eq!(
        rename_action(
            crate::sftp::FileSide::Remote,
            "/srv/old.txt",
            "/srv",
            "new.txt"
        ),
        Some(FileBrowserAction::Rename {
            side: crate::sftp::FileSide::Remote,
            old_path: "/srv/old.txt".into(),
            new_path: "/srv/new.txt".into(),
        })
    );
}

#[test]
fn context_menu_render_shows_main_labels() {
    let mut state = FileBrowserState::new("/home/local".into());
    state.ready = true;
    state.open_context_menu(file_context_menu(
        crate::sftp::FileSide::Remote,
        entry("server.log", false),
        "/srv/remote",
        "/home/local",
        egui::pos2(100.0, 100.0),
    ));
    let painted_text = render_text(state);
    assert!(
        painted_text.iter().any(|text| text == "下载到本地 (local)"),
        "painted text: {painted_text:#?}"
    );
    assert!(painted_text.iter().any(|text| text == "重命名"));
    assert!(painted_text.iter().any(|text| text == "删除"));
}

#[test]
fn delete_context_action_opens_confirmation() {
    let menu = file_context_menu(
        crate::sftp::FileSide::Remote,
        entry("old", true),
        "/srv",
        "/tmp",
        egui::Pos2::ZERO,
    );
    let ContextOutcome::Delete(dialog) = context_action(&menu, ContextCommand::Delete) else {
        panic!("expected delete confirmation");
    };
    assert_eq!(dialog.path, "/tmp/old");
    assert_eq!(dialog.name, "old");
    assert!(dialog.is_dir);
}

#[test]
fn confirmed_delete_targets_the_original_entry() {
    let dialog = DeleteDialogState {
        side: crate::sftp::FileSide::Local,
        name: "cache".into(),
        path: "/tmp/cache".into(),
        is_dir: true,
        just_opened: false,
    };
    assert_eq!(
        delete_action(&dialog),
        FileBrowserAction::Delete {
            side: crate::sftp::FileSide::Local,
            path: "/tmp/cache".into(),
            is_dir: true,
        }
    );
}

#[test]
fn delete_dialog_width_grows_with_name_and_stops_at_maximum() {
    let ctx = egui::Context::default();
    let _ = ctx.run(Default::default(), |ctx| {
        let short = delete_dialog_width(ctx, "a");
        let medium_name = "medium-name-".repeat(4);
        let medium = delete_dialog_width(ctx, &medium_name);
        let long = delete_dialog_width(ctx, &"very-long-name-".repeat(40));
        let medium_message = format!("确定要删除“{medium_name}”吗？");
        let expected_medium = ctx.fonts(|fonts| {
            fonts
                .layout_no_wrap(medium_message, egui::FontId::proportional(12.0), TEXT)
                .size()
                .x
        }) + DELETE_DIALOG_HORIZONTAL_CHROME;

        assert_eq!(short, DELETE_DIALOG_MIN_WIDTH);
        assert!(medium > DELETE_DIALOG_MIN_WIDTH);
        assert!(medium < DELETE_DIALOG_MAX_WIDTH);
        assert!((medium - expected_medium).abs() < 0.001);
        assert_eq!(long, DELETE_DIALOG_MAX_WIDTH);
    });
}

#[test]
fn long_delete_name_wraps_and_makes_dialog_taller() {
    let short = rendered_delete_dialog_size("a", "/tmp/a");
    let long_name = "超长文件名".repeat(80);
    let long_path = format!("/tmp/{long_name}");
    let long = rendered_delete_dialog_size(&long_name, &long_path);
    let ascii_name = "verylongfilename".repeat(80);
    let ascii_path = format!("/tmp/{ascii_name}");
    let ascii = rendered_delete_dialog_size(&ascii_name, &ascii_path);

    assert!(short.y < 138.0, "short dialog height: {}", short.y);
    assert!(
        long.y > short.y,
        "long dialog height: {}, short dialog height: {}",
        long.y,
        short.y
    );
    assert!(
        ascii.y > short.y,
        "ASCII dialog height: {}, short dialog height: {}",
        ascii.y,
        short.y
    );
}

#[test]
fn delete_dialog_outer_width_stays_within_targets_and_ignores_path() {
    let short = rendered_delete_dialog_size("a", "/tmp/a");
    let long_name = "超长文件名".repeat(80);
    let long = rendered_delete_dialog_size(&long_name, &format!("/tmp/{long_name}"));
    let long_path =
        rendered_delete_dialog_size("a", &format!("/tmp/{}", "verylongpath".repeat(100)));

    assert!(
        (short.x - DELETE_DIALOG_MIN_WIDTH).abs() <= 1.0,
        "short dialog width: {}",
        short.x
    );
    assert!(
        long.x <= DELETE_DIALOG_MAX_WIDTH + 1.0,
        "long dialog width: {}",
        long.x
    );
    assert!(
        long.x > short.x,
        "long dialog width: {}, short dialog width: {}",
        long.x,
        short.x
    );
    assert!(
        (long_path.x - short.x).abs() <= 1.0,
        "long-path dialog width: {}, short-path dialog width: {}",
        long_path.x,
        short.x
    );
}

#[test]
fn replacing_long_delete_dialog_with_short_one_resets_height() {
    fn render_frames(
        ctx: &egui::Context,
        state: &mut FileBrowserState,
        frame_count: usize,
    ) -> egui::Vec2 {
        let mut size = egui::Vec2::ZERO;
        for _ in 0..frame_count {
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1280.0, 800.0),
                )),
                ..Default::default()
            };
            let _ = ctx.run(input, |ctx| {
                let mut actions = Vec::new();
                size = render_delete_dialog(ctx, state, &mut actions)
                    .expect("delete dialog should render")
                    .size();
            });
        }
        size
    }

    let ctx = egui::Context::default();
    let mut state = FileBrowserState::new("/tmp".into());
    let long_name = "verylongfilename".repeat(80);
    state.delete_dialog = Some(DeleteDialogState {
        side: crate::sftp::FileSide::Local,
        path: format!("/tmp/{long_name}"),
        name: long_name,
        is_dir: false,
        just_opened: false,
    });
    let long = render_frames(&ctx, &mut state, 4);

    state.delete_dialog = Some(DeleteDialogState {
        side: crate::sftp::FileSide::Local,
        name: "a".into(),
        path: "/tmp/a".into(),
        is_dir: false,
        just_opened: false,
    });
    let short = render_frames(&ctx, &mut state, 4);

    assert!(
        (long.x - DELETE_DIALOG_MAX_WIDTH).abs() <= 1.0,
        "long dialog width: {}",
        long.x
    );
    assert!(short.y < 138.0, "short dialog height: {}", short.y);
    assert!(
        short.y < long.y,
        "short dialog height: {}, long dialog height: {}",
        short.y,
        long.y
    );
    assert!(
        (short.x - DELETE_DIALOG_MIN_WIDTH).abs() <= 1.0,
        "short dialog width: {}",
        short.x
    );
}

#[test]
fn create_action_rejects_invalid_names_and_joins_current_path() {
    for name in ["", " ", ".", "..", "a/b", r"a\b"] {
        assert!(
            create_action(crate::sftp::FileSide::Local, "/tmp", CreateKind::File, name).is_none()
        );
    }
    assert_eq!(
        create_action(
            crate::sftp::FileSide::Remote,
            "/srv",
            CreateKind::Directory,
            "release"
        ),
        Some(FileBrowserAction::Create {
            side: crate::sftp::FileSide::Remote,
            path: "/srv/release".into(),
            kind: CreateKind::Directory,
        })
    );
}

#[test]
fn dual_panes_render_create_buttons() {
    let painted_text = render_text(FileBrowserState::new("/tmp".into()));
    assert_eq!(
        painted_text.iter().filter(|text| *text == "＋文件").count(),
        2
    );
    assert_eq!(
        painted_text.iter().filter(|text| *text == "＋目录").count(),
        2
    );
}

#[test]
fn rename_context_action_opens_prefilled_dialog() {
    let menu = file_context_menu(
        crate::sftp::FileSide::Local,
        entry("old.txt", false),
        "/tmp",
        "/srv",
        egui::Pos2::ZERO,
    );
    let ContextOutcome::Rename(dialog) = context_action(&menu, ContextCommand::Rename) else {
        panic!("expected rename dialog");
    };
    assert_eq!(dialog.value, "old.txt");
    assert_eq!(dialog.old_path, "/tmp/old.txt");
    assert_eq!(dialog.parent_path, "/tmp");
}

#[test]
fn rename_dialog_render_shows_prefilled_name_and_controls() {
    let mut state = FileBrowserState::new("/tmp".into());
    state.rename_dialog = Some(super::RenameDialogState {
        side: crate::sftp::FileSide::Local,
        old_path: "/tmp/old.txt".into(),
        parent_path: "/tmp".into(),
        value: "old.txt".into(),
        request_focus: false,
    });

    let painted_text = render_text(state);
    for expected in ["重命名", "old.txt", "取消", "确定"] {
        assert!(
            painted_text.iter().any(|text| text == expected),
            "missing {expected:?} in painted text: {painted_text:#?}"
        );
    }
}

#[test]
fn menu_click_does_not_immediately_cancel_new_rename_dialog() {
    let window = egui::Rect::from_min_max(egui::pos2(400.0, 300.0), egui::pos2(800.0, 500.0));
    let menu_pointer = Some(egui::pos2(350.0, 680.0));

    assert!(!should_cancel_rename(
        true,
        false,
        true,
        menu_pointer,
        Some(window)
    ));
    assert!(should_cancel_rename(
        false,
        false,
        true,
        menu_pointer,
        Some(window)
    ));
}

#[test]
fn dual_pane_render_has_unique_ids() {
    let painted_text = render_text(FileBrowserState::new("/tmp".into()));
    assert!(
        painted_text
            .iter()
            .all(|text| !text.contains("use of") && !text.contains("ScrollArea ID")),
        "duplicate egui IDs: {painted_text:#?}"
    );
}

#[test]
fn dot_prefixed_entries_are_hidden_by_default() {
    assert!(!is_visible_entry(&entry(".git", true)));
    assert!(!is_visible_entry(&entry(".env", false)));
    assert!(is_visible_entry(&entry("src", true)));
    assert!(is_visible_entry(&entry("README.md", false)));
}

#[test]
fn visible_entry_count_excludes_dot_prefixed_entries() {
    let entries = vec![
        entry(".git", true),
        entry("src", true),
        entry("main.rs", false),
    ];
    assert_eq!(visible_entries(&entries).count(), 2);
}

#[test]
fn directories_and_files_use_distinct_icon_kinds() {
    assert_eq!(file_icon_kind(&entry("src", true)), FileIconKind::Folder);
    assert_eq!(file_icon_kind(&entry("LICENSE", false)), FileIconKind::File);
}

#[test]
fn file_extensions_select_specialized_icon_kinds() {
    assert_eq!(file_icon_kind(&entry("main.RS", false)), FileIconKind::Code);
    assert_eq!(
        file_icon_kind(&entry("config.toml", false)),
        FileIconKind::Text
    );
    assert_eq!(
        file_icon_kind(&entry("photo.webp", false)),
        FileIconKind::Image
    );
    assert_eq!(
        file_icon_kind(&entry("backup.tar.gz", false)),
        FileIconKind::Archive
    );
    assert_eq!(
        file_icon_kind(&entry("kernel.elf", false)),
        FileIconKind::Binary
    );
    assert_eq!(file_icon_kind(&entry("LICENSE", false)), FileIconKind::File);
}

#[test]
fn file_rows_use_main_time_format_and_vector_icons() {
    let mtime = 1_784_135_600;
    let mut state = FileBrowserState::new("/tmp".into());
    state.local.entries.push(FileEntry {
        name: "builds".into(),
        path: "/tmp/builds".into(),
        is_dir: true,
        size: 0,
        mtime,
    });
    let painted_text = render_text(state);
    assert!(painted_text.iter().any(|text| text == &format_mtime(mtime)));
    assert!(!painted_text.iter().any(|text| text == "▸" || text == "·"));
    assert!(!painted_text.iter().any(|text| text.contains("📁")));
    assert!(!painted_text
        .iter()
        .any(|text| text.contains(&mtime.to_string())));
}

#[test]
fn file_columns_fill_available_width() {
    let columns = file_columns(600.0);
    assert_eq!(columns.size, 64.0);
    assert_eq!(columns.mtime, 94.0);
    assert_eq!(columns.name + columns.size + columns.mtime, 600.0);
}

#[test]
fn file_name_column_keeps_a_usable_minimum() {
    assert_eq!(file_columns(180.0).name, 80.0);
}

#[test]
fn zero_mtime_is_blank() {
    assert_eq!(format_mtime(0), "");
}

#[test]
fn mtime_matches_main_display_shape() {
    let value = format_mtime(1_784_135_600);
    assert_eq!(value.len(), 11);
    assert_eq!(&value[2..3], "-");
    assert_eq!(&value[5..6], " ");
    assert_eq!(&value[8..9], ":");
    assert!(value
        .chars()
        .enumerate()
        .all(|(index, ch)| matches!(index, 2 | 5 | 8) || ch.is_ascii_digit()));
}

#[test]
fn size_format_uses_readable_binary_units() {
    assert_eq!(format_size(512), "512 B");
    assert_eq!(format_size(1536), "1.5 KB");
    assert_eq!(format_size(2 * 1024 * 1024), "2.0 MB");
}

#[test]
fn panel_reserves_only_toggle_height_when_collapsed() {
    assert_eq!(reserved_height(false), 22.0);
    assert_eq!(reserved_height(true), 278.0);
}
