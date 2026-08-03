use super::*;

impl App {
    pub(super) fn render_serial_failure_card(
        &self,
        ctx: &egui::Context,
        pane_rect: egui::Rect,
        error: &str,
    ) -> bool {
        let width = pane_rect.width().min(480.0).max(260.0);
        let position = egui::pos2(
            (pane_rect.center().x - width / 2.0).max(pane_rect.left()),
            pane_rect.top() + 48.0,
        );
        let mut retry = false;
        egui::Area::new(egui::Id::new("serial_open_failure_card"))
            .order(egui::Order::Foreground)
            .fixed_pos(position)
            .show(ctx, |ui| {
                egui::Frame::new()
                    .fill(egui::Color32::from_rgb(0x16, 0x1b, 0x22))
                    .stroke(egui::Stroke::new(
                        1.0,
                        egui::Color32::from_rgb(0xf8, 0x51, 0x49),
                    ))
                    .corner_radius(8.0)
                    .inner_margin(egui::Margin::same(16))
                    .show(ui, |ui| {
                        ui.set_width((width - 32.0).max(228.0));
                        ui.label(
                            egui::RichText::new("串口连接失败")
                                .size(17.0)
                                .strong()
                                .color(egui::Color32::from_rgb(0xff, 0x7b, 0x72)),
                        );
                        ui.add_space(8.0);
                        ui.label(
                            egui::RichText::new(crate::serial::user_open_error_message(error))
                                .size(13.0)
                                .color(egui::Color32::from_rgb(0xc9, 0xd1, 0xd9)),
                        );
                        ui.add_space(12.0);
                        if ui.button("重新连接串口").clicked() {
                            retry = true;
                        }
                    });
            });
        retry
    }

    pub(super) fn render_connection_context_menu(&mut self, ctx: &egui::Context) {
        // 侧边栏连接右键菜单
        if let Some((conn_idx, pos)) = self.sidebar.conn_context_menu {
            let mut close_menu = false;
            let mut action: Option<&str> = None;
            egui::Window::new("conn_ctx_menu")
                .title_bar(false)
                .resizable(false)
                .collapsible(false)
                .movable(false)
                .fixed_pos(pos)
                .fixed_size(egui::vec2(130.0, 0.0))
                .frame(
                    egui::Frame::new()
                        .fill(egui::Color32::from_rgb(0x1c, 0x20, 0x28))
                        .stroke(egui::Stroke::new(
                            1.0,
                            egui::Color32::from_rgb(0x30, 0x36, 0x3d),
                        ))
                        .corner_radius(4.0)
                        .inner_margin(egui::Margin::same(4)),
                )
                .show(ctx, |ui| {
                    let item = |ui: &mut egui::Ui, label: &str, red: bool| -> bool {
                        let (rect, resp) =
                            ui.allocate_exact_size(egui::vec2(122.0, 26.0), egui::Sense::click());
                        if resp.hovered() {
                            ui.painter().rect_filled(
                                rect,
                                3.0,
                                egui::Color32::from_rgb(0x30, 0x36, 0x3d),
                            );
                        }
                        let color = if red {
                            egui::Color32::from_rgb(0xf8, 0x51, 0x49)
                        } else {
                            egui::Color32::from_rgb(0xc9, 0xd1, 0xd9)
                        };
                        ui.painter().text(
                            rect.left_center() + egui::vec2(8.0, 0.0),
                            egui::Align2::LEFT_CENTER,
                            label,
                            egui::FontId::proportional(12.0),
                            color,
                        );
                        resp.clicked()
                    };
                    if item(ui, "连接", false) {
                        action = Some("connect");
                        close_menu = true;
                    }
                    if item(ui, "新建会话", false) {
                        action = Some("new_session");
                        close_menu = true;
                    }
                    ui.separator();
                    if item(ui, "编辑属性", false) {
                        action = Some("edit");
                        close_menu = true;
                    }
                    if item(ui, "删除", true) {
                        action = Some("delete");
                        close_menu = true;
                    }
                });

            // 点击菜单外关闭
            if !close_menu
                && ctx.input(|i| {
                    i.pointer.any_pressed()
                        && i.pointer.interact_pos().map_or(false, |p| {
                            let r = egui::Rect::from_min_size(pos, egui::vec2(130.0, 130.0));
                            !r.contains(p)
                        })
                })
            {
                close_menu = true;
            }

            if close_menu {
                self.sidebar.conn_context_menu = None;
            }

            if let Some(act) = action {
                if let Some(conn) = self.sidebar.connections.get(conn_idx).cloned() {
                    match act {
                        "connect" => {
                            // 跟单击一样的连接逻辑
                            let mut conn = conn;
                            if conn.password.is_empty()
                                && (conn.auth == "keyring" || conn.auth == "password")
                            {
                                let entry = crate::keyring::KeyringEntry::new(
                                    &conn.user, &conn.host, conn.port,
                                );
                                if let Ok(Some(pw)) = entry.retrieve_password() {
                                    conn.password = pw;
                                    conn.auth = "password".to_string();
                                }
                            }
                            self.sidebar.on_connect = Some(conn);
                        }
                        "new_session" => {
                            self.sidebar.show_new_connection = true;
                            self.sidebar.new_conn = sidebar::NewConnForm::default();
                        }
                        "edit" => {
                            // 用连接信息填充新建对话框作为编辑
                            self.sidebar.show_new_connection = true;
                            self.sidebar.new_conn = sidebar::NewConnForm {
                                label: conn.label.clone(),
                                host: conn.host.clone(),
                                port: conn.port.to_string(),
                                user: conn.user.clone(),
                                auth_idx: if conn.auth == "key" { 0 } else { 1 },
                                key_path: conn.key_path.clone(),
                                password: String::new(),
                                group: conn.group.clone(),
                                new_group: String::new(),
                                status: String::new(),
                            };
                        }
                        "delete" => {
                            // 从 connections.toml 删除
                            let mut store = crate::connections::ConnectionStore::load();
                            let host_key = format!("{}:{}", conn.host, conn.port);
                            for (_, group) in store.groups.iter_mut() {
                                group.hosts.remove(&host_key);
                            }
                            if let Err(e) = store.save() {
                                log::warn!("[SIDEBAR] 删除失败: {}", e);
                            }
                            self.sidebar.reload();
                        }
                        _ => {}
                    }
                }
            }
        }
    }

