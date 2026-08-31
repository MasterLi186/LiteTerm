use super::*;

fn monitor_with_rate(iface: &str, rx_rate: u64, tx_rate: u64) -> crate::monitor::MonitorData {
    crate::monitor::MonitorData {
        cpu_percent: 10.0,
        cpu_name: "Test CPU".into(),
        memory_used: 1,
        memory_total: 2,
        memory_text: "1G / 2G".into(),
        memory_percent: 50.0,
        swap_used: 0,
        swap_total: 0,
        swap_text: "0K / 0K".into(),
        swap_percent: 0.0,
        uptime_text: "1分钟".into(),
        load_text: "0.1, 0.2, 0.3".into(),
        disk_items: Vec::new(),
        processes: Vec::new(),
        zombie_processes: Vec::new(),
        process_stats: crate::monitor::ProcessStats::default(),
        net_interfaces: vec![crate::monitor::NetIfaceInfo {
            name: iface.into(),
            rx_rate,
            tx_rate,
        }],
        preferred_net_interface: Some(iface.into()),
    }
}

fn sample_process(memory: &str, cpu: f32, name: &str) -> crate::monitor::ProcessInfo {
    crate::monitor::ProcessInfo {
        pid: 1,
        user: "tester".into(),
        state: "R".into(),
        mem_mb: memory.into(),
        mem_bytes: 1,
        resident_mem_mb: memory.into(),
        resident_mem_bytes: 1,
        cpu,
        name: name.into(),
        command: name.into(),
        start_time: String::new(),
    }
}

fn sample_disk(mount: &str, percent: u8, avail: &str, size: &str) -> crate::monitor::DiskItem {
    crate::monitor::DiskItem {
        mount: mount.into(),
        avail: avail.into(),
        size: size.into(),
        percent,
    }
}

#[test]
fn ssh_connection_debug_redacts_key_path_and_password() {
    let connection = SshConnection {
        label: "生产机".into(),
        host: "server.example.com".into(),
        port: 2222,
        user: "deploy".into(),
        auth: "key".into(),
        key_path: "KEY_PATH_SENTINEL".into(),
        password: "PASSWORD_SENTINEL".into(),
        group: "生产".into(),
        group_color: [1, 2, 3],
    };

    let debug = format!("{connection:?}");

    assert!(debug.contains("生产机"));
    assert!(debug.contains("server.example.com"));
    assert!(debug.contains("deploy"));
    assert!(!debug.contains("KEY_PATH_SENTINEL"));
    assert!(!debug.contains("PASSWORD_SENTINEL"));
}

#[derive(Debug)]
struct PaintedText {
    text: String,
    pos: egui::Pos2,
    color: egui::Color32,
    halign: egui::Align,
}

fn collect_painted_text(shape: &egui::epaint::Shape, output: &mut Vec<PaintedText>) {
    match shape {
        egui::epaint::Shape::Text(text) => {
            let section_color = text
                .galley
                .job
                .sections
                .first()
                .map(|section| section.format.color)
                .unwrap_or(text.fallback_color);
            let color = text.override_text_color.unwrap_or_else(|| {
                if section_color == egui::Color32::PLACEHOLDER {
                    text.fallback_color
                } else {
                    section_color
                }
            });
            output.push(PaintedText {
                text: text.galley.job.text.clone(),
                pos: text.pos,
                color,
                halign: text.galley.job.halign,
            });
        }
        egui::epaint::Shape::Vec(shapes) => {
            for shape in shapes {
                collect_painted_text(shape, output);
            }
        }
        _ => {}
    }
}

fn render_sidebar_text(sidebar: &mut Sidebar) -> Vec<PaintedText> {
    let ctx = egui::Context::default();
    let mut painted_text = Vec::new();
    for _ in 0..2 {
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(800.0, 600.0),
            )),
            ..Default::default()
        };
        let output = ctx.run(input, |ctx| {
            sidebar.ui(ctx);
        });
        painted_text.clear();
        for clipped in &output.shapes {
            collect_painted_text(&clipped.shape, &mut painted_text);
        }
    }
    painted_text
}

