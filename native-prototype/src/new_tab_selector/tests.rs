use super::{
    draw_serial_section, group_ssh_connections, modal_geometry, modal_root_sense,
    parse_shells_with, resolve_ssh_connection, serial_column_rects, shell_candidates_with_fallback,
    shell_display_name, ssh_connection_key, ssh_endpoint_label, NewTabAction, NewTabSelector,
    SerialScanState, ACCENT_CYAN, ACCENT_GREEN, ACCENT_YELLOW, BACKDROP_ALPHA, MODAL_BACKGROUND,
    MODAL_BORDER, MODAL_BORDER_RADIUS, MODAL_HEADER_HEIGHT, MODAL_MARGIN, MODAL_MAX_WIDTH,
    MODAL_PADDING, SHELL_BACKGROUND,
};
use crate::sidebar::SshConnection;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};

fn executable(path: &Path) -> bool {
    matches!(
        path.to_str(),
        Some("/bin/bash" | "/bin/zsh" | "/usr/bin/fish")
    )
}

fn connection(label: &str, host: &str, user: &str, password: &str) -> SshConnection {
    SshConnection {
        label: label.into(),
        host: host.into(),
        port: 22,
        user: user.into(),
        auth: "key".into(),
        key_path: "~/.ssh/id_ed25519".into(),
        password: password.into(),
        group: "default".into(),
        group_color: [0x58, 0xa6, 0xff],
    }
}

fn text_shape_position(
    shapes: &[egui::epaint::ClippedShape],
    text: &str,
) -> Option<(egui::Pos2, egui::Vec2)> {
    fn find(shape: &egui::Shape, text: &str) -> Option<(egui::Pos2, egui::Vec2)> {
        match shape {
            egui::Shape::Text(shape) if shape.galley.text() == text => {
                Some((shape.pos, shape.galley.size()))
            }
            egui::Shape::Vec(shapes) => shapes.iter().find_map(|shape| find(shape, text)),
            _ => None,
        }
    }

    shapes.iter().find_map(|shape| find(&shape.shape, text))
}

fn clipped_text_shape(
    shapes: &[egui::epaint::ClippedShape],
    text: &str,
) -> Option<(egui::Pos2, egui::Vec2, egui::Rect)> {
    fn find(shape: &egui::Shape, text: &str) -> Option<(egui::Pos2, egui::Vec2)> {
        match shape {
            egui::Shape::Text(shape) if shape.galley.text() == text => {
                Some((shape.pos, shape.galley.size()))
            }
            egui::Shape::Vec(shapes) => shapes.iter().find_map(|shape| find(shape, text)),
            _ => None,
        }
    }

    shapes.iter().find_map(|clipped| {
        find(&clipped.shape, text).map(|(position, size)| (position, size, clipped.clip_rect))
    })
}

fn serial_row_rects(shapes: &[egui::epaint::ClippedShape]) -> Vec<(egui::Rect, egui::Rect)> {
    fn collect(
        shape: &egui::Shape,
        clip_rect: egui::Rect,
        rows: &mut Vec<(egui::Rect, egui::Rect)>,
    ) {
        match shape {
            egui::Shape::Rect(rect)
                if rect.fill == SHELL_BACKGROUND
                    && rect.stroke == egui::Stroke::new(1.0, MODAL_BORDER) =>
            {
                rows.push((rect.rect, clip_rect));
            }
            egui::Shape::Vec(shapes) => {
                for shape in shapes {
                    collect(shape, clip_rect, rows);
                }
            }
            _ => {}
        }
    }

    let mut rows = Vec::new();
    for clipped in shapes {
        collect(&clipped.shape, clipped.clip_rect, &mut rows);
    }
    rows
}