    pub(super) fn render_ssh_password_prompt(&mut self, ctx: &egui::Context) {
        // SSH 密码弹窗（在 main egui 上下文渲染，不在 sidebar panel 内）
        if self.sidebar.password_prompt.is_some() {
            let mut do_connect = false;
            let mut do_cancel = false;
            egui::Window::new("输入 SSH 密码")
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .resizable(false)
                .collapsible(false)
                .default_width(320.0)
                .frame(
                    egui::Frame::new()
                        .fill(egui::Color32::from_rgb(0x1c, 0x20, 0x28))
                        .stroke(egui::Stroke::new(
                            1.0,
                            egui::Color32::from_rgb(0x30, 0x36, 0x3d),
                        ))
                        .corner_radius(6.0)
                        .inner_margin(egui::Margin::same(12)),
                )
                .show(ctx, |ui| {
                    if let Some(conn) = &self.sidebar.password_prompt {
                        ui.label(
                            egui::RichText::new(format!(
                                "{}@{}:{}",
                                conn.user, conn.host, conn.port
                            ))
                            .size(13.0)
                            .color(egui::Color32::from_rgb(0x00, 0xd4, 0xff)),
                        );
                    }
                    if !self.sidebar.password_error.is_empty() {
                        ui.label(
                            egui::RichText::new(&self.sidebar.password_error)
                                .size(11.0)
                                .color(egui::Color32::from_rgb(0xf8, 0x51, 0x49)),
                        );
                    }
                    ui.add_space(6.0);
                    ui.horizontal(|ui| {
                        ui.label("密码:");
                        let resp = ui.add(
                            egui::TextEdit::singleline(&mut self.sidebar.password_input)
                                .password(true)
                                .desired_width(200.0),
                        );
                        // 回车触发连接
                        if resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                            do_connect = true;
                        }
                    });
                    ui.add_space(6.0);
                    ui.horizontal(|ui| {
                        if ui.button("连接").clicked() {
                            do_connect = true;
                        }
                        if ui.button("取消").clicked() {
                            do_cancel = true;
                        }
                    });
                });
            if do_connect {
                if let Some(mut conn) = self.sidebar.password_prompt.take() {
                    conn.password = self.sidebar.password_input.clone();
                    conn.auth = "password".to_string();
                    self.sidebar.password_connect = Some(conn);
                    self.sidebar.password_input.clear();
                    self.sidebar.password_error.clear();
                }
            }
            if do_cancel {
                self.sidebar.password_prompt = None;
                self.sidebar.password_input.clear();
                self.sidebar.password_error.clear();
            }
        }
    }

    pub(super) fn render_terminal_context_menu(
        &mut self,
        ctx: &egui::Context,
        horizontal_split_label: &'static str,
        vertical_split_label: &'static str,
        split_actions_enabled: bool,
        logging_active: bool,
        recording_active: bool,
        serial_state: Option<SerialTerminalMenuState>,
    ) -> Option<&'static str> {
        let mut term_menu_action: Option<&'static str> = None;
        // 终端右键菜单
        if self.show_terminal_menu {
            let pos = self.terminal_menu_pos;
            let mut close = false;
            let menu_response = egui::Window::new("terminal_ctx_menu")
                .title_bar(false)
                .resizable(false)
                .collapsible(false)
                .movable(false)
                .fixed_pos(pos)
                .default_width(160.0)
                .min_width(160.0)
                .max_width(160.0)
                .frame(
                    egui::Frame::new()
                        .fill(egui::Color32::from_rgb(0x1c, 0x20, 0x28))
                        .stroke(egui::Stroke::new(
                            1.0,
                            egui::Color32::from_rgb(0x30, 0x36, 0x3d),
                        ))
                        .corner_radius(4.0)
                        .inner_margin(egui::Margin::same(4)),
                )
                .show(ctx, |ui| {
                    let item = |ui: &mut egui::Ui, label: &str, enabled: bool| -> bool {
                        let (rect, resp) =
                            ui.allocate_exact_size(egui::vec2(152.0, 26.0), egui::Sense::click());
                        if enabled && resp.hovered() {
                            ui.painter().rect_filled(
                                rect,
                                3.0,
                                egui::Color32::from_rgb(0x30, 0x36, 0x3d),
                            );
                        }
                        ui.painter().text(
                            rect.left_center() + egui::vec2(8.0, 0.0),
                            egui::Align2::LEFT_CENTER,
                            label,
                            egui::FontId::proportional(12.0),
                            if enabled {
                                egui::Color32::from_rgb(0xc9, 0xd1, 0xd9)
                            } else {
                                egui::Color32::from_rgb(0x6e, 0x76, 0x81)
                            },
                        );
                        enabled && resp.clicked()
                    };

                    if item(ui, "复制", true) {
                        term_menu_action = Some("copy");
                        close = true;
                    }
                    if item(ui, "粘贴", true) {
                        term_menu_action = Some("paste");
                        close = true;
                    }
                    if item(ui, "全选", true) {
                        term_menu_action = Some("select_all");
                        close = true;
                    }
                    if item(ui, "清屏", true) {
                        term_menu_action = Some("clear");
                        close = true;
                    }
                    if item(ui, "清空缓存", true) {
                        term_menu_action = Some("clear_scrollback");
                        close = true;
                    }
                    ui.separator();
                    if item(ui, "搜索 (Ctrl+F)", true) {
                        term_menu_action = Some("search");
                        close = true;
                    }
                    ui.separator();
                    if item(ui, "终端主题", true) {
                        term_menu_action = Some("theme");
                        close = true;
                    }
                    if let Some(serial_state) = serial_state {
                        ui.separator();
                        let reconnect_enabled = serial_state != SerialTerminalMenuState::Connecting;
                        let disconnect_enabled = serial_state == SerialTerminalMenuState::Connected;
                        if item(ui, "重新连接串口", reconnect_enabled) {
                            term_menu_action = Some("serial_reconnect");
                            close = true;
                        }
                        if item(ui, "断开串口", disconnect_enabled) {
                            term_menu_action = Some("serial_disconnect");
                            close = true;
                        }
                    }
                    ui.separator();
                    if item(
                        ui,
                        if logging_active {
                            "⏹ 停止录制日志"
                        } else {
                            "⏺ 开始录制日志"
                        },
                        true,
                    ) {
                        term_menu_action = Some(if logging_active {
                            "stop_log"
                        } else {
                            "start_log"
                        });
                        close = true;
                    }
                    if item(
                        ui,
                        if recording_active {
                            "⏹ 停止录屏"
                        } else {
                            "⏺ 开始录屏"
                        },
                        true,
                    ) {
                        term_menu_action = Some(if recording_active {
                            "stop_recording"
                        } else {
                            "start_recording"
                        });
                        close = true;
                    }
                    if item(ui, "▶ 回放录屏", true) {
                        term_menu_action = Some("play_recording");
                        close = true;
                    }
                    ui.separator();
                    if item(ui, horizontal_split_label, split_actions_enabled) {
                        term_menu_action = Some("split_h");
                        close = true;
                    }
                    if item(ui, vertical_split_label, split_actions_enabled) {
                        term_menu_action = Some("split_v");
                        close = true;
                    }
                    if item(ui, "关闭面板", true) {
                        term_menu_action = Some("close_pane");
                        close = true;
                    }
                });
            let actual_menu_rect = menu_response.map(|response| response.response.rect);

            // 点击菜单外关闭
            let pointer_requested_close = ctx.input(|input| {
                should_close_terminal_context_menu(
                    close,
                    input.pointer.any_pressed(),
                    input.pointer.interact_pos(),
                    actual_menu_rect,
                    self.terminal_menu_ignore_pointer_press_once,
                )
            });
            self.terminal_menu_ignore_pointer_press_once = false;
            if pointer_requested_close {
                self.show_terminal_menu = false;
            }
        }
        term_menu_action
    }
}
