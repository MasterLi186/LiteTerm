use super::dialogs::*;
use super::*;

pub fn render(ctx: &egui::Context, state: &mut FileBrowserState) -> Vec<FileBrowserAction> {
    let mut actions = Vec::new();
    let mut pending_menu = None;
    let mut pending_create = None;
    state.prune_completed(Instant::now());
    egui::TopBottomPanel::bottom("file_browser_toggle")
        .exact_height(TOGGLE_HEIGHT)
        .frame(
            egui::Frame::new()
                .fill(HEADER_BG)
                .stroke(egui::Stroke::new(1.0, BORDER))
                .inner_margin(egui::Margin::same(0)),
        )
        .show(ctx, |ui| {
            let label = if state.open {
                "▼ 隐藏文件管理器"
            } else {
                "▲ 显示文件管理器"
            };
            if ui
                .add_sized(
                    ui.available_size(),
                    egui::Button::new(egui::RichText::new(label).size(10.0).color(MUTED))
                        .frame(false),
                )
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
        .exact_height(PANEL_HEIGHT)
        .frame(
            egui::Frame::new()
                .fill(PANEL_BG)
                .stroke(egui::Stroke::new(1.0, BORDER))
                .inner_margin(egui::Margin::same(4)),
        )
        .show(ctx, |ui| {
            for transfer in &state.transfers {
                let percent = if transfer.total == 0 {
                    0.0
                } else {
                    transfer.transferred as f32 / transfer.total as f32
                };
                ui.horizontal(|ui| {
                    ui.label(
                        egui::RichText::new(match transfer.direction {
                            TransferDirection::Upload => "↑",
                            TransferDirection::Download => "↓",
                        })
                        .color(CYAN),
                    );
                    ui.label(
                        egui::RichText::new(&transfer.filename)
                            .size(10.0)
                            .color(TEXT),
                    );
                    ui.add(
                        egui::ProgressBar::new(percent)
                            .desired_width(180.0)
                            .fill(CYAN),
                    );
                    if let Some(error) = &transfer.error {
                        ui.colored_label(RED, error);
                    }
                });
            }
            let local_destination = state.local.path.clone();
            let remote_destination = state.remote.path.clone();
            ui.spacing_mut().item_spacing.x = 1.0;
            ui.columns(2, |columns| {
                columns[0].push_id("local_file_pane", |ui| {
                    render_pane(
                        ui,
                        FileSide::Local,
                        &mut state.local,
                        &remote_destination,
                        true,
                        &mut PaneOutputs {
                            pending_menu: &mut pending_menu,
                            pending_create: &mut pending_create,
                            actions: &mut actions,
                        },
                    );
                });
                columns[1].push_id("remote_file_pane", |ui| {
                    render_pane(
                        ui,
                        FileSide::Remote,
                        &mut state.remote,
                        &local_destination,
                        state.ready,
                        &mut PaneOutputs {
                            pending_menu: &mut pending_menu,
                            pending_create: &mut pending_create,
                            actions: &mut actions,
                        },
                    );
                });
            });
        });
    if let Some(menu) = pending_menu {
        state.open_context_menu(menu);
    }
    if let Some(dialog) = pending_create {
        state.create_dialog = Some(dialog);
    }
    render_context_menu(ctx, state, &mut actions);
    render_rename_dialog(ctx, state, &mut actions);
    render_create_dialog(ctx, state, &mut actions);
    render_delete_dialog(ctx, state, &mut actions);
    actions
}

fn render_pane(
    ui: &mut egui::Ui,
    side: FileSide,
    pane: &mut PaneState,
    destination_path: &str,
    create_enabled: bool,
    output: &mut PaneOutputs<'_>,
) {
    egui::Frame::new()
        .fill(HEADER_BG)
        .inner_margin(egui::Margin::symmetric(8, 3))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                let (title, color) = match side {
                    FileSide::Local => ("本地", MUTED),
                    FileSide::Remote => ("远端", GREEN),
                };
                ui.label(egui::RichText::new(title).size(10.0).strong().color(color));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.spacing_mut().item_spacing.x = 4.0;
                    ui.label(
                        egui::RichText::new(format!(
                            "{} 项",
                            visible_entries(&pane.entries).count()
                        ))
                        .size(9.0)
                        .color(DIM),
                    );
                    if ui
                        .add_enabled(
                            create_enabled,
                            egui::Button::new(
                                egui::RichText::new("＋目录").size(10.0).color(MUTED),
                            )
                            .min_size(egui::vec2(42.0, 18.0))
                            .fill(PANEL_BG)
                            .stroke(egui::Stroke::new(1.0, BORDER)),
                        )
                        .clicked()
                    {
                        *output.pending_create = Some(CreateDialogState {
                            side,
                            parent_path: pane.path.clone(),
                            kind: CreateKind::Directory,
                            value: String::new(),
                            request_focus: true,
                        });
                    }
                    if ui
                        .add_enabled(
                            create_enabled,
                            egui::Button::new(
                                egui::RichText::new("＋文件").size(10.0).color(MUTED),
                            )
                            .min_size(egui::vec2(42.0, 18.0))
                            .fill(PANEL_BG)
                            .stroke(egui::Stroke::new(1.0, BORDER)),
                        )
                        .clicked()
                    {
                        *output.pending_create = Some(CreateDialogState {
                            side,
                            parent_path: pane.path.clone(),
                            kind: CreateKind::File,
                            value: String::new(),
                            request_focus: true,
                        });
                    }
                });
            });
        });

    egui::Frame::new()
        .fill(PANEL_BG)
        .inner_margin(egui::Margin::symmetric(4, 2))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                let controls_width = 48.0;
                let path_width = (ui.available_width() - controls_width).max(80.0);
                let response = ui.add_sized(
                    egui::vec2(path_width, 20.0),
                    egui::TextEdit::singleline(&mut pane.input)
                        .text_color(TEXT)
                        .margin(egui::Margin::symmetric(6, 2)),
                );
                if response.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter)) {
                    output.actions.push(FileBrowserAction::List {
                        side,
                        path: pane.input.clone(),
                    });
                }
                if ui
                    .add_sized(
                        egui::vec2(20.0, 20.0),
                        egui::Button::new(egui::RichText::new("⟳").color(MUTED)).frame(false),
                    )
                    .on_hover_text("刷新")
                    .clicked()
                {
                    output.actions.push(FileBrowserAction::List {
                        side,
                        path: pane.path.clone(),
                    });
                }
                if ui
                    .add_sized(
                        egui::vec2(20.0, 20.0),
                        egui::Button::new(egui::RichText::new("↑").color(MUTED)).frame(false),
                    )
                    .on_hover_text("上级目录")
                    .clicked()
                {
                    output.actions.push(FileBrowserAction::List {
                        side,
                        path: crate::sftp::parent_path(&pane.path),
                    });
                }
            });
        });

    if pane.loading {
        ui.horizontal(|ui| {
            ui.spinner();
            ui.label(egui::RichText::new("加载中…").size(10.0).color(MUTED));
        });
    }
    if let Some(error) = &pane.error {
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(error).size(10.0).color(RED));
            if side == FileSide::Remote && ui.small_button("重新连接").clicked() {
                output.actions.push(FileBrowserAction::Reconnect);
            }
        });
    }

    render_column_header(ui);

    egui::ScrollArea::vertical()
        .id_salt(match side {
            FileSide::Local => "local_file_scroll",
            FileSide::Remote => "remote_file_scroll",
        })
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            for (index, entry) in visible_entries(&pane.entries).enumerate() {
                let selected = pane.selected.as_deref() == Some(entry.name.as_str());
                let response = render_file_row(ui, entry, selected, index);
                if response.clicked() {
                    pane.selected = Some(entry.name.clone());
                }
                if response.secondary_clicked() {
                    pane.selected = Some(entry.name.clone());
                    *output.pending_menu = Some(file_context_menu(
                        side,
                        entry.clone(),
                        &pane.path,
                        destination_path,
                        response
                            .interact_pointer_pos()
                            .unwrap_or(response.rect.left_top()),
                    ));
                }
                if response.double_clicked() {
                    if entry.is_dir {
                        output.actions.push(FileBrowserAction::List {
                            side,
                            path: entry.path.clone(),
                        });
                    } else {
                        match side {
                            FileSide::Local => output.actions.push(FileBrowserAction::Upload {
                                local_path: entry.path.clone(),
                                remote_path: crate::sftp::join_path(destination_path, &entry.name),
                            }),
                            FileSide::Remote => output.actions.push(FileBrowserAction::Download {
                                remote_path: entry.path.clone(),
                                local_path: crate::sftp::join_path(destination_path, &entry.name),
                            }),
                        }
                    }
                }
            }
        });
}

