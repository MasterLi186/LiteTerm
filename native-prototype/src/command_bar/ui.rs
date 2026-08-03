use super::*;

impl CommandBar {
    /// 返回要发送到终端的命令（已带 `\n`）。
    /// `sidebar_width`：添加/编辑弹窗仍使用该值保持在终端内容区域内。
    pub fn ui(&mut self, ctx: &egui::Context, sidebar_width: f32) -> Option<String> {
        let mut clicked_cmd: Option<String> = None;
        let btn_color = egui::Color32::from_rgb(0xc9, 0xd1, 0xd9);
        let btn_bg = egui::Color32::from_rgb(0x16, 0x1b, 0x22);
        let btn_border = egui::Stroke::new(1.0, egui::Color32::from_rgb(0x30, 0x36, 0x3d));
        let muted = egui::Color32::from_rgb(0x8b, 0x94, 0x9e);
        let legacy_popup_left = sidebar_width + 8.0;
        let mut history_opened_this_frame = false;

        // 先画 SidePanel 再画 BottomPanel 时，底栏区域已在侧边栏右侧
        egui::TopBottomPanel::bottom("command_bar")
            .exact_height(COMMAND_BAR_HEIGHT)
            .frame(
                egui::Frame::new()
                    .fill(egui::Color32::from_rgb(0x0d, 0x11, 0x17))
                    .inner_margin(egui::Margin::symmetric(4, 2))
                    .stroke(egui::Stroke::new(
                        1.0,
                        egui::Color32::from_rgb(0x21, 0x26, 0x2d),
                    )),
            )
            .show(ctx, |ui| {
                // 第一行：+ 按钮 + 快捷命令按钮
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 3.0;

                    let add_btn = ui
                        .add(
                            egui::Button::new(egui::RichText::new("+").size(11.0).color(muted))
                                .fill(btn_bg)
                                .stroke(btn_border)
                                .corner_radius(3.0)
                                .min_size(egui::vec2(20.0, 20.0)),
                        )
                        .on_hover_text("添加快捷命令");
                    if add_btn.clicked() {
                        self.edit_index = None;
                        self.add_label.clear();
                        self.add_command.clear();
                        self.add_error.clear();
                        self.show_add = true;
                        self.show_history = false;
                        self.show_favorites = false;
                    }

                    let mut edit_req: Option<usize> = None;
                    let mut delete_req: Option<usize> = None;
                    let mut fav_req: Option<usize> = None;
                    let cmd_snapshot: Vec<(String, String)> = self
                        .commands
                        .iter()
                        .map(|c| (c.label.clone(), c.command.clone()))
                        .collect();
                    for (i, (label, command)) in cmd_snapshot.iter().enumerate() {
                        let btn = ui
                            .add(
                                egui::Button::new(
                                    egui::RichText::new(label).size(10.0).color(btn_color),
                                )
                                .fill(btn_bg)
                                .stroke(btn_border)
                                .corner_radius(3.0)
                                .min_size(egui::vec2(0.0, 20.0)),
                            )
                            .on_hover_text(command);
                        if btn.clicked() {
                            clicked_cmd = Some(format!("{}\n", command.trim_end_matches('\n')));
                        }
                        btn.context_menu(|ui| {
                            if ui.button("编辑").clicked() {
                                edit_req = Some(i);
                                ui.close_menu();
                            }
                            if ui.button("删除").clicked() {
                                delete_req = Some(i);
                                ui.close_menu();
                            }
                            if ui.button("加入收藏").clicked() {
                                fav_req = Some(i);
                                ui.close_menu();
                            }
                        });
                    }
                    if let Some(i) = edit_req {
                        if let Some(cmd) = self.commands.get(i) {
                            self.edit_index = Some(i);
                            self.add_label = cmd.label.clone();
                            self.add_command = cmd.command.clone();
                            self.add_error.clear();
                            self.show_add = true;
                        }
                    }
                    if let Some(i) = delete_req {
                        if i < self.commands.len() {
                            self.commands.remove(i);
                            self.save_commands();
                        }
                    }
                    if let Some(i) = fav_req {
                        if let Some(cmd) = self.commands.get(i).cloned() {
                            if !self.favorites.iter().any(|f| f.command == cmd.command) {
                                self.favorites.insert(
                                    0,
                                    QuickCommand {
                                        label: cmd.label,
                                        command: cmd.command,
                                        system: false,
                                    },
                                );
                                self.save_favorites();
                            }
                        }
                    }
                });

                // 第二行：命令输入框 + 历史 + 收藏
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 4.0;

                    ui.label(egui::RichText::new("命令输入:").size(10.0).color(muted));

                    let input_resp = ui.add(
                        egui::TextEdit::singleline(&mut self.input_text)
                            .hint_text("输入命令后回车发送到终端")
                            .desired_width((ui.available_width() - 54.0).max(80.0))
                            .font(egui::FontId::proportional(11.0))
                            .id_salt("cmd_bar_input"),
                    );
                    let enter = ui.input(|i| i.key_pressed(egui::Key::Enter));
                    if enter
                        && (input_resp.has_focus() || input_resp.lost_focus())
                        && !self.input_text.trim().is_empty()
                    {
                        let cmd = self.input_text.trim().to_string();
                        self.push_history(&cmd);
                        clicked_cmd = Some(format!("{}\n", cmd));
                        self.input_text.clear();
                    }

                    let hist_btn = ui
                        .add(
                            egui::Button::new(egui::RichText::new("⏱").size(12.0))
                                .frame(false)
                                .min_size(egui::vec2(20.0, 18.0)),
                        )
                        .on_hover_text("历史记录");
                    self.last_history_button_rect = Some(hist_btn.rect);
                    if hist_btn.clicked() {
                        let opening = !self.show_history;
                        self.show_history = opening;
                        history_opened_this_frame = opening;
                        self.show_favorites = false;
                        self.show_add = false;
                    }

                    let fav_btn = ui
                        .add(
                            egui::Button::new(egui::RichText::new("★").size(12.0))
                                .frame(false)
                                .min_size(egui::vec2(20.0, 18.0)),
                        )
                        .on_hover_text("收藏");
                    self.last_favorites_button_rect = Some(fav_btn.rect);
                    if fav_btn.clicked() {
                        self.show_favorites = !self.show_favorites;
                        self.show_history = false;
                        self.show_add = false;
                    }
                });
            });

        // ── 历史记录弹出 ──
        if self.show_history {
            let screen = ctx.input(|input| input.screen_rect);
            let popup_size = history_popup_size(self.history.len(), screen);
            let fallback_button = egui::Rect::from_min_size(
                egui::pos2(
                    screen.right() - HISTORY_POPUP_MARGIN,
                    screen.bottom() - COMMAND_BAR_HEIGHT,
                ),
                egui::Vec2::ZERO,
            );
            let popup_position = history_popup_position(
                self.last_history_button_rect.unwrap_or(fallback_button),
                popup_size,
                screen,
            );
            let (backdrop_layer, popup_layer) = history_popup_layer_ids();
            ctx.set_sublayer(backdrop_layer, popup_layer);

            let backdrop_clicked = egui::Area::new(backdrop_layer.id)
                .order(backdrop_layer.order)
                .fixed_pos(screen.min)
                .sense(egui::Sense::hover())
                .show(ctx, |ui| {
                    let (_, response) = ui.allocate_exact_size(screen.size(), egui::Sense::click());
                    response.clicked()
                })
                .inner;

            let mut clear_history = false;
            let mut fill_input: Option<String> = None;
            let mut execute_cmd: Option<String> = None;
            let mut toggle_favorite: Option<String> = None;
            let mut delete_history: Option<String> = None;
            let history_snapshot = self.history.clone();
            let favorite_commands: Vec<String> = self
                .favorites
                .iter()
                .map(|favorite| favorite.command.clone())
                .collect();
            let popup_stroke = if popup_size.x >= 2.0 && popup_size.y >= 2.0 {
                egui::Stroke::new(1.0, egui::Color32::from_rgb(0x30, 0x36, 0x3d))
            } else {
                egui::Stroke::NONE
            };
            let popup_frame = egui::Frame::new()
                .fill(egui::Color32::from_rgb(0x16, 0x1b, 0x22))
                .stroke(popup_stroke)
                .corner_radius(6.0)
                .inner_margin(egui::Margin::same(0));
            let frame_margin = popup_frame.total_margin().sum();
            let popup_content_size = egui::vec2(
                (popup_size.x - frame_margin.x).max(0.0),
                (popup_size.y - frame_margin.y).max(0.0),
            );

            egui::Window::new("command_history_popup")
                .id(popup_layer.id)
                .order(popup_layer.order)
                .title_bar(false)
                .collapsible(false)
                .resizable(false)
                .fixed_pos(popup_position)
                .fixed_size(popup_content_size)
                .frame(popup_frame)
                .show(ctx, |ui| {
                    let header_height = 32.0_f32.min(ui.available_height().max(0.0));
                    let (header_rect, _) = ui.allocate_exact_size(
                        egui::vec2(ui.available_width().max(0.0), header_height),
                        egui::Sense::hover(),
                    );
                    let show_clear = header_rect.width() >= 88.0 && header_rect.height() >= 16.0;
                    let clear_rect = egui::Rect::from_center_size(
                        egui::pos2(header_rect.right() - 22.0, header_rect.center().y),
                        egui::vec2(36.0, header_rect.height()),
                    )
                    .intersect(header_rect);
                    let title_right = if show_clear {
                        clear_rect.left() - 2.0
                    } else {
                        header_rect.right() - 4.0
                    };
                    let title_rect = egui::Rect::from_min_max(
                        egui::pos2(
                            (header_rect.left() + 10.0).min(title_right),
                            header_rect.top(),
                        ),
                        egui::pos2(title_right, header_rect.bottom()),
                    );
                    if title_rect.is_positive() {
                        ui.painter().with_clip_rect(title_rect).text(
                            title_rect.left_center(),
                            egui::Align2::LEFT_CENTER,
                            format!("命令历史（{}）", history_snapshot.len()),
                            egui::FontId::proportional(11.0),
                            muted,
                        );
                    }
                    if show_clear {
                        let clear_response = ui
                            .interact(
                                clear_rect,
                                egui::Id::new("command_history_clear"),
                                egui::Sense::click(),
                            )
                            .on_hover_text("清空历史");
                        let clear_color =
                            if clear_response.hovered() && !history_snapshot.is_empty() {
                                egui::Color32::from_rgb(0xf8, 0x51, 0x49)
                            } else {
                                muted
                            };
                        ui.painter().text(
                            clear_rect.center(),
                            egui::Align2::CENTER_CENTER,
                            "清空",
                            egui::FontId::proportional(10.0),
                            clear_color,
                        );
                        if !history_snapshot.is_empty() && clear_response.clicked() {
                            clear_history = true;
                        }
                    }
                    ui.painter().line_segment(
                        [header_rect.left_bottom(), header_rect.right_bottom()],
                        egui::Stroke::new(1.0, egui::Color32::from_rgb(0x30, 0x36, 0x3d)),
                    );

                    if history_snapshot.is_empty() {
                        let (empty_rect, _) = ui.allocate_exact_size(
                            egui::vec2(
                                ui.available_width().max(0.0),
                                ui.available_height().max(0.0),
                            ),
                            egui::Sense::hover(),
                        );
                        if empty_rect.is_positive() {
                            ui.painter().with_clip_rect(empty_rect).text(
                                empty_rect.center(),
                                egui::Align2::CENTER_CENTER,
                                "暂无历史记录",
                                egui::FontId::proportional(11.0),
                                muted,
                            );
                        }
                    } else if ui.available_height() > 0.0 {
                        egui::ScrollArea::vertical()
                            .auto_shrink([false, false])
                            .min_scrolled_height(0.0)
                            .max_height(ui.available_height())
                            .show(ui, |ui| {
                                ui.spacing_mut().item_spacing.y = 0.0;
                                for (index, command) in history_snapshot.iter().enumerate() {
                                    let is_favorite =
                                        favorite_commands.iter().any(|item| item == command);
                                    let (row_rect, _) = ui.allocate_exact_size(
                                        egui::vec2(
                                            ui.available_width().max(0.0),
                                            HISTORY_POPUP_ROW_HEIGHT,
                                        ),
                                        egui::Sense::hover(),
                                    );
                                    let row_response = ui.interact(
                                        row_rect,
                                        history_popup_row_id(index),
                                        egui::Sense::hover(),
                                    );
                                    if row_response.hovered() {
                                        ui.painter().rect_filled(
                                            row_rect,
                                            0.0,
                                            egui::Color32::from_rgb(0x21, 0x26, 0x2d),
                                        );
                                    }

                                    let show_actions =
                                        row_rect.width() >= HISTORY_POPUP_ACTIONS_MIN_ROW_WIDTH;
                                    let actions_width = if show_actions {
                                        HISTORY_POPUP_ACTIONS_WIDTH
                                    } else {
                                        HISTORY_POPUP_ACTION_TRAILING_PADDING
                                    };
                                    let command_left =
                                        (row_rect.left() + 10.0).min(row_rect.right());
                                    let command_right =
                                        (row_rect.right() - actions_width).max(command_left);
                                    let command_rect = egui::Rect::from_min_max(
                                        egui::pos2(command_left, row_rect.top()),
                                        egui::pos2(command_right, row_rect.bottom()),
                                    );
                                    if command_rect.is_positive() {
                                        let command_response = ui
                                            .interact(
                                                command_rect,
                                                history_popup_row_id(index).with("command"),
                                                egui::Sense::click(),
                                            )
                                            .on_hover_text("点击填入输入框");
                                        if command_response.clicked() {
                                            fill_input = Some(command.clone());
                                        }
                                        ui.painter().with_clip_rect(command_rect).text(
                                            command_rect.left_center(),
                                            egui::Align2::LEFT_CENTER,
                                            command,
                                            egui::FontId::monospace(11.0),
                                            btn_color,
                                        );
                                    }

                                    if show_actions {
                                        let actions_left = row_rect.right() - actions_width;
                                        let action_rect = |slot: usize| {
                                            egui::Rect::from_center_size(
                                                egui::pos2(
                                                    actions_left
                                                        + HISTORY_POPUP_ACTION_SLOT_WIDTH / 2.0
                                                        + slot as f32
                                                            * HISTORY_POPUP_ACTION_SLOT_WIDTH,
                                                    row_rect.center().y,
                                                ),
                                                egui::vec2(
                                                    HISTORY_POPUP_ACTION_SLOT_WIDTH,
                                                    HISTORY_POPUP_ACTION_SLOT_WIDTH,
                                                ),
                                            )
                                        };

                                        let execute_rect = action_rect(0);
                                        let execute_response = ui
                                            .interact(
                                                execute_rect,
                                                history_popup_row_id(index).with("execute"),
                                                egui::Sense::click(),
                                            )
                                            .on_hover_text("立即执行");
                                        ui.painter().text(
                                            execute_rect.center(),
                                            egui::Align2::CENTER_CENTER,
                                            "▶",
                                            egui::FontId::proportional(11.0),
                                            egui::Color32::from_rgb(0x3f, 0xb9, 0x50),
                                        );
                                        if execute_response.clicked() {
                                            execute_cmd = Some(command.clone());
                                        }

                                        let copy_rect = action_rect(1);
                                        let copy_response = ui
                                            .interact(
                                                copy_rect,
                                                history_popup_row_id(index).with("copy"),
                                                egui::Sense::click(),
                                            )
                                            .on_hover_text("复制");
                                        ui.painter().text(
                                            copy_rect.center(),
                                            egui::Align2::CENTER_CENTER,
                                            "⧉",
                                            egui::FontId::proportional(12.0),
                                            egui::Color32::from_rgb(0x58, 0xa6, 0xff),
                                        );
                                        if copy_response.clicked() {
                                            ctx.copy_text(command.clone());
                                        }

                                        let favorite_rect = action_rect(2);
                                        let favorite_response = ui
                                            .interact(
                                                favorite_rect,
                                                history_popup_row_id(index).with("favorite"),
                                                egui::Sense::click(),
                                            )
                                            .on_hover_text(if is_favorite {
                                                "取消收藏"
                                            } else {
                                                "收藏"
                                            });
                                        ui.painter().text(
                                            favorite_rect.center(),
                                            egui::Align2::CENTER_CENTER,
                                            if is_favorite { "★" } else { "☆" },
                                            egui::FontId::proportional(12.0),
                                            if is_favorite {
                                                egui::Color32::from_rgb(0xe3, 0xb3, 0x41)
                                            } else {
                                                muted
                                            },
                                        );
                                        if favorite_response.clicked() {
                                            toggle_favorite = Some(command.clone());
                                        }

                                        let delete_rect = action_rect(3);
                                        let delete_response = ui
                                            .interact(
                                                delete_rect,
                                                history_popup_row_id(index).with("delete"),
                                                egui::Sense::click(),
                                            )
                                            .on_hover_text("删除");
                                        ui.painter().text(
                                            delete_rect.center(),
                                            egui::Align2::CENTER_CENTER,
                                            "×",
                                            egui::FontId::proportional(13.0),
                                            egui::Color32::from_rgb(0xf8, 0x51, 0x49),
                                        );
                                        if delete_response.clicked() {
                                            delete_history = Some(command.clone());
                                        }
                                    }

                                    ui.painter().line_segment(
                                        [row_rect.left_bottom(), row_rect.right_bottom()],
                                        egui::Stroke::new(
                                            0.5,
                                            egui::Color32::from_rgba_unmultiplied(
                                                0x30, 0x36, 0x3d, 120,
                                            ),
                                        ),
                                    );
                                }
                            });
                    }
                });

            if clear_history {
                self.history.clear();
                self.save_history();
            }
            if let Some(cmd) = fill_input {
                self.input_text = cmd;
            }
            if let Some(cmd) = toggle_favorite {
                if let Some(index) = self
                    .favorites
                    .iter()
                    .position(|favorite| favorite.command == cmd)
                {
                    self.favorites.remove(index);
                } else {
                    let label: String = cmd.chars().take(16).collect();
                    let label = if cmd.chars().count() > 16 {
                        format!("{label}…")
                    } else {
                        label
                    };
                    self.favorites.insert(
                        0,
                        QuickCommand {
                            label,
                            command: cmd,
                            system: false,
                        },
                    );
                }
                self.save_favorites();
            }
            if let Some(cmd) = delete_history {
                self.history.retain(|history| history != &cmd);
                self.save_history();
            }
            if let Some(cmd) = execute_cmd {
                clicked_cmd = Some(format!("{}\n", cmd));
                self.show_history = false;
            }

            let escape_pressed = ctx.input(|input| input.key_pressed(egui::Key::Escape));
            if escape_pressed || backdrop_clicked && !history_opened_this_frame {
                self.show_history = false;
            }
        }

        // ── 收藏弹出 ──
        if self.show_favorites {
            let mut close = false;
            let mut pick: Option<String> = None;
            let mut remove_idx: Option<usize> = None;
            let screen = ctx.input(|input| input.screen_rect);
            let popup_size = favorites_popup_size(self.favorites.len(), screen);
            let fallback_button = egui::Rect::from_min_size(
                egui::pos2(
                    screen.right() - HISTORY_POPUP_MARGIN,
                    screen.bottom() - COMMAND_BAR_HEIGHT,
                ),
                egui::Vec2::ZERO,
            );
            let popup_position = above_button(
                self.last_favorites_button_rect.unwrap_or(fallback_button),
                popup_size,
                screen,
            );
            let popup_frame = egui::Frame::new()
                .fill(egui::Color32::from_rgb(0x16, 0x1b, 0x22))
                .stroke(egui::Stroke::new(
                    1.0,
                    egui::Color32::from_rgb(0x30, 0x36, 0x3d),
                ))
                .corner_radius(6.0)
                .inner_margin(egui::Margin::same(8));
            let frame_margin = popup_frame.total_margin().sum();
            let popup_content_size = egui::vec2(
                (popup_size.x - frame_margin.x).max(0.0),
                (popup_size.y - frame_margin.y).max(0.0),
            );
            egui::Window::new("命令收藏")
                .id(egui::Id::new("command_favorites_popup"))
                .title_bar(false)
                .collapsible(false)
                .fixed_pos(popup_position)
                .fixed_size(popup_content_size)
                .constrain_to(screen)
                .frame(popup_frame)
                .show(ctx, |ui| {
                    let header_height = 28.0_f32.min(ui.available_height().max(0.0));
                    let (header_rect, _) = ui.allocate_exact_size(
                        egui::vec2(ui.available_width().max(0.0), header_height),
                        egui::Sense::hover(),
                    );
                    if header_rect.is_positive() {
                        ui.painter().text(
                            header_rect.center(),
                            egui::Align2::CENTER_CENTER,
                            "命令收藏",
                            egui::FontId::proportional(13.0),
                            muted,
                        );
                        ui.painter().line_segment(
                            [header_rect.left_bottom(), header_rect.right_bottom()],
                            egui::Stroke::new(1.0, egui::Color32::from_rgb(0x30, 0x36, 0x3d)),
                        );
                    }

                    if self.favorites.is_empty() {
                        ui.label(
                            egui::RichText::new("暂无收藏。可在历史或快捷命令右键加入。")
                                .size(11.0)
                                .color(muted),
                        );
                    } else {
                        let list_height = (ui.available_height() - 28.0).max(0.0);
                        egui::ScrollArea::vertical()
                            .max_height(list_height)
                            .show(ui, |ui| {
                                for (i, fav) in self.favorites.iter().enumerate() {
                                    ui.horizontal(|ui| {
                                        let label = favorite_display_text(&fav.label, &fav.command);
                                        let resp =
                                            ui.add(
                                                egui::Button::new(
                                                    egui::RichText::new(label)
                                                        .size(11.0)
                                                        .color(btn_color),
                                                )
                                                .fill(egui::Color32::TRANSPARENT)
                                                .stroke(egui::Stroke::NONE)
                                                .min_size(egui::vec2(
                                                    (ui.available_width() - 28.0).max(0.0),
                                                    20.0,
                                                )),
                                            );
                                        if resp.clicked() {
                                            pick = Some(fav.command.clone());
                                        }
                                        if ui.small_button("×").on_hover_text("移除收藏").clicked()
                                        {
                                            remove_idx = Some(i);
                                        }
                                    });
                                }
                            });
                    }
                    ui.add_space(4.0);
                    if ui.button("关闭").clicked() {
                        close = true;
                    }
                });
            if let Some(i) = remove_idx {
                if i < self.favorites.len() {
                    self.favorites.remove(i);
                    self.save_favorites();
                }
            }
            if let Some(cmd) = pick {
                self.push_history(&cmd);
                clicked_cmd = Some(format!("{}\n", cmd.trim_end_matches('\n')));
                self.show_favorites = false;
            }
            if close {
                self.show_favorites = false;
            }
        }

        // ── 添加/编辑快捷命令 ──
        if self.show_add {
            let title = if self.edit_index.is_some() {
                "编辑快捷命令"
            } else {
                "添加快捷命令"
            };
            let mut close = false;
            let mut save = false;
            egui::Window::new(title)
                .collapsible(false)
                .resizable(false)
                .default_width(320.0)
                .anchor(
                    egui::Align2::LEFT_BOTTOM,
                    [legacy_popup_left, -COMMAND_BAR_HEIGHT - 4.0],
                )
                .frame(
                    egui::Frame::new()
                        .fill(egui::Color32::from_rgb(0x16, 0x1b, 0x22))
                        .stroke(egui::Stroke::new(
                            1.0,
                            egui::Color32::from_rgb(0x30, 0x36, 0x3d),
                        ))
                        .corner_radius(6.0)
                        .inner_margin(egui::Margin::same(10)),
                )
                .show(ctx, |ui| {
                    ui.label(egui::RichText::new("标签").size(10.0).color(muted));
                    ui.add(
                        egui::TextEdit::singleline(&mut self.add_label)
                            .desired_width(280.0)
                            .hint_text("显示名称，最多 20 字"),
                    );
                    ui.add_space(4.0);
                    ui.label(egui::RichText::new("命令").size(10.0).color(muted));
                    ui.add(
                        egui::TextEdit::singleline(&mut self.add_command)
                            .desired_width(280.0)
                            .hint_text("发送到终端的命令")
                            .font(egui::FontId::monospace(11.0)),
                    );
                    if !self.add_error.is_empty() {
                        ui.label(
                            egui::RichText::new(&self.add_error)
                                .size(10.0)
                                .color(egui::Color32::from_rgb(0xf8, 0x51, 0x49)),
                        );
                    }
                    ui.add_space(6.0);
                    ui.horizontal(|ui| {
                        if ui.button("取消").clicked() {
                            close = true;
                        }
                        if ui.button("确定").clicked() {
                            save = true;
                        }
                    });
                });
            let (escape_pressed, enter_pressed) = ctx.input(|input| {
                (
                    input.key_pressed(egui::Key::Escape),
                    input.key_pressed(egui::Key::Enter),
                )
            });
            if escape_pressed {
                close = true;
                save = false;
            } else if enter_pressed {
                save = true;
            }
            if save {
                let label = self.add_label.trim().to_string();
                let command = self.add_command.trim().to_string();
                if label.is_empty() {
                    self.add_error = "请输入标签名称".into();
                } else if command.is_empty() {
                    self.add_error = "请输入命令内容".into();
                } else {
                    let label = if label.chars().count() > 20 {
                        label.chars().take(20).collect()
                    } else {
                        label
                    };
                    if let Some(i) = self.edit_index {
                        if let Some(cmd) = self.commands.get_mut(i) {
                            cmd.label = label;
                            cmd.command = command;
                        }
                    } else {
                        self.commands.push(QuickCommand {
                            label,
                            command,
                            system: false,
                        });
                    }
                    self.save_commands();
                    self.show_add = false;
                    self.add_error.clear();
                }
            }
            if close {
                self.show_add = false;
            }
        }

        // 统一记入历史（push_history 去重，输入框路径再记一次只会顶到最前）
        if let Some(ref cmd) = clicked_cmd {
            let c = cmd.clone();
            self.push_history(&c);
        }

        clicked_cmd
    }
}