#[test]
fn modal_geometry_preserves_a_safe_margin_in_a_small_viewport() {
    let viewport = egui::Rect::from_min_size(egui::pos2(7.0, 11.0), egui::vec2(40.0, 40.0));

    let geometry = modal_geometry(viewport).expect("有效的小视口应产生布局");

    assert_eq!(geometry.rect.size(), egui::vec2(8.0, 32.0));
    assert_eq!(geometry.rect.center(), viewport.center());
    assert!(viewport.contains_rect(geometry.rect));
}

#[test]
fn modal_geometry_uses_520_pixels_and_80_percent_height_for_normal_viewport() {
    let viewport = egui::Rect::from_min_size(egui::pos2(10.0, 20.0), egui::vec2(800.0, 600.0));

    let geometry = modal_geometry(viewport).expect("正常视口应产生布局");

    assert_eq!(geometry.rect.size(), egui::vec2(520.0, 480.0));
    assert_eq!(geometry.rect.center(), viewport.center());
    assert!(viewport.contains_rect(geometry.rect));
}

#[test]
fn modal_geometry_uses_viewport_minus_32_and_80_percent_height_when_narrow() {
    let viewport = egui::Rect::from_min_size(egui::pos2(3.0, 7.0), egui::vec2(400.0, 300.0));

    let geometry = modal_geometry(viewport).expect("窄而高的视口应产生布局");

    assert_eq!(geometry.rect.size(), egui::vec2(368.0, 240.0));
    assert_eq!(geometry.rect.center(), viewport.center());
    assert!(viewport.contains_rect(geometry.rect));
}

#[test]
fn modal_presentation_matches_the_main_selector() {
    assert_eq!(MODAL_MAX_WIDTH, 520.0);
    assert_eq!(MODAL_MARGIN, 16.0);
    assert_eq!(MODAL_BORDER_RADIUS, 8.0);
    assert_eq!(BACKDROP_ALPHA, 153);
    assert_eq!(MODAL_BACKGROUND, egui::Color32::from_rgb(0x16, 0x1b, 0x22));
    assert_eq!(MODAL_BORDER, egui::Color32::from_rgb(0x30, 0x36, 0x3d));
}

#[test]
fn section_accents_match_the_approved_palette() {
    assert_eq!(ACCENT_CYAN, egui::Color32::from_rgb(0x00, 0xd4, 0xff));
    assert_eq!(ACCENT_GREEN, egui::Color32::from_rgb(0x00, 0xff, 0x9f));
    assert_eq!(ACCENT_YELLOW, egui::Color32::from_rgb(0xf1, 0xfa, 0x8c));
}

#[test]
fn shell_buttons_wrap_as_whole_buttons_without_vertical_text() {
    let viewport = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 600.0));
    let ctx = egui::Context::default();
    let mut selector = NewTabSelector::new();
    selector.visible = true;
    selector.shells = ["sh", "bash", "rbash", "dash", "fish", "tmux"]
        .into_iter()
        .flat_map(|name| {
            [
                PathBuf::from(format!("/bin/{name}")),
                PathBuf::from(format!("/usr/bin/{name}")),
            ]
        })
        .collect();

    for time in [1.0, 2.0] {
        ctx.begin_pass(egui::RawInput {
            screen_rect: Some(viewport),
            time: Some(time),
            ..Default::default()
        });
        assert_eq!(selector.show(&ctx, &[]), NewTabAction::None);
        if time == 1.0 {
            let _ = ctx.end_pass();
        }
    }
    let output = ctx.end_pass();
    let (_, tmux_size) =
        text_shape_position(&output.shapes, "tmux").expect("tmux button label should be rendered");

    assert!(tmux_size.x > 20.0, "tmux must stay on one horizontal line");
    assert!(tmux_size.y < 20.0, "tmux must not wrap into vertical text");
}