fn render_column_header(ui: &mut egui::Ui) {
    let width = ui.available_width().max(238.0);
    let (rect, _) = ui.allocate_exact_size(egui::vec2(width, ROW_HEIGHT), egui::Sense::hover());
    let columns = file_columns(rect.width());
    let painter = ui.painter();
    painter.rect_filled(rect, 0.0, HEADER_BG);
    painter.line_segment(
        [rect.left_bottom(), rect.right_bottom()],
        egui::Stroke::new(1.0, BORDER),
    );
    let font = egui::FontId::proportional(10.0);
    painter.text(
        egui::pos2(rect.left() + 8.0, rect.center().y),
        egui::Align2::LEFT_CENTER,
        "文件名",
        font.clone(),
        DIM,
    );
    painter.text(
        egui::pos2(
            rect.left() + columns.name + columns.size - 8.0,
            rect.center().y,
        ),
        egui::Align2::RIGHT_CENTER,
        "大小",
        font.clone(),
        DIM,
    );
    painter.text(
        egui::pos2(
            rect.left() + columns.name + columns.size + 8.0,
            rect.center().y,
        ),
        egui::Align2::LEFT_CENTER,
        "修改时间",
        font,
        DIM,
    );
}

fn render_file_row(
    ui: &mut egui::Ui,
    entry: &FileEntry,
    selected: bool,
    index: usize,
) -> egui::Response {
    let width = ui.available_width().max(238.0);
    let (rect, response) =
        ui.allocate_exact_size(egui::vec2(width, ROW_HEIGHT), egui::Sense::click());
    let background = if selected {
        egui::Color32::from_rgba_unmultiplied(0x00, 0xd4, 0xff, 0x1a)
    } else if response.hovered() {
        egui::Color32::from_rgba_unmultiplied(0x00, 0xd4, 0xff, 0x0f)
    } else if index % 2 == 1 {
        egui::Color32::from_rgba_unmultiplied(0xff, 0xff, 0xff, 0x04)
    } else {
        egui::Color32::TRANSPARENT
    };
    let columns = file_columns(rect.width());
    let painter = ui.painter();
    painter.rect_filled(rect, 0.0, background);
    painter.line_segment(
        [rect.left_bottom(), rect.right_bottom()],
        egui::Stroke::new(0.5, BORDER),
    );

    paint_file_icon(
        painter,
        egui::pos2(rect.left() + 13.0, rect.center().y),
        file_icon_kind(entry),
    );
    let name_rect = egui::Rect::from_min_max(
        egui::pos2(rect.left() + 20.0, rect.top()),
        egui::pos2(rect.left() + columns.name - 4.0, rect.bottom()),
    );
    ui.painter().with_clip_rect(name_rect).text(
        name_rect.left_center(),
        egui::Align2::LEFT_CENTER,
        &entry.name,
        egui::FontId::proportional(11.0),
        if selected { CYAN } else { TEXT },
    );
    if !entry.is_dir {
        painter.text(
            egui::pos2(
                rect.left() + columns.name + columns.size - 8.0,
                rect.center().y,
            ),
            egui::Align2::RIGHT_CENTER,
            format_size(entry.size),
            egui::FontId::proportional(10.0),
            MUTED,
        );
    }
    painter.text(
        egui::pos2(
            rect.left() + columns.name + columns.size + 8.0,
            rect.center().y,
        ),
        egui::Align2::LEFT_CENTER,
        format_mtime(entry.mtime),
        egui::FontId::proportional(10.0),
        MUTED,
    );

    response
}