#[test]
fn connection_group_name_uses_readable_secondary_text_color() {
    let group_name = "生产分组颜色探针";
    let mut sidebar = Sidebar::new();
    sidebar.connections = vec![SshConnection {
        label: "测试连接".into(),
        host: "127.0.0.1".into(),
        port: 22,
        user: "root".into(),
        auth: "password".into(),
        key_path: String::new(),
        password: String::new(),
        group: group_name.into(),
        group_color: [0x58, 0xa6, 0xff],
    }];
    sidebar.connections_visible = true;

    let painted_text = render_sidebar_text(&mut sidebar);
    let group = painted_text
        .iter()
        .find(|text| text.text == group_name)
        .unwrap_or_else(|| panic!("missing rendered group name in {painted_text:?}"));

    assert_eq!(group.color, egui::Color32::from_rgb(0x8b, 0x94, 0x9e));
}

#[test]
fn sidebar_typography_uses_local_readable_sizes() {
    assert_eq!(SIDEBAR_META_SIZE, 11.0);
    assert_eq!(SIDEBAR_BODY_SIZE, 12.0);
    assert_eq!(SIDEBAR_SECTION_SIZE, 12.0);
    assert_eq!(SIDEBAR_VALUE_SIZE, 13.0);
}

#[test]
fn sidebar_card_inner_width_subtracts_both_margins_and_saturates() {
    assert_eq!(sidebar_card_inner_width(196.0, 8.0), 180.0);
    assert_eq!(sidebar_card_inner_width(12.0, 8.0), 0.0);
}

#[test]
fn monitor_card_width_preserves_normal_size_and_shrinks_to_available_space() {
    assert_eq!(sidebar_monitor_card_width(220.0, 220.0), 218.0);
    assert_eq!(sidebar_monitor_card_width(220.0, 120.0), 118.0);
    assert_eq!(sidebar_monitor_card_width(12.0, 1.0), 0.0);
}

#[test]
fn monitor_text_geometry_matches_the_rendered_card_interior() {
    assert_eq!(sidebar_uptime_column_width(218.0), 101.0);
    assert_eq!(sidebar_cpu_text_width(218.0), 202.0);
    assert_eq!(sidebar_uptime_column_width(118.0), 51.0);
    assert_eq!(sidebar_cpu_text_width(118.0), 102.0);
}

#[test]
fn monitor_frame_geometry_never_exceeds_available_width() {
    let cases: [(f32, f32); 6] = [
        (220.0, 220.0),
        (220.0, 120.0),
        (220.0, 12.0),
        (220.0, 1.0),
        (220.0, 0.0),
        (220.0, -10.0),
    ];

    for (panel_width, available_width) in cases {
        let geometry = sidebar_monitor_card_geometry(panel_width, available_width);
        let outer_limit = panel_width.max(0.0).min(available_width.max(0.0));
        let uptime_outer_width = geometry.uptime_content_width
            + geometry.uptime_inner_margin * 2.0
            + geometry.stroke_width * 2.0;
        let regular_outer_width = geometry.card_content_width + geometry.stroke_width * 2.0;

        assert!(geometry.card_content_width >= 0.0);
        assert!(geometry.uptime_content_width >= 0.0);
        assert!(geometry.uptime_inner_margin >= 0.0);
        assert!(geometry.stroke_width >= 0.0);
        assert_eq!(uptime_outer_width, outer_limit);
        assert_eq!(regular_outer_width, outer_limit);
    }

    let normal = sidebar_monitor_card_geometry(220.0, 220.0);
    assert_eq!(normal.card_content_width, 218.0);
    assert_eq!(normal.uptime_content_width, 202.0);
    assert_eq!(normal.uptime_inner_margin, 8.0);
    assert_eq!(normal.stroke_width, 1.0);
    assert_eq!(normal.card_content_width + normal.stroke_width * 2.0, 220.0);
    assert!(normal.can_render);

    let narrow = sidebar_monitor_card_geometry(220.0, 120.0);
    assert_eq!(narrow.card_content_width, 118.0);
    assert_eq!(narrow.uptime_content_width, 102.0);
    assert_eq!(narrow.card_content_width + narrow.stroke_width * 2.0, 120.0);
    assert!(narrow.can_render);

    let tiny = sidebar_monitor_card_geometry(220.0, 12.0);
    assert_eq!(tiny.card_content_width, 10.0);
    assert_eq!(tiny.uptime_content_width, 0.0);
    assert_eq!(tiny.uptime_inner_margin, 5.0);
    assert_eq!(tiny.stroke_width, 1.0);
    assert!(!tiny.can_render);

    let subpixel = sidebar_monitor_card_geometry(220.0, 1.0);
    assert_eq!(subpixel.card_content_width, 0.0);
    assert_eq!(subpixel.uptime_inner_margin, 0.0);
    assert_eq!(subpixel.stroke_width, 0.5);
    assert!(!subpixel.can_render);
}