#[test]
fn short_content_keeps_the_modal_below_its_80_percent_height_cap() {
    let viewport = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 600.0));
    let ctx = egui::Context::default();
    ctx.begin_pass(egui::RawInput {
        screen_rect: Some(viewport),
        ..Default::default()
    });
    let mut selector = NewTabSelector::new();
    selector.visible = true;

    assert_eq!(selector.show(&ctx, &[]), NewTabAction::None);
    let _ = ctx.end_pass();
    let modal = egui::AreaState::load(&ctx, egui::Id::new("new_tab_selector_window"))
        .expect("selector area should be recorded");
    let max_height = modal_geometry(viewport)
        .expect("valid viewport")
        .rect
        .height();

    assert!(modal.rect().height() < max_height);
    assert_eq!(modal.rect().width(), MODAL_MAX_WIDTH);
}

#[test]
fn header_title_is_left_aligned_at_16_pixels_and_vertically_centered() {
    let viewport = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 600.0));
    let ctx = egui::Context::default();
    ctx.begin_pass(egui::RawInput {
        screen_rect: Some(viewport),
        time: Some(1.0),
        ..Default::default()
    });
    let mut selector = NewTabSelector::new();
    selector.visible = true;

    assert_eq!(selector.show(&ctx, &[]), NewTabAction::None);
    let _ = ctx.end_pass();
    ctx.begin_pass(egui::RawInput {
        screen_rect: Some(viewport),
        time: Some(2.0),
        ..Default::default()
    });
    assert_eq!(selector.show(&ctx, &[]), NewTabAction::None);
    let output = ctx.end_pass();
    let modal = egui::AreaState::load(&ctx, egui::Id::new("new_tab_selector_window"))
        .expect("selector area should be recorded")
        .rect();
    let (title_pos, title_size) = text_shape_position(&output.shapes, "新建标签页")
        .expect("header title should be painted as text");

    assert_eq!(title_pos.x, modal.left() + 1.0 + MODAL_PADDING);
    assert_eq!(
        title_pos.y + title_size.y / 2.0,
        modal.top() + 1.0 + MODAL_HEADER_HEIGHT / 2.0
    );
}

#[test]
fn header_close_button_is_aligned_to_the_modal_right_edge() {
    let viewport = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 600.0));
    let ctx = egui::Context::default();
    ctx.begin_pass(egui::RawInput {
        screen_rect: Some(viewport),
        time: Some(1.0),
        ..Default::default()
    });
    let mut selector = NewTabSelector::new();
    selector.visible = true;

    assert_eq!(selector.show(&ctx, &[]), NewTabAction::None);
    let _ = ctx.end_pass();
    ctx.begin_pass(egui::RawInput {
        screen_rect: Some(viewport),
        time: Some(2.0),
        ..Default::default()
    });
    assert_eq!(selector.show(&ctx, &[]), NewTabAction::None);
    let output = ctx.end_pass();
    let modal = egui::AreaState::load(&ctx, egui::Id::new("new_tab_selector_window"))
        .expect("selector area should be recorded")
        .rect();
    let (close_pos, close_size) =
        text_shape_position(&output.shapes, "×").expect("close button should be rendered");

    assert!(close_pos.x + close_size.x <= modal.right() - 1.0);
    assert!(close_pos.x >= modal.right() - 36.0);
}

#[test]
fn rendered_modal_never_exceeds_its_narrow_viewport_constraint() {
    let viewport = egui::Rect::from_min_size(egui::pos2(7.0, 11.0), egui::vec2(40.0, 200.0));
    let constraint = modal_geometry(viewport)
        .expect("valid narrow viewport")
        .rect;
    let ctx = egui::Context::default();
    ctx.begin_pass(egui::RawInput {
        screen_rect: Some(viewport),
        time: Some(1.0),
        ..Default::default()
    });
    let mut selector = NewTabSelector::new();
    selector.visible = true;

    assert_eq!(selector.show(&ctx, &[]), NewTabAction::None);
    let _ = ctx.end_pass();
    ctx.begin_pass(egui::RawInput {
        screen_rect: Some(viewport),
        time: Some(2.0),
        ..Default::default()
    });
    assert_eq!(selector.show(&ctx, &[]), NewTabAction::None);
    let _ = ctx.end_pass();
    let rendered = egui::AreaState::load(&ctx, egui::Id::new("new_tab_selector_window"))
        .expect("selector area should be recorded")
        .rect();

    assert!(
        rendered.width() <= constraint.width(),
        "rendered={} constrained={}",
        rendered.width(),
        constraint.width()
    );
    assert!(
        viewport.contains_rect(rendered),
        "viewport={viewport:?} rendered={rendered:?}"
    );
}

