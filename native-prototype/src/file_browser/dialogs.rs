use super::*;

pub(super) fn render_context_menu(
    ctx: &egui::Context,
    state: &mut FileBrowserState,
    actions: &mut Vec<FileBrowserAction>,
) {
    let Some(menu) = state.context_menu.clone() else {
        return;
    };
    let menu_size = egui::vec2(160.0, 90.0);
    let screen = ctx.input(|input| input.screen_rect);
    let position = popup_position(menu.pointer, menu_size, screen);
    let items = context_menu_items(
        menu.side,
        &menu.entry,
        &state.local.path,
        &state.remote.path,
        state.ready,
    );
    let mut selected = None;
    let area = egui::Area::new(egui::Id::new("file_context_menu"))
        .order(egui::Order::Foreground)
        .fixed_pos(position)
        .show(ctx, |ui| {
            egui::Frame::new()
                .fill(HEADER_BG)
                .stroke(egui::Stroke::new(1.0, BORDER))
                .corner_radius(4.0)
                .inner_margin(egui::Margin::symmetric(4, 4))
                .show(ui, |ui| {
                    ui.set_min_width(152.0);
                    ui.spacing_mut().item_spacing.y = 1.0;
                    for item in items {
                        if item.separator_before {
                            ui.separator();
                        }
                        let (rect, response) =
                            ui.allocate_exact_size(egui::vec2(152.0, 22.0), egui::Sense::click());
                        ui.painter().rect_filled(
                            rect,
                            3.0,
                            context_item_fill(response.hovered(), item.enabled),
                        );
                        let color = if !item.enabled {
                            DIM
                        } else if item.command == ContextCommand::Delete {
                            RED
                        } else {
                            TEXT
                        };
                        ui.painter().text(
                            rect.left_center() + egui::vec2(8.0, 0.0),
                            egui::Align2::LEFT_CENTER,
                            &item.label,
                            egui::FontId::proportional(11.0),
                            color,
                        );
                        if item.enabled && response.clicked() {
                            selected = Some(item.command);
                        }
                    }
                });
        });

    if let Some(command) = selected {
        match context_action(&menu, command) {
            ContextOutcome::Action(action) => actions.push(action),
            ContextOutcome::Rename(dialog) => state.rename_dialog = Some(dialog),
            ContextOutcome::Delete(dialog) => state.delete_dialog = Some(dialog),
        }
        state.context_menu = None;
        return;
    }

    let close = ctx.input(|input| {
        input.key_pressed(egui::Key::Escape)
            || input.pointer.primary_clicked()
                && input
                    .pointer
                    .interact_pos()
                    .is_some_and(|pointer| !area.response.rect.contains(pointer))
    });
    if close {
        state.context_menu = None;
    }
}

pub(super) fn render_rename_dialog(
    ctx: &egui::Context,
    state: &mut FileBrowserState,
    actions: &mut Vec<FileBrowserAction>,
) {
    let Some(mut dialog) = state.rename_dialog.take() else {
        return;
    };
    let mut cancel = false;
    let mut submit = false;
    let just_opened = dialog.request_focus;
    let screen = ctx.input(|input| input.screen_rect);

    egui::Area::new(egui::Id::new("file_rename_backdrop"))
        .order(egui::Order::Foreground)
        .fixed_pos(screen.min)
        .sense(egui::Sense::click())
        .show(ctx, |ui| {
            let (rect, _) = ui.allocate_exact_size(screen.size(), egui::Sense::click());
            ui.painter().rect_filled(
                rect,
                0.0,
                egui::Color32::from_rgba_unmultiplied(0, 0, 0, 100),
            );
        });

    let window = egui::Window::new("重命名")
        .id(egui::Id::new("file_rename_dialog"))
        .order(egui::Order::Foreground)
        .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
        .fixed_size(egui::vec2(320.0, 104.0))
        .collapsible(false)
        .resizable(false)
        .title_bar(true)
        .show(ctx, |ui| {
            ui.add_space(4.0);
            let response = ui.add_sized(
                egui::vec2(ui.available_width(), 24.0),
                egui::TextEdit::singleline(&mut dialog.value)
                    .text_color(TEXT)
                    .margin(egui::Margin::symmetric(7, 3)),
            );
            if dialog.request_focus {
                response.request_focus();
                if let Some(mut edit_state) = egui::TextEdit::load_state(ui.ctx(), response.id) {
                    edit_state
                        .cursor
                        .set_char_range(Some(egui::text::CCursorRange::two(
                            egui::text::CCursor::new(0),
                            egui::text::CCursor::new(dialog.value.chars().count()),
                        )));
                    egui::TextEdit::store_state(ui.ctx(), response.id, edit_state);
                }
                dialog.request_focus = false;
            }
            if response.lost_focus()
                && ui.input(|input| input.key_pressed(egui::Key::Enter))
                && !dialog.value.trim().is_empty()
            {
                submit = true;
            }
            ui.add_space(6.0);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let valid = !dialog.value.trim().is_empty();
                if ui.add_enabled(valid, egui::Button::new("确定")).clicked() {
                    submit = true;
                }
                if ui.button("取消").clicked() {
                    cancel = true;
                }
            });
        });

    let cancel_from_input = ctx.input(|input| {
        should_cancel_rename(
            just_opened,
            input.key_pressed(egui::Key::Escape),
            input.pointer.primary_clicked(),
            input.pointer.interact_pos(),
            window.as_ref().map(|window| window.response.rect),
        )
    });
    if cancel_from_input {
        cancel = true;
    }
    if submit {
        if let Some(action) = rename_action(
            dialog.side,
            &dialog.old_path,
            &dialog.parent_path,
            &dialog.value,
        ) {
            actions.push(action);
        } else {
            state.rename_dialog = Some(dialog);
        }
    } else if !cancel {
        state.rename_dialog = Some(dialog);
    }
}