#[test]
fn monitor_frame_geometry_normalizes_non_finite_dimensions_without_overflow() {
    let cases = [
        (f32::NAN, 220.0),
        (f32::INFINITY, 220.0),
        (f32::NEG_INFINITY, 220.0),
        (220.0, f32::NAN),
        (220.0, f32::INFINITY),
        (220.0, f32::NEG_INFINITY),
        (f32::NAN, f32::INFINITY),
    ];

    for (panel_width, available_width) in cases {
        let normalized_panel = if panel_width.is_finite() {
            panel_width.max(0.0)
        } else {
            0.0
        };
        let normalized_available = if available_width.is_finite() {
            available_width.max(0.0)
        } else {
            0.0
        };
        let outer_limit = normalized_panel.min(normalized_available);
        let card_content_width = sidebar_monitor_card_width(panel_width, available_width);
        let geometry = sidebar_monitor_card_geometry(panel_width, available_width);
        let regular_outer_width = geometry.card_content_width + geometry.stroke_width * 2.0;
        let uptime_outer_width = geometry.uptime_content_width
            + geometry.uptime_inner_margin * 2.0
            + geometry.stroke_width * 2.0;

        assert!(card_content_width.is_finite());
        assert!(card_content_width >= 0.0);
        assert_eq!(card_content_width, geometry.card_content_width);
        assert!(geometry.card_content_width.is_finite());
        assert!(geometry.uptime_content_width.is_finite());
        assert!(geometry.uptime_inner_margin.is_finite());
        assert!(geometry.stroke_width.is_finite());
        assert!(regular_outer_width.is_finite());
        assert!(uptime_outer_width.is_finite());
        assert!(regular_outer_width <= outer_limit);
        assert!(uptime_outer_width <= outer_limit);
    }
}

#[test]
fn monitor_card_frame_uses_the_requested_outer_width_with_wide_children() {
    fn fixed_frame_response(
        ui: &mut egui::Ui,
        outer_width: f32,
        id: &'static str,
    ) -> egui::Response {
        let geometry = sidebar_monitor_card_geometry(outer_width, outer_width);
        show_sidebar_monitor_card(
            ui,
            geometry,
            egui::Color32::TRANSPARENT,
            egui::Color32::GRAY,
            0.0,
            |ui| {
                ui.set_width(geometry.card_content_width);
                ui.add_sized(
                    [outer_width + 80.0, 0.0],
                    egui::Label::new("比监控卡更宽的内容"),
                );
                egui::ComboBox::from_id_salt(id)
                    .selected_text("宽下拉框")
                    .width(outer_width + 40.0)
                    .show_ui(ui, |ui| {
                        ui.label("选项");
                    });
            },
        )
        .expect("renderable geometry should create a monitor card")
        .response
    }

    egui::__run_test_ui(|ui| {
        let normal = fixed_frame_response(ui, 220.0, "normal_wide_card");
        let narrow = fixed_frame_response(ui, 120.0, "narrow_wide_card");
        eprintln!(
            "fixed monitor card response widths: normal={}, narrow={}",
            normal.rect.width(),
            narrow.rect.width()
        );

        assert_eq!(normal.rect.width(), 220.0);
        assert_eq!(narrow.rect.width(), 120.0);
    });
}