#[test]
fn long_ssh_label_and_endpoint_use_disjoint_clipped_regions() {
    let viewport = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 600.0));
    let label = "超长连接标签".repeat(20);
    let host = format!("{}.example", "endpoint".repeat(4));
    let endpoint = format!("{host}:22");
    let connections = vec![connection(&label, &host, "root", "")];
    let ctx = egui::Context::default();
    let mut selector = NewTabSelector::new();
    selector.visible = true;

    for time in [1.0, 2.0] {
        ctx.begin_pass(egui::RawInput {
            screen_rect: Some(viewport),
            time: Some(time),
            ..Default::default()
        });
        assert_eq!(selector.show(&ctx, &connections), NewTabAction::None);
        if time == 1.0 {
            let _ = ctx.end_pass();
        }
    }
    let output = ctx.end_pass();
    let modal = egui::AreaState::load(&ctx, egui::Id::new("new_tab_selector_window"))
        .expect("selector area should be recorded")
        .rect();
    let (_, _, label_clip) =
        clipped_text_shape(&output.shapes, &label).expect("long SSH label should be rendered");
    let (_, _, endpoint_clip) = clipped_text_shape(&output.shapes, &endpoint)
        .expect("long SSH endpoint should be rendered");

    assert!(label_clip.right() + 8.0 <= endpoint_clip.left());
    assert!(endpoint_clip.right() <= modal.right() - 1.0 - MODAL_PADDING - 8.0);
}

#[test]
fn long_serial_rows_fill_the_vertical_viewport_without_horizontal_overflow() {
    let viewport = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(520.0, 240.0));
    let ports = (0..40)
        .map(|index| crate::serial::SerialPortInfo {
            name: format!("串口设备-{index}-{}", "很长的名称".repeat(8)),
            path: format!("/dev/serial/by-id/device-{index}-{}", "x".repeat(48)),
            port_type: "USB serial adapter".repeat(4),
            serial_number: Some(format!("SERIAL-{index}")),
        })
        .collect();
    let ctx = egui::Context::default();
    let scan = SerialScanState::Ready(ports);
    let mut baud_rate = 115_200;

    for time in [1.0, 2.0] {
        ctx.begin_pass(egui::RawInput {
            screen_rect: Some(viewport),
            time: Some(time),
            ..Default::default()
        });
        egui::CentralPanel::default().show(&ctx, |ui| {
            egui::ScrollArea::vertical()
                .max_height(viewport.height())
                .auto_shrink([false, true])
                .show(ui, |ui| {
                    let content_width = ui.available_width();
                    ui.set_width(content_width);
                    let mut action = NewTabAction::None;
                    draw_serial_section(ui, &scan, &mut baud_rate, &mut action);
                });
        });
        let output = ctx.end_pass();
        let rows = serial_row_rects(&output.shapes);

        assert!(!rows.is_empty(), "第 {time} 帧应绘制可见串口行");
        for (row, scroll_clip) in rows {
            assert!(
                row.left() >= scroll_clip.left() && row.right() <= scroll_clip.right(),
                "第 {time} 帧串口行越过滚动视口：row={row:?} clip={scroll_clip:?}"
            );
            assert!(
                row.width() <= scroll_clip.width(),
                "第 {time} 帧串口行产生水平溢出：row={row:?} clip={scroll_clip:?}"
            );
        }
    }
}