pub(super) fn should_cancel_rename(
    just_opened: bool,
    escape_pressed: bool,
    primary_clicked: bool,
    pointer: Option<egui::Pos2>,
    window: Option<egui::Rect>,
) -> bool {
    escape_pressed
        || !just_opened
            && primary_clicked
            && pointer.is_some_and(|pointer| window.is_none_or(|window| !window.contains(pointer)))
}

pub(super) fn render_create_dialog(
    ctx: &egui::Context,
    state: &mut FileBrowserState,
    actions: &mut Vec<FileBrowserAction>,
) {
    let Some(mut dialog) = state.create_dialog.take() else {
        return;
    };
    let mut cancel = false;
    let mut submit = false;
    let just_opened = dialog.request_focus;
    let screen = ctx.input(|input| input.screen_rect);

    egui::Area::new(egui::Id::new("file_create_backdrop"))
        .order(egui::Order::Foreground)
        .fixed_pos(screen.min)
        .sense(egui::Sense::click())
        .show(ctx, |ui| {
            let (rect, _) = ui.allocate_exact_size(screen.size(), egui::Sense::click());
            ui.painter().rect_filled(
                rect,
                0.0,
                egui::Color32::from_rgba_unmultiplied(0, 0, 0, 100),
            );
        });

    let title = match dialog.kind {
        CreateKind::File => "新建文件",
        CreateKind::Directory => "新建目录",
    };
    let window = egui::Window::new(title)
        .id(egui::Id::new("file_create_dialog"))
        .order(egui::Order::Foreground)
        .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
        .fixed_size(egui::vec2(340.0, 112.0))
        .collapsible(false)
        .resizable(false)
        .title_bar(true)
        .show(ctx, |ui| {
            ui.add_space(4.0);
            let response = ui.add_sized(
                egui::vec2(ui.available_width(), 24.0),
                egui::TextEdit::singleline(&mut dialog.value)
                    .text_color(TEXT)
                    .margin(egui::Margin::symmetric(7, 3)),
            );
            if dialog.request_focus {
                response.request_focus();
                dialog.request_focus = false;
            }
            let valid = create_action(dialog.side, &dialog.parent_path, dialog.kind, &dialog.value)
                .is_some();
            if response.lost_focus()
                && ui.input(|input| input.key_pressed(egui::Key::Enter))
                && valid
            {
                submit = true;
            }
            ui.add_space(6.0);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.add_enabled(valid, egui::Button::new("创建")).clicked() {
                    submit = true;
                }
                if ui.button("取消").clicked() {
                    cancel = true;
                }
            });
        });

    if ctx.input(|input| {
        should_cancel_rename(
            just_opened,
            input.key_pressed(egui::Key::Escape),
            input.pointer.primary_clicked(),
            input.pointer.interact_pos(),
            window.as_ref().map(|window| window.response.rect),
        )
    }) {
        cancel = true;
    }

    if submit {
        if let Some(action) =
            create_action(dialog.side, &dialog.parent_path, dialog.kind, &dialog.value)
        {
            actions.push(action);
        } else {
            state.create_dialog = Some(dialog);
        }
    } else if !cancel {
        state.create_dialog = Some(dialog);
    }
}