fn render_clickable_monitor_row(
    ctx: &egui::Context,
    events: Vec<egui::Event>,
) -> (bool, egui::Rect) {
    let input = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(320.0, 200.0),
        )),
        events,
        ..Default::default()
    };
    let mut clicked = false;
    let mut row_rect = egui::Rect::NOTHING;
    let _ = ctx.run(input, |ctx| {
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.set_width(220.0);
            let geometry = sidebar_monitor_card_geometry(220.0, 220.0);
            let _ = show_sidebar_monitor_card(
                ui,
                geometry,
                egui::Color32::TRANSPARENT,
                egui::Color32::GRAY,
                0.0,
                |ui| {
                    let row_width = ui.available_width();
                    let columns = process_table_columns((row_width - 16.0).max(0.0));
                    let row_height = process_row_height(ui, columns, "128M", "liteterm-native");
                    let (_, response) = ui.allocate_exact_size(
                        egui::vec2(row_width, row_height),
                        egui::Sense::click(),
                    );
                    render_process_row_content(
                        ui,
                        response.rect,
                        columns,
                        "128M",
                        42.0,
                        "liteterm-native",
                        egui::Color32::GRAY,
                    );
                    clicked = response.clicked();
                    row_rect = response.rect;
                },
            );
        });
    });
    (clicked, row_rect)
}

#[test]
fn monitor_card_background_does_not_intercept_process_row_clicks() {
    let ctx = egui::Context::default();
    let (_, row_rect) = render_clickable_monitor_row(&ctx, Vec::new());
    let position = row_rect.center();

    let _ = render_clickable_monitor_row(
        &ctx,
        vec![
            egui::Event::PointerMoved(position),
            egui::Event::PointerButton {
                pos: position,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::NONE,
            },
        ],
    );
    let (clicked, _) = render_clickable_monitor_row(
        &ctx,
        vec![egui::Event::PointerButton {
            pos: position,
            button: egui::PointerButton::Primary,
            pressed: false,
            modifiers: egui::Modifiers::NONE,
        }],
    );

    assert!(clicked, "监控卡背景不得遮挡进程行的点击响应");
}