#[test]
fn serial_scan_generation_rejects_stale_results() {
    let mut selector = NewTabSelector::new();
    let first = selector.begin_serial_scan();
    let current = selector.begin_serial_scan();
    assert!(!selector.apply_serial_scan(first, Ok(Vec::new())));
    assert!(selector.apply_serial_scan(current, Ok(Vec::new())));
    assert!(matches!(selector.serial_scan, SerialScanState::Ready(_)));
}

#[test]
fn serial_columns_are_ordered_and_share_exact_boundaries() {
    let rect = egui::Rect::from_min_size(egui::pos2(10.0, 20.0), egui::vec2(500.0, 34.0));
    let columns = serial_column_rects(rect);

    assert_eq!(columns.device.right(), columns.model.left());
    assert_eq!(columns.model.right(), columns.serial.left());
    assert_eq!(columns.serial.right(), columns.kind.left());
    assert!(columns.device.left() >= rect.left());
    assert!(columns.kind.right() <= rect.right());
    assert!(columns.device.width() < columns.model.width());
}

#[test]
fn modal_geometry_rejects_non_finite_zero_and_inverted_viewports() {
    let non_finite = egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(f32::INFINITY, 100.0));
    let zero = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(0.0, 100.0));
    let inverted = egui::Rect::from_min_max(egui::pos2(20.0, 20.0), egui::pos2(10.0, 10.0));

    assert!(modal_geometry(non_finite).is_none());
    assert!(modal_geometry(zero).is_none());
    assert!(modal_geometry(inverted).is_none());
}

#[test]
fn modal_root_captures_clicks_without_claiming_drag_gestures() {
    let sense = modal_root_sense();

    assert!(sense.senses_click());
    assert!(!sense.senses_drag());
}

#[test]
fn new_selector_defers_shell_discovery_until_open() {
    let selector = NewTabSelector::new();

    assert!(selector.shells.is_empty());
    assert!(!selector.is_open());
}

#[test]
fn accepted_open_action_keeps_selector_visible() {
    let mut selector = NewTabSelector::new();
    selector.visible = true;
    let action = NewTabAction::OpenShell(Path::new("/bin/bash").to_path_buf());

    let returned = selector.apply_action(action.clone());

    assert_eq!(returned, action);
    assert!(selector.is_open());
}

#[test]
fn accepted_new_ssh_action_keeps_selector_visible_for_main() {
    let mut selector = NewTabSelector::new();
    selector.visible = true;

    let returned = selector.apply_action(NewTabAction::NewSsh);

    assert_eq!(returned, NewTabAction::NewSsh);
    assert_eq!(format!("{returned:?}"), "NewSsh");
    assert!(selector.is_open());
}

#[test]
fn close_action_hides_selector() {
    let mut selector = NewTabSelector::new();
    selector.visible = true;

    let returned = selector.apply_action(NewTabAction::Close);

    assert_eq!(returned, NewTabAction::Close);
    assert!(!selector.is_open());
}

#[test]
fn parser_ignores_blank_and_comment_lines() {
    let parsed = parse_shells_with(
        "\n  # login shells\n/bin/bash\n\t\n# /bin/zsh\n/usr/bin/fish\n",
        executable,
    );

    assert_eq!(
        parsed,
        vec![
            Path::new("/bin/bash").to_path_buf(),
            Path::new("/usr/bin/fish").to_path_buf(),
        ]
    );
}

#[test]
fn parser_accepts_only_absolute_executable_files() {
    let parsed = parse_shells_with(
        "bin/bash\n/bin/bash\n/bin/not-executable\n/usr/bin/fish\n",
        executable,
    );

    assert_eq!(
        parsed,
        vec![
            Path::new("/bin/bash").to_path_buf(),
            Path::new("/usr/bin/fish").to_path_buf(),
        ]
    );
}