pub(super) const DELETE_DIALOG_MIN_WIDTH: f32 = 260.0;
pub(super) const DELETE_DIALOG_MAX_WIDTH: f32 = 520.0;
pub(super) const DELETE_DIALOG_HORIZONTAL_CHROME: f32 = 44.0;

pub(super) fn delete_dialog_width(ctx: &egui::Context, name: &str) -> f32 {
    let message = format!("确定要删除“{name}”吗？");
    let text_width = ctx.fonts(|fonts| {
        fonts
            .layout_no_wrap(message, egui::FontId::proportional(12.0), TEXT)
            .size()
            .x
    });
    (text_width + DELETE_DIALOG_HORIZONTAL_CHROME)
        .clamp(DELETE_DIALOG_MIN_WIDTH, DELETE_DIALOG_MAX_WIDTH)
}

pub(super) fn render_delete_dialog(
    ctx: &egui::Context,
    state: &mut FileBrowserState,
    actions: &mut Vec<FileBrowserAction>,
) -> Option<egui::Rect> {
    let mut dialog = state.delete_dialog.take()?;
    let just_opened = dialog.just_opened;
    dialog.just_opened = false;
    let mut cancel = false;
    let mut submit = false;
    let screen = ctx.input(|input| input.screen_rect);
    let dialog_width = delete_dialog_width(ctx, &dialog.name);
    let window_horizontal_chrome = egui::Frame::window(ctx.style().as_ref())
        .total_margin()
        .sum()
        .x;
    let dialog_content_width = dialog_width - window_horizontal_chrome;
    let side_tag = match dialog.side {
        FileSide::Local => "local",
        FileSide::Remote => "remote",
    };
    let window_id = egui::Id::new(("file_delete_dialog", side_tag, dialog.path.as_str()));

    egui::Area::new(egui::Id::new("file_delete_backdrop"))
        .order(egui::Order::Foreground)
        .fixed_pos(screen.min)
        .sense(egui::Sense::click())
        .show(ctx, |ui| {
            let (rect, _) = ui.allocate_exact_size(screen.size(), egui::Sense::click());
            ui.painter().rect_filled(
                rect,
                0.0,
                egui::Color32::from_rgba_unmultiplied(0, 0, 0, 100),
            );
        });

    let window = egui::Window::new("确认删除")
        .id(window_id)
        .order(egui::Order::Foreground)
        .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
        .default_width(dialog_content_width)
        .default_height(0.0)
        .min_width(dialog_content_width)
        .max_width(dialog_content_width)
        .collapsible(false)
        .resizable(false)
        .title_bar(true)
        .show(ctx, |ui| {
            ui.add(
                egui::Label::new(
                    egui::RichText::new(format!("确定要删除“{}”吗？", dialog.name))
                        .size(12.0)
                        .color(TEXT),
                )
                .wrap(),
            );
            ui.add_space(4.0);
            ui.add(
                egui::Label::new(egui::RichText::new(&dialog.path).size(10.0).color(MUTED)).wrap(),
            );
            ui.add_space(8.0);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .add(
                        egui::Button::new(egui::RichText::new("删除").color(RED)).fill(
                            egui::Color32::from_rgba_unmultiplied(RED.r(), RED.g(), RED.b(), 32),
                        ),
                    )
                    .clicked()
                {
                    submit = true;
                }
                if ui.button("取消").clicked() {
                    cancel = true;
                }
            });
        });
    let window_rect = window.as_ref().map(|window| window.response.rect);

    if ctx.input(|input| {
        should_cancel_rename(
            just_opened,
            input.key_pressed(egui::Key::Escape),
            input.pointer.primary_clicked(),
            input.pointer.interact_pos(),
            window.as_ref().map(|window| window.response.rect),
        )
    }) {
        cancel = true;
    }

    if submit {
        actions.push(delete_action(&dialog));
    } else if !cancel {
        state.delete_dialog = Some(dialog);
    }

    window_rect
}