#[test]
fn process_row_memory_cpu_and_name_columns_are_all_clickable() {
    for fraction in [0.08, 0.42, 0.88] {
        let ctx = egui::Context::default();
        let (_, row_rect) = render_clickable_monitor_row(&ctx, Vec::new());
        let position = egui::pos2(
            egui::lerp(row_rect.left()..=row_rect.right(), fraction),
            row_rect.center().y,
        );
        let _ = render_clickable_monitor_row(
            &ctx,
            vec![
                egui::Event::PointerMoved(position),
                egui::Event::PointerButton {
                    pos: position,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
        );
        let (clicked, _) = render_clickable_monitor_row(
            &ctx,
            vec![egui::Event::PointerButton {
                pos: position,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::NONE,
            }],
        );
        assert!(clicked, "进程行 {:.0}% 位置应响应点击", fraction * 100.0);
    }
}

#[test]
fn monitor_card_frame_preserves_insets_and_advances_parent_without_overlap() {
    egui::__run_test_ui(|ui| {
        let geometry = sidebar_monitor_card_geometry(220.0, 220.0);
        let item_spacing = ui.spacing().item_spacing.y;
        let card = show_sidebar_monitor_card(
            ui,
            geometry,
            egui::Color32::TRANSPARENT,
            egui::Color32::GRAY,
            8.0,
            |ui| {
                let content_origin = ui.max_rect().min;
                let last = ui.label("第二行");
                (content_origin, last.rect.bottom())
            },
        )
        .expect("normal geometry should create a monitor card");
        let cursor_after_card = ui.cursor();
        let following = ui.label("后续控件");

        assert_eq!(card.response.rect.width(), 220.0);
        assert_eq!(card.inner.0.x - card.response.rect.left(), 9.0);
        assert_eq!(card.inner.0.y - card.response.rect.top(), 9.0);
        assert_eq!(card.response.rect.bottom() - card.inner.1, 9.0);
        assert_eq!(
            cursor_after_card.top(),
            card.response.rect.bottom() + item_spacing
        );
        assert!(following.rect.top() >= card.response.rect.bottom());
    });
}

#[test]
fn monitor_card_frame_skips_non_renderable_geometry_without_advancing_parent() {
    egui::__run_test_ui(|ui| {
        let geometry = sidebar_monitor_card_geometry(220.0, 12.0);
        assert!(!geometry.can_render);
        let cursor_before = ui.cursor();
        let mut called = false;
        let card = show_sidebar_monitor_card(
            ui,
            geometry,
            egui::Color32::TRANSPARENT,
            egui::Color32::GRAY,
            0.0,
            |_| {
                called = true;
            },
        );

        assert!(card.is_none());
        assert!(!called);
        assert_eq!(ui.cursor(), cursor_before);
    });
}

#[test]
fn process_table_columns_fit_and_are_shared_by_every_row() {
    egui::__run_test_ui(|_ui| {
        let content_width = 202.0;
        let columns = process_table_columns(content_width);
        let total_width = columns.memory_width
            + columns.cpu_width
            + columns.command_width
            + columns.gap_width * 2.0;

        assert!(total_width <= content_width);
        assert!(columns.memory_width > 0.0);
        assert!(columns.cpu_width > 0.0);
        assert!(columns.command_width > 0.0);
        assert!((columns.memory_width - columns.cpu_width).abs() < f32::EPSILON);
        assert!((columns.cpu_width - columns.command_width).abs() < 0.001);

        let first = process_row_rects(
            egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(218.0, 24.0)),
            columns,
        );
        let second = process_row_rects(
            egui::Rect::from_min_size(egui::pos2(0.0, 24.0), egui::vec2(218.0, 48.0)),
            columns,
        );
        assert_eq!(first.cpu.left(), second.cpu.left());
        assert_eq!(first.command.left(), second.command.left());
    });
}

#[test]
fn process_row_content_does_not_advance_the_parent_cursor() {
    egui::__run_test_ui(|ui| {
        let processes = [sample_process(
            "123 MiB",
            42.0,
            "a very long process command that wraps",
        )];
        let row_width = 218.0;
        let columns = process_table_columns(row_width - 16.0);
        let row_height = process_row_height(ui, columns, "123 MiB", &processes[0].name);
        let (row_rect, _) =
            ui.allocate_exact_size(egui::vec2(row_width, row_height), egui::Sense::hover());
        let cursor_after_row = ui.cursor();

        render_process_row_content(
            ui,
            row_rect,
            columns,
            "123 MiB",
            42.0,
            &processes[0].name,
            egui::Color32::GRAY,
        );

        assert_eq!(ui.cursor(), cursor_after_row);
    });
}

#[test]
fn disk_table_columns_fit_content_and_rects_do_not_overlap() {
    egui::__run_test_ui(|ui| {
        let disks = [
            sample_disk("/", 8, "163.9G", "186.7G"),
            sample_disk("/7940_01", 99, "195.2G", "13.9T"),
        ];
        let content_width = 202.0;
        let columns = disk_table_columns(ui, content_width, &disks);
        let total_width = columns.mount_width
            + columns.percent_width
            + columns.capacity_width
            + columns.gap_width * 2.0;
        assert!(total_width <= content_width);

        let row_rect = egui::Rect::from_min_size(
            egui::pos2(10.0, 20.0),
            egui::vec2(218.0, DISK_ROW_MIN_HEIGHT),
        );
        let rects = disk_row_rects(row_rect, columns);
        assert!(row_rect.contains_rect(rects.mount));
        assert!(row_rect.contains_rect(rects.percent));
        assert!(row_rect.contains_rect(rects.capacity));
        assert!(rects.mount.right() <= rects.percent.left());
        assert!(rects.percent.right() <= rects.capacity.left());
    });
}

#[test]
fn disk_row_content_wraps_without_advancing_or_expanding_parent() {
    egui::__run_test_ui(|ui| {
        let disks = [sample_disk(
            "/这是一个很长的挂载点/資料/🚀/不会按字节切片",
            96,
            "933.4G",
            "1.8T",
        )];
        let row_width = 218.0;
        let columns = disk_table_columns(ui, row_width - 16.0, &disks);
        let row_height = disk_row_height(
            ui,
            row_width,
            columns,
            &disks[0].mount,
            "96%",
            "933.4G/1.8T",
        );
        let (row_rect, _) =
            ui.allocate_exact_size(egui::vec2(row_width, row_height), egui::Sense::hover());
        let cursor_after_row = ui.cursor();
        let parent_min_rect_after_row = ui.min_rect();

        render_disk_row_content(
            ui,
            row_rect,
            columns,
            &disks[0].mount,
            "96%",
            "933.4G/1.8T",
            DiskRowColors {
                mount: egui::Color32::GRAY,
                percent: egui::Color32::RED,
                capacity: egui::Color32::LIGHT_GRAY,
            },
        );

        assert_eq!(ui.cursor(), cursor_after_row);
        assert_eq!(ui.min_rect(), parent_min_rect_after_row);
    });
}

#[test]
fn network_line_points_map_history_to_the_full_chart_rect() {
    let rect = egui::Rect::from_min_max(egui::pos2(10.0, 20.0), egui::pos2(110.0, 70.0));
    let points = network_line_points(&[0.0, 5.0, 10.0], rect, 10.0);

    assert_eq!(points.len(), 3);
    assert_eq!(points.first().unwrap().x, rect.left());
    assert_eq!(points.last().unwrap().x, rect.right());
    assert!(points.last().unwrap().y < points.first().unwrap().y);
}

#[test]
fn network_line_points_require_two_history_samples() {
    let rect = egui::Rect::from_min_max(egui::pos2(10.0, 20.0), egui::pos2(110.0, 70.0));

    assert!(network_line_points(&[], rect, 10.0).is_empty());
    assert!(network_line_points(&[5.0], rect, 10.0).is_empty());
}

#[test]
fn network_line_points_normalize_non_positive_max_without_non_finite_points() {
    let rect = egui::Rect::from_min_max(egui::pos2(10.0, 20.0), egui::pos2(110.0, 70.0));

    for max_value in [0.0, -10.0] {
        let points = network_line_points(&[0.0, 0.0], rect, max_value);
        assert_eq!(points.len(), 2);
        assert!(points
            .iter()
            .all(|point| point.x.is_finite() && point.y.is_finite()));
    }
}

#[test]
fn network_line_points_keep_tiny_chart_heights_inside_the_rect() {
    for height in [1.0, 3.5] {
        let rect = egui::Rect::from_min_size(egui::pos2(10.0, 20.0), egui::vec2(100.0, height));
        let points = network_line_points(&[0.0, 5.0, 10.0], rect, 10.0);

        assert_eq!(points.len(), 3);
        assert!(points.iter().all(|point| rect.contains(*point)));
    }
}

#[test]
fn network_line_points_reject_non_finite_inputs() {
    let normal_rect = egui::Rect::from_min_max(egui::pos2(10.0, 20.0), egui::pos2(110.0, 70.0));
    let invalid_rects = [
        egui::Rect {
            min: egui::pos2(f32::NAN, 20.0),
            max: egui::pos2(110.0, 70.0),
        },
        egui::Rect {
            min: egui::pos2(10.0, 20.0),
            max: egui::pos2(f32::INFINITY, 70.0),
        },
        egui::Rect {
            min: egui::pos2(10.0, 20.0),
            max: egui::pos2(110.0, f32::INFINITY),
        },
    ];

    for rect in invalid_rects {
        assert!(network_line_points(&[0.0, 10.0], rect, 10.0).is_empty());
    }
    for max_value in [f64::NAN, f64::INFINITY] {
        assert!(network_line_points(&[0.0, 10.0], normal_rect, max_value).is_empty());
    }
    for data in [[f64::NAN, 10.0], [0.0, f64::INFINITY]] {
        assert!(network_line_points(&data, normal_rect, 10.0).is_empty());
    }
}

#[test]
fn network_line_points_share_vertical_mapping_when_given_the_same_max() {
    let rect = egui::Rect::from_min_max(egui::pos2(10.0, 20.0), egui::pos2(110.0, 70.0));
    let tx_points = network_line_points(&[0.0, 5.0], rect, 10.0);
    let rx_points = network_line_points(&[5.0, 10.0], rect, 10.0);

    assert_eq!(tx_points[1].y, rx_points[0].y);
}

#[test]
fn remote_monitor_presentation_without_snapshot_never_falls_back_to_local() {
    let presentation = super::monitor_source_presentation(
        &crate::monitor::MonitorKey::remote("alice", "alpha.example", 22),
        None,
        None,
    );

    assert_eq!(presentation.title, "已连接");
    assert_eq!(presentation.detail, "alice@alpha.example:22");
    assert_eq!(presentation.message, "正在采集");
    assert_eq!(presentation.dot, super::MonitorDot::Remote);
    assert_ne!(presentation.title, "本机");
}

#[test]
fn process_manager_open_action_preserves_local_and_remote_monitor_keys() {
    let remote = crate::monitor::MonitorKey::remote("alice", "alpha.example", 22);

    assert_eq!(
        process_manager_open_action(&remote),
        Some(OpenProcessManagerAction {
            key: remote.clone()
        })
    );
    assert_eq!(
        process_manager_open_action(&crate::monitor::MonitorKey::Local),
        Some(OpenProcessManagerAction {
            key: crate::monitor::MonitorKey::Local
        })
    );
}

#[test]
fn process_manager_open_action_can_be_taken_once() {
    let mut sidebar = Sidebar::new_for_test();
    let key = crate::monitor::MonitorKey::remote("alice", "alpha.example", 22);
    sidebar.open_process_manager = process_manager_open_action(&key);

    assert_eq!(
        sidebar.take_open_process_manager(),
        Some(OpenProcessManagerAction { key })
    );
    assert_eq!(sidebar.take_open_process_manager(), None);
}

#[test]
fn monitor_presentation_omits_redundant_cpu_and_memory_summary() {
    let snapshot = crate::monitor::MonitorData {
        cpu_percent: f32::NAN,
        cpu_name: String::new(),
        memory_used: 0,
        memory_total: 0,
        memory_text: "1G / 2G".into(),
        memory_percent: 0.0,
        swap_used: 0,
        swap_total: 0,
        swap_text: String::new(),
        swap_percent: 0.0,
        uptime_text: String::new(),
        load_text: String::new(),
        disk_items: Vec::new(),
        processes: Vec::new(),
        zombie_processes: Vec::new(),
        process_stats: crate::monitor::ProcessStats::default(),
        net_interfaces: Vec::new(),
        preferred_net_interface: None,
    };

    let presentation = super::monitor_source_presentation(
        &crate::monitor::MonitorKey::Local,
        Some(&snapshot),
        None,
    );

    assert!(presentation.message.is_empty());
}

#[test]
fn network_history_is_isolated_between_remote_monitor_keys() {
    let mut sidebar = Sidebar::new_for_test();
    let a = crate::monitor::MonitorKey::remote("alice", "alpha.example", 22);
    let b = crate::monitor::MonitorKey::remote("alice", "beta.example", 22);

    sidebar.on_monitor_update(&a, &monitor_with_rate("eth0", 100, 200));
    sidebar.on_monitor_update(&b, &monitor_with_rate("ens3", 300, 400));

    assert_eq!(sidebar.monitor_view(&a).net_rx_history, [100.0]);
    assert_eq!(sidebar.monitor_view(&a).net_tx_history, [200.0]);
    assert_eq!(sidebar.monitor_view(&b).net_rx_history, [300.0]);
    assert_eq!(sidebar.monitor_view(&b).net_tx_history, [400.0]);
}

#[test]
fn duplicate_tabs_share_monitor_view_history_for_the_same_key() {
    let mut sidebar = Sidebar::new_for_test();
    let key = crate::monitor::MonitorKey::remote("alice", "alpha.example", 22);

    sidebar.on_monitor_update(&key, &monitor_with_rate("eth0", 100, 200));
    sidebar.on_monitor_update(&key, &monitor_with_rate("eth0", 150, 250));

    assert_eq!(sidebar.monitor_view(&key).net_rx_history, [100.0, 150.0]);
    assert_eq!(sidebar.monitor_view(&key).net_tx_history, [200.0, 250.0]);
}

#[test]
fn process_tab_and_interface_selection_are_isolated_by_monitor_key() {
    let mut sidebar = Sidebar::new_for_test();
    let local = crate::monitor::MonitorKey::Local;
    let remote = crate::monitor::MonitorKey::remote("alice", "alpha.example", 22);
    sidebar.on_monitor_update(&local, &monitor_with_rate("lo0", 10, 20));
    sidebar.on_monitor_update(&remote, &monitor_with_rate("eth0", 30, 40));

    {
        let local_view = sidebar.monitor_views.get_mut(&local).unwrap();
        local_view.process_tab = 2;
        local_view.selected_iface = Some("lo0".into());
    }

    assert_eq!(sidebar.monitor_view(&local).process_tab, 2);
    assert_eq!(
        sidebar.monitor_view(&local).selected_iface.as_deref(),
        Some("lo0")
    );
    assert_eq!(sidebar.monitor_view(&remote).process_tab, 1);
    assert_eq!(
        sidebar.monitor_view(&remote).selected_iface.as_deref(),
        Some("eth0")
    );
}

#[test]
fn monitor_errors_distinguish_empty_and_retained_snapshots() {
    let key = crate::monitor::MonitorKey::remote("alice", "alpha.example", 22);
    let error_only =
        monitor_source_presentation(&key, None, Some("监控更新失败：连接暂时不可用\n"));
    assert_eq!(error_only.message, "采集失败：连接暂时不可用");
    assert_eq!(error_only.warning, None);

    let snapshot = monitor_with_rate("eth0", 100, 200);
    let retained = monitor_source_presentation(&key, Some(&snapshot), Some("连接暂时不可用"));
    assert!(retained.message.is_empty());
    assert_eq!(retained.warning.as_deref(), Some("监控暂时中断"));
}

#[test]
fn remove_monitor_view_only_removes_the_target_key() {
    let mut sidebar = Sidebar::new_for_test();
    let a = crate::monitor::MonitorKey::remote("alice", "alpha.example", 22);
    let b = crate::monitor::MonitorKey::remote("alice", "beta.example", 22);
    sidebar.on_monitor_update(&a, &monitor_with_rate("eth0", 100, 200));
    sidebar.on_monitor_update(&b, &monitor_with_rate("ens3", 300, 400));

    sidebar.remove_monitor_view(&a);

    assert!(!sidebar.monitor_views.contains_key(&a));
    assert_eq!(sidebar.monitor_view(&b).net_rx_history, [300.0]);
}

#[test]
fn missing_selected_interface_falls_back_and_resets_history() {
    let mut sidebar = Sidebar::new_for_test();
    let key = crate::monitor::MonitorKey::remote("alice", "alpha.example", 22);
    sidebar.on_monitor_update(&key, &monitor_with_rate("eth0", 100, 200));

    sidebar.on_monitor_update(&key, &monitor_with_rate("ens3", 300, 400));

    let view = sidebar.monitor_view(&key);
    assert_eq!(view.selected_iface.as_deref(), Some("ens3"));
    assert_eq!(view.last_chart_iface.as_deref(), Some("ens3"));
    assert_eq!(view.net_rx_history, [300.0]);
    assert_eq!(view.net_tx_history, [400.0]);
}

#[test]
fn automatic_interface_uses_default_route_preference() {
    let mut sidebar = Sidebar::new_for_test();
    let key = crate::monitor::MonitorKey::Local;
    let mut snapshot = monitor_with_rate("docker0", 10, 20);
    snapshot.net_interfaces.push(crate::monitor::NetIfaceInfo {
        name: "enp4s0f1".into(),
        rx_rate: 300,
        tx_rate: 400,
    });
    snapshot.preferred_net_interface = Some("enp4s0f1".into());

    sidebar.on_monitor_update(&key, &snapshot);

    assert_eq!(
        sidebar.monitor_view(&key).selected_iface.as_deref(),
        Some("enp4s0f1")
    );
}

#[test]
fn manual_interface_survives_default_route_changes_until_it_disappears() {
    let mut sidebar = Sidebar::new_for_test();
    let key = crate::monitor::MonitorKey::Local;
    let mut snapshot = monitor_with_rate("eth0", 10, 20);
    snapshot.net_interfaces.push(crate::monitor::NetIfaceInfo {
        name: "wlan0".into(),
        rx_rate: 30,
        tx_rate: 40,
    });
    sidebar.on_monitor_update(&key, &snapshot);
    {
        let view = sidebar.monitor_views.get_mut(&key).unwrap();
        view.selected_iface = Some("wlan0".into());
        view.interface_selection_manual = true;
    }

    snapshot.preferred_net_interface = Some("eth0".into());
    sidebar.on_monitor_update(&key, &snapshot);
    assert_eq!(
        sidebar.monitor_view(&key).selected_iface.as_deref(),
        Some("wlan0")
    );

    snapshot
        .net_interfaces
        .retain(|interface| interface.name != "wlan0");
    sidebar.on_monitor_update(&key, &snapshot);
    assert_eq!(
        sidebar.monitor_view(&key).selected_iface.as_deref(),
        Some("eth0")
    );
    assert!(!sidebar.monitor_view(&key).interface_selection_manual);
}