#[test]
fn parser_deduplicates_shells_without_reordering_them() {
    let parsed = parse_shells_with(
        "/bin/zsh\n/bin/bash\n/bin/zsh\n/usr/bin/fish\n/bin/bash\n",
        executable,
    );

    assert_eq!(
        parsed,
        vec![
            Path::new("/bin/zsh").to_path_buf(),
            Path::new("/bin/bash").to_path_buf(),
            Path::new("/usr/bin/fish").to_path_buf(),
        ]
    );
}

#[test]
fn shell_labels_are_basenames_only() {
    assert_eq!(shell_display_name(Path::new("/bin/bash")), "bash");
    assert_eq!(shell_display_name(Path::new("/usr/local/bin/fish")), "fish");
    assert_eq!(shell_display_name(Path::new("zsh")), "zsh");
}

#[test]
fn executable_environment_shell_is_used_when_shell_file_is_unavailable() {
    let parsed = shell_candidates_with_fallback(None, Some(OsStr::new("/bin/zsh")), executable);

    assert_eq!(parsed, vec![Path::new("/bin/zsh").to_path_buf()]);
}

#[test]
fn executable_environment_shell_is_used_when_parsed_list_is_empty() {
    let parsed = shell_candidates_with_fallback(
        Some("# no configured shells\nrelative-shell\n"),
        Some(OsStr::new("/bin/bash")),
        executable,
    );

    assert_eq!(parsed, vec![Path::new("/bin/bash").to_path_buf()]);
}

#[test]
fn invalid_environment_shell_is_not_used_as_fallback() {
    let parsed = shell_candidates_with_fallback(Some(""), Some(OsStr::new("bin/bash")), executable);

    assert!(parsed.is_empty());
}

#[test]
fn ssh_identity_distinguishes_connections_with_duplicate_labels() {
    let first = connection("生产机", "server-a.example", "root", "");
    let second = connection("生产机", "server-b.example", "deploy", "");

    assert_ne!(
        ssh_connection_key(0, &first),
        ssh_connection_key(1, &second),
        "显示标签不得作为 SSH 连接的唯一标识"
    );
}

#[test]
fn ssh_connections_group_by_label_and_color_in_first_seen_order() {
    let mut blue_first = connection("A", "a.example", "root", "");
    blue_first.group = "生产".into();
    blue_first.group_color = [0x58, 0xa6, 0xff];
    let mut green = connection("B", "b.example", "deploy", "");
    green.group = "测试".into();
    green.group_color = [0x3f, 0xb9, 0x50];
    let mut blue_second = connection("C", "c.example", "ops", "");
    blue_second.group = "生产".into();
    blue_second.group_color = [0x58, 0xa6, 0xff];
    let mut red_same_label = connection("D", "d.example", "admin", "");
    red_same_label.group = "生产".into();
    red_same_label.group_color = [0xf8, 0x51, 0x49];
    let connections = vec![blue_first, green, blue_second, red_same_label];

    let groups = group_ssh_connections(&connections);

    assert_eq!(groups.len(), 3);
    assert_eq!(
        (groups[0].label, groups[0].color),
        ("生产", [0x58, 0xa6, 0xff])
    );
    assert_eq!(
        groups[0]
            .rows
            .iter()
            .map(|row| row.snapshot_index)
            .collect::<Vec<_>>(),
        vec![0, 2]
    );
    assert_eq!(
        (groups[1].label, groups[1].color),
        ("测试", [0x3f, 0xb9, 0x50])
    );
    assert_eq!(groups[1].rows[0].snapshot_index, 1);
    assert_eq!(
        (groups[2].label, groups[2].color),
        ("生产", [0xf8, 0x51, 0x49])
    );
    assert_eq!(groups[2].rows[0].snapshot_index, 3);
}