fn paint_file_icon(painter: &egui::Painter, center: egui::Pos2, kind: FileIconKind) {
    match kind {
        FileIconKind::Folder => {
            let left = center.x - 6.0;
            let right = center.x + 6.0;
            let top = center.y - 3.0;
            let bottom = center.y + 4.0;
            let tab_top = center.y - 5.0;
            let tab_right = center.x - 1.0;
            let stroke = egui::Stroke::new(1.0, YELLOW);
            for points in [
                [egui::pos2(left, top), egui::pos2(left, tab_top)],
                [egui::pos2(left, tab_top), egui::pos2(tab_right, tab_top)],
                [
                    egui::pos2(tab_right, tab_top),
                    egui::pos2(tab_right + 2.0, top),
                ],
                [egui::pos2(tab_right + 2.0, top), egui::pos2(right, top)],
                [egui::pos2(right, top), egui::pos2(right, bottom)],
                [egui::pos2(right, bottom), egui::pos2(left, bottom)],
                [egui::pos2(left, bottom), egui::pos2(left, top)],
            ] {
                painter.line_segment(points, stroke);
            }
        }
        FileIconKind::Code => {
            let stroke = egui::Stroke::new(1.2, CYAN);
            for points in [
                [
                    egui::pos2(center.x - 1.0, center.y - 5.0),
                    egui::pos2(center.x - 6.0, center.y),
                ],
                [
                    egui::pos2(center.x - 6.0, center.y),
                    egui::pos2(center.x - 1.0, center.y + 5.0),
                ],
                [
                    egui::pos2(center.x + 1.0, center.y - 5.0),
                    egui::pos2(center.x + 6.0, center.y),
                ],
                [
                    egui::pos2(center.x + 6.0, center.y),
                    egui::pos2(center.x + 1.0, center.y + 5.0),
                ],
            ] {
                painter.line_segment(points, stroke);
            }
        }
        FileIconKind::Text => {
            paint_document_outline(painter, center, MUTED);
            let stroke = egui::Stroke::new(1.0, MUTED);
            painter.line_segment(
                [
                    egui::pos2(center.x - 2.0, center.y),
                    egui::pos2(center.x + 2.0, center.y),
                ],
                stroke,
            );
            painter.line_segment(
                [
                    egui::pos2(center.x - 2.0, center.y + 3.0),
                    egui::pos2(center.x + 2.0, center.y + 3.0),
                ],
                stroke,
            );
        }
        FileIconKind::Image => {
            let stroke = egui::Stroke::new(1.0, GREEN);
            let left = center.x - 6.0;
            let right = center.x + 6.0;
            let top = center.y - 5.0;
            let bottom = center.y + 5.0;
            for points in [
                [egui::pos2(left, top), egui::pos2(right, top)],
                [egui::pos2(right, top), egui::pos2(right, bottom)],
                [egui::pos2(right, bottom), egui::pos2(left, bottom)],
                [egui::pos2(left, bottom), egui::pos2(left, top)],
                [
                    egui::pos2(left + 1.0, bottom - 1.0),
                    egui::pos2(center.x - 1.0, center.y),
                ],
                [
                    egui::pos2(center.x - 1.0, center.y),
                    egui::pos2(center.x + 1.0, center.y + 2.0),
                ],
                [
                    egui::pos2(center.x + 1.0, center.y + 2.0),
                    egui::pos2(right - 1.0, center.y - 2.0),
                ],
            ] {
                painter.line_segment(points, stroke);
            }
            painter.circle_filled(egui::pos2(center.x + 2.5, top + 2.5), 1.0, GREEN);
        }
        FileIconKind::Archive => {
            let stroke = egui::Stroke::new(1.0, YELLOW);
            painter.rect_stroke(
                egui::Rect::from_center_size(center, egui::vec2(11.0, 10.0)),
                1.0,
                stroke,
                egui::StrokeKind::Inside,
            );
            painter.line_segment(
                [
                    egui::pos2(center.x, center.y - 5.0),
                    egui::pos2(center.x, center.y + 5.0),
                ],
                stroke,
            );
            for offset in [-3.0, 0.0, 3.0] {
                painter.line_segment(
                    [
                        egui::pos2(center.x, center.y + offset),
                        egui::pos2(center.x + 2.0, center.y + offset),
                    ],
                    stroke,
                );
            }
        }
        FileIconKind::Binary => {
            let stroke = egui::Stroke::new(1.0, RED);
            let points = vec![
                egui::pos2(center.x, center.y - 6.0),
                egui::pos2(center.x + 5.0, center.y - 3.0),
                egui::pos2(center.x + 5.0, center.y + 3.0),
                egui::pos2(center.x, center.y + 6.0),
                egui::pos2(center.x - 5.0, center.y + 3.0),
                egui::pos2(center.x - 5.0, center.y - 3.0),
            ];
            painter.add(egui::Shape::closed_line(points, stroke));
            painter.circle_filled(center, 1.5, RED);
        }
        FileIconKind::File => {
            paint_document_outline(painter, center, DIM);
        }
    }
}

fn paint_document_outline(painter: &egui::Painter, center: egui::Pos2, color: egui::Color32) {
    let left = center.x - 4.0;
    let right = center.x + 4.0;
    let top = center.y - 6.0;
    let bottom = center.y + 6.0;
    let fold = 3.0;
    let stroke = egui::Stroke::new(1.0, color);
    for points in [
        [egui::pos2(left, top), egui::pos2(right - fold, top)],
        [egui::pos2(right - fold, top), egui::pos2(right, top + fold)],
        [egui::pos2(right, top + fold), egui::pos2(right, bottom)],
        [egui::pos2(right, bottom), egui::pos2(left, bottom)],
        [egui::pos2(left, bottom), egui::pos2(left, top)],
        [
            egui::pos2(right - fold, top),
            egui::pos2(right - fold, top + fold),
        ],
        [
            egui::pos2(right - fold, top + fold),
            egui::pos2(right, top + fold),
        ],
    ] {
        painter.line_segment(points, stroke);
    }
}