#[test]
fn ssh_endpoint_contains_host_and_port_but_not_username() {
    let connection = connection("生产机", "server.example", "root", "");

    let endpoint = ssh_endpoint_label(&connection);

    assert_eq!(endpoint, "server.example:22");
    assert!(!endpoint.contains("root"));
}

#[test]
fn ssh_identity_distinguishes_same_public_fields_with_different_passwords() {
    let first = connection("生产机", "server.example", "root", "first-secret");
    let second = connection("生产机", "server.example", "root", "second-secret");
    let mut connections = vec![first];

    let first_key = ssh_connection_key(0, &connections[0]);
    connections[0] = second;
    let second_key = ssh_connection_key(0, &connections[0]);

    assert_ne!(first_key, second_key);
    assert!(resolve_ssh_connection(&connections, &first_key).is_none());
    assert!(std::ptr::eq(
        resolve_ssh_connection(&connections, &second_key).expect("更新后的连接应可解析"),
        &connections[0]
    ));
    assert!(!first_key.contains("first-secret"));
    assert!(!second_key.contains("second-secret"));
    assert!(!first_key.contains("id_ed25519"));
}

#[test]
fn ssh_identity_detects_key_path_changes_at_the_same_snapshot_index() {
    let first = connection("生产机", "server.example", "root", "");
    let mut second = first.clone();
    second.key_path = "~/.ssh/another_ed25519".into();

    let first_key = ssh_connection_key(0, &first);
    let second_key = ssh_connection_key(0, &second);

    assert_ne!(first_key, second_key);
    assert!(!first_key.contains("id_ed25519"));
    assert!(!second_key.contains("another_ed25519"));
}

#[test]
fn ssh_identity_distinguishes_exact_duplicate_records_by_snapshot_index() {
    let first = connection("生产机", "server.example", "root", "same-secret");
    let second = first.clone();
    let connections = vec![first, second];

    let first_key = ssh_connection_key(0, &connections[0]);
    let second_key = ssh_connection_key(1, &connections[1]);

    assert_ne!(first_key, second_key);
    assert!(std::ptr::eq(
        resolve_ssh_connection(&connections, &first_key).expect("第一个连接应可解析"),
        &connections[0]
    ));
    assert!(std::ptr::eq(
        resolve_ssh_connection(&connections, &second_key).expect("第二个连接应可解析"),
        &connections[1]
    ));
}

#[test]
fn ssh_resolver_rejects_unknown_tampered_and_ambiguous_ids() {
    let connection = connection("生产机", "server.example", "root", "secret");
    let connections = vec![connection];
    let valid = ssh_connection_key(0, &connections[0]);
    let mut tampered = valid.clone();
    tampered.push('0');
    let unknown_index = ssh_connection_key(9, &connections[0]);

    assert!(resolve_ssh_connection(&connections, "ssh-v2").is_none());
    assert!(resolve_ssh_connection(&connections, &tampered).is_none());
    assert!(resolve_ssh_connection(&connections, &unknown_index).is_none());
}

#[test]
fn ssh_resolver_uses_the_stable_connection_identity() {
    let first = connection("生产机", "server-a.example", "root", "");
    let second = connection("生产机", "server-b.example", "deploy", "");
    let connections = vec![first, second.clone()];
    let key = ssh_connection_key(1, &second);

    let resolved = resolve_ssh_connection(&connections, &key).expect("连接应可解析");

    assert_eq!(resolved.host, "server-b.example");
    assert_eq!(resolved.user, "deploy");
}

#[test]
fn ssh_action_debug_output_redacts_identifier() {
    let connection = connection("生产机", "server.example", "root", "secret");
    let key = ssh_connection_key(0, &connection);
    let debug = format!("{:?}", NewTabAction::OpenSsh(key.clone()));

    assert!(!debug.contains(&key));
    assert!(!debug.contains("server.example"));
    assert_eq!(debug, "OpenSsh(\"<redacted>\")");
}
