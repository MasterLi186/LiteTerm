use super::*;

impl Sidebar {
    pub fn ui_with_monitor(
        &mut self,
        ctx: &egui::Context,
        active_key: &crate::monitor::MonitorKey,
        monitor: Option<&crate::monitor::MonitorData>,
        error: Option<&str>,
    ) -> f32 {
        if !self.visible {
            return 0.0;
        }
        let panel_width = self.width;
        self.monitor_views.entry(active_key.clone()).or_default();
        let presentation = monitor_source_presentation(active_key, monitor, error);
        self.ui_inner(ctx, panel_width, active_key, monitor, &presentation)
    }

    pub fn ui(&mut self, ctx: &egui::Context) -> f32 {
        if !self.visible {
            return 0.0;
        }
        let panel_width = self.width;
        let active_key = crate::monitor::MonitorKey::Local;
        self.monitor_views.entry(active_key.clone()).or_default();
        let presentation = monitor_source_presentation(&active_key, None, None);
        self.ui_inner(ctx, panel_width, &active_key, None, &presentation)
    }

    fn ui_inner(
        &mut self,
        ctx: &egui::Context,
        panel_width: f32,
        active_key: &crate::monitor::MonitorKey,
        monitor: Option<&crate::monitor::MonitorData>,
        presentation: &MonitorSourcePresentation,
    ) -> f32 {
        egui::SidePanel::left("sidebar")
            .exact_width(panel_width)
            .resizable(false)
            .frame(
                egui::Frame::new()
                    .fill(egui::Color32::from_rgb(0x0d, 0x11, 0x17))
                    .inner_margin(egui::Margin::same(0)),
            )
            .show(ctx, |ui| {
                ui.style_mut().visuals.override_text_color =
                    Some(egui::Color32::from_rgb(0x8b, 0x94, 0x9e));

                // Header + toolbar
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    ui.add_space(6.0);
                    let arrow = if self.connections_visible {
                        "▼"
                    } else {
                        "▶"
                    };
                    let toggle = ui.add(
                        egui::Button::new(
                            egui::RichText::new(format!("{} 连接管理", arrow))
                                .size(SIDEBAR_SECTION_SIZE)
                                .color(egui::Color32::from_rgb(0x8b, 0x94, 0x9e)),
                        )
                        .frame(false),
                    );
                    if toggle.clicked() {
                        self.connections_visible = !self.connections_visible;
                    }

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.spacing_mut().item_spacing.x = 4.0;
                        let normal = egui::Color32::from_rgb(0x8b, 0x94, 0x9e);
                        let cyan = egui::Color32::from_rgb(0x00, 0xd4, 0xff);

                        let r1 = ui.add(
                            egui::Button::new(egui::RichText::new("+").size(14.0).color(cyan))
                                .frame(false)
                                .min_size(egui::vec2(20.0, 18.0)),
                        );
                        r1.clone().on_hover_text("新建连接");
                        if r1.clicked() {
                            self.show_new_connection = true;
                            self.new_conn = NewConnForm::default();
                        }

                        let r2 = ui.add(
                            egui::Button::new(egui::RichText::new("⚿").size(13.0).color(normal))
                                .frame(false)
                                .min_size(egui::vec2(20.0, 18.0)),
                        );
                        r2.clone().on_hover_text("SSH 密钥管理");
                        if r2.clicked() {
                            self.show_key_manager = true;
                            self.ssh_keys_loaded = false;
                        }

                        let r3 = ui.add(
                            egui::Button::new(egui::RichText::new("⬆").size(12.0).color(normal))
                                .frame(false)
                                .min_size(egui::vec2(20.0, 18.0)),
                        );
                        r3.clone().on_hover_text("导出配置");
                        if r3.clicked() {
                            self.export_connections_with_dialog();
                        }

                        let r4 = ui.add(
                            egui::Button::new(egui::RichText::new("⬇").size(12.0).color(normal))
                                .frame(false)
                                .min_size(egui::vec2(20.0, 18.0)),
                        );
                        r4.clone().on_hover_text("导入配置");
                        if r4.clicked() {
                            self.import_connections_with_dialog();
                        }
                    });
                });
                ui.add_space(2.0);
                ui.separator();

                // 整个侧边栏内容可滚动
                egui::ScrollArea::vertical()
                    .id_salt("sidebar_main_scroll")
                    .show(ui, |ui| {
                        if self.connections_visible {
                            let mut current_group = String::new();
                            for (i, conn) in self.connections.iter().enumerate() {
                                if conn.group != current_group {
                                    current_group = conn.group.clone();
                                    let is_collapsed =
                                        self.collapsed_groups.contains(&current_group);
                                    ui.add_space(6.0);
                                    let gr = ui.horizontal(|ui| {
                                        ui.add_space(8.0);
                                        let arrow = if is_collapsed { "▶" } else { "▼" };
                                        ui.label(
                                            egui::RichText::new(arrow)
                                                .size(8.0)
                                                .color(egui::Color32::from_rgb(0x48, 0x4f, 0x58)),
                                        );
                                        let dot = egui::Color32::from_rgb(
                                            conn.group_color[0],
                                            conn.group_color[1],
                                            conn.group_color[2],
                                        );
                                        let (r, _) = ui.allocate_exact_size(
                                            egui::vec2(8.0, 8.0),
                                            egui::Sense::hover(),
                                        );
                                        ui.painter().circle_filled(r.center(), 4.0, dot);
                                        ui.label(
                                            egui::RichText::new(&current_group)
                                                .size(SIDEBAR_META_SIZE)
                                                .color(egui::Color32::from_rgb(0x8b, 0x94, 0x9e)),
                                        )
                                    });
                                    if gr.inner.clicked() {
                                        if is_collapsed {
                                            self.collapsed_groups.remove(&current_group);
                                        } else {
                                            self.collapsed_groups.insert(current_group.clone());
                                        }
                                    }
                                    ui.add_space(2.0);
                                    if is_collapsed {
                                        continue;
                                    }
                                }
                                if self.collapsed_groups.contains(&conn.group) {
                                    continue;
                                }

                                let is_selected = self.selected == Some(i);
                                let resp = ui.horizontal(|ui| {
                                    ui.add_space(16.0);
                                    let (rect, resp) = ui.allocate_exact_size(
                                        egui::vec2(panel_width - 20.0, CONNECTION_ROW_HEIGHT),
                                        egui::Sense::click(),
                                    );
                                    if is_selected || resp.hovered() {
                                        let bg = if is_selected {
                                            egui::Color32::from_rgb(0x1c, 0x20, 0x28)
                                        } else {
                                            egui::Color32::from_rgba_unmultiplied(
                                                0x30, 0x36, 0x3d, 0x60,
                                            )
                                        };
                                        ui.painter().rect_filled(rect, 3.0, bg);
                                    }
                                    let tr = rect.shrink2(egui::vec2(4.0, 0.0));
                                    let g = ui.painter().layout(
                                        conn.label.clone(),
                                        egui::FontId::proportional(SIDEBAR_BODY_SIZE),
                                        egui::Color32::from_rgb(0xc9, 0xd1, 0xd9),
                                        tr.width(),
                                    );
                                    ui.painter().galley(
                                        tr.left_center() - egui::vec2(0.0, g.size().y / 2.0),
                                        g,
                                        egui::Color32::from_rgb(0xc9, 0xd1, 0xd9),
                                    );
                                    resp
                                });
                                if resp.inner.clicked() {
                                    log::debug!(
                                        "[SIDEBAR] 点击连接: {} ({}:{}) auth={}",
                                        conn.label,
                                        conn.host,
                                        conn.port,
                                        conn.auth
                                    );
                                    self.selected = Some(i);
                                    let mut conn = conn.clone();
                                    // 密码为空时先尝试从 keyring 读
                                    if conn.password.is_empty()
                                        && (conn.auth == "keyring" || conn.auth == "password")
                                    {
                                        let entry = crate::keyring::KeyringEntry::new(
                                            &conn.user, &conn.host, conn.port,
                                        );
                                        match entry.retrieve_password() {
                                            Ok(Some(pw)) => {
                                                log::debug!(
                                                    "[SIDEBAR] keyring 读到密码: {}",
                                                    conn.label
                                                );
                                                conn.password = pw;
                                                conn.auth = "password".to_string();
                                                self.on_connect = Some(conn);
                                            }
                                            _ => {
                                                log::debug!(
                                                    "[SIDEBAR] keyring 无密码，弹框: {}",
                                                    conn.label
                                                );
                                                self.password_prompt = Some(conn);
                                                self.password_input.clear();
                                                self.password_error.clear();
                                            }
                                        }
                                    } else {
                                        self.on_connect = Some(conn);
                                    }
                                }
                                // 右键菜单
                                if resp.inner.secondary_clicked() {
                                    let pos =
                                        ctx.input(|i| i.pointer.interact_pos().unwrap_or_default());
                                    self.conn_context_menu = Some((i, pos));
                                    self.selected = Some(i);
                                }
                            }
                            ui.add_space(4.0);
                        }

                        // 当前标签监控来源 + 系统监控面板
                        ui.separator();
                        ui.horizontal(|ui| {
                            ui.add_space(8.0);
                            let (r, _) =
                                ui.allocate_exact_size(egui::vec2(8.0, 8.0), egui::Sense::hover());
                            ui.painter()
                                .circle_filled(r.center(), 4.0, presentation.dot.color());
                            ui.label(
                                egui::RichText::new(&presentation.title)
                                    .size(SIDEBAR_SECTION_SIZE)
                                    .color(egui::Color32::from_rgb(0xe6, 0xed, 0xf3)),
                            );
                            if !presentation.detail.is_empty() {
                                ui.label(
                                    egui::RichText::new(&presentation.detail)
                                        .size(SIDEBAR_META_SIZE)
                                        .color(egui::Color32::from_rgb(0x8b, 0x94, 0x9e)),
                                );
                            }
                        });
                        if let Some(warning) = &presentation.warning {
                            ui.label(
                                egui::RichText::new(warning)
                                    .size(SIDEBAR_META_SIZE)
                                    .color(egui::Color32::from_rgb(0xd2, 0x99, 0x22)),
                            );
                        }
                        if !presentation.message.is_empty() {
                            ui.label(
                                egui::RichText::new(&presentation.message)
                                    .size(SIDEBAR_META_SIZE)
                                    .color(egui::Color32::from_rgb(0x8b, 0x94, 0x9e)),
                            );
                        }

                        if let Some(mon) = monitor {
                            let open_process_manager = &mut self.open_process_manager;
                            let open_network_detail = &mut self.open_network_detail;
                            let view = self.monitor_views.entry(active_key.clone()).or_default();
                            Self::render_monitor_static(
                                ui,
                                mon,
                                panel_width,
                                active_key,
                                &mut view.selected_iface,
                                &mut view.interface_selection_manual,
                                &mut view.process_tab,
                                &mut view.net_rx_history,
                                &mut view.net_tx_history,
                                open_process_manager,
                                open_network_detail,
                            );
                        }
                    }); // end ScrollArea
            });

        self.render_dialogs(ctx);
        panel_width
    }

    fn render_monitor_static(
        ui: &mut egui::Ui,
        mon: &crate::monitor::MonitorData,
        panel_width: f32,
        active_key: &crate::monitor::MonitorKey,
        selected_iface: &mut Option<String>,
        interface_selection_manual: &mut bool,
        process_tab: &mut u8,
        net_rx_history: &mut Vec<f64>,
        net_tx_history: &mut Vec<f64>,
        open_process_manager: &mut Option<OpenProcessManagerAction>,
        open_network_detail: &mut Option<OpenNetworkDetailAction>,
    ) {
        let geometry = sidebar_monitor_card_geometry(panel_width, ui.available_width());
        if !geometry.can_render {
            return;
        }
        let cpu_text_width = geometry.uptime_content_width;
        let label_color = egui::Color32::from_rgb(0x48, 0x4f, 0x58);
        let value_color = egui::Color32::from_rgb(0xe6, 0xed, 0xf3);
        let section_color = egui::Color32::from_rgb(0x8b, 0x94, 0x9e);
        let card_bg = egui::Color32::from_rgb(0x16, 0x1b, 0x22);
        let card_border = egui::Color32::from_rgb(0x21, 0x26, 0x2d);
        let bar_bg = egui::Color32::from_rgb(0x21, 0x26, 0x2d);

        // ── Uptime & Load ──
        ui.add_space(4.0);
        let _ = show_sidebar_monitor_card(
            ui,
            geometry,
            card_bg,
            card_border,
            geometry.uptime_inner_margin,
            |ui| {
                ui.spacing_mut().item_spacing.x = 0.0;
                ui.columns(2, |columns| {
                    columns[0].vertical(|ui| {
                        ui.label(
                            egui::RichText::new("运行时间")
                                .size(SIDEBAR_META_SIZE)
                                .color(label_color),
                        );
                        ui.label(
                            egui::RichText::new(&mon.uptime_text)
                                .size(SIDEBAR_VALUE_SIZE)
                                .color(value_color),
                        );
                    });
                    columns[1].vertical(|ui| {
                        ui.label(
                            egui::RichText::new("系统负载")
                                .size(SIDEBAR_META_SIZE)
                                .color(label_color),
                        );
                        ui.label(
                            egui::RichText::new(&mon.load_text)
                                .size(SIDEBAR_VALUE_SIZE)
                                .color(value_color),
                        );
                    });
                });
            },
        );

        // ── 资源 ──
        ui.add_space(4.0);
        let _ = show_sidebar_monitor_card(ui, geometry, card_bg, card_border, 0.0, |ui| {
            ui.horizontal(|ui| {
                ui.add_space(8.0);
                ui.label(
                    egui::RichText::new("资源")
                        .size(SIDEBAR_SECTION_SIZE)
                        .color(section_color),
                );
            });
            let (sep, _) =
                ui.allocate_exact_size(egui::vec2(ui.available_width(), 1.0), egui::Sense::hover());
            ui.painter().rect_filled(sep, 0.0, card_border);

            // CPU
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.add_space(8.0);
                ui.label(
                    egui::RichText::new("CPU")
                        .size(SIDEBAR_BODY_SIZE)
                        .color(section_color),
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.add_space(8.0);
                    ui.label(
                        egui::RichText::new(format!("{:.1}%", mon.cpu_percent))
                            .size(SIDEBAR_VALUE_SIZE)
                            .color(value_color),
                    );
                });
            });
            ui.horizontal(|ui| {
                ui.add_space(8.0);
                ui.add_sized(
                    [cpu_text_width, 0.0],
                    egui::Label::new(
                        egui::RichText::new(&mon.cpu_name)
                            .size(SIDEBAR_META_SIZE)
                            .color(label_color),
                    )
                    .wrap(),
                );
            });
            Self::draw_gauge(ui, cpu_text_width, mon.cpu_percent, bar_bg);
            ui.add_space(4.0);

            // Memory
            ui.horizontal(|ui| {
                ui.add_space(8.0);
                ui.label(
                    egui::RichText::new("内存")
                        .size(SIDEBAR_BODY_SIZE)
                        .color(section_color),
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.add_space(8.0);
                    ui.label(
                        egui::RichText::new(&mon.memory_text)
                            .size(SIDEBAR_VALUE_SIZE)
                            .color(value_color),
                    );
                });
            });
            Self::draw_gauge(ui, cpu_text_width, mon.memory_percent, bar_bg);
            ui.add_space(4.0);

            // Swap
            ui.horizontal(|ui| {
                ui.add_space(8.0);
                ui.label(
                    egui::RichText::new("交换")
                        .size(SIDEBAR_BODY_SIZE)
                        .color(section_color),
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.add_space(8.0);
                    ui.label(
                        egui::RichText::new(&mon.swap_text)
                            .size(SIDEBAR_VALUE_SIZE)
                            .color(value_color),
                    );
                });
            });
            Self::draw_gauge(ui, cpu_text_width, mon.swap_percent, bar_bg);
            ui.add_space(4.0);
        });

        // ── 进程 ──
        if !mon.processes.is_empty() {
            let mut sorted = mon.processes.clone();
            match *process_tab {
                0 => sorted.sort_by(|a, b| b.mem_bytes.cmp(&a.mem_bytes)),
                2 => sorted.sort_by(|a, b| a.name.cmp(&b.name)),
                _ => sorted.sort_by(|a, b| {
                    b.cpu
                        .partial_cmp(&a.cpu)
                        .unwrap_or(std::cmp::Ordering::Equal)
                }),
            }

            ui.add_space(4.0);
            let _ = show_sidebar_monitor_card(ui, geometry, card_bg, card_border, 0.0, |ui| {
                ui.horizontal(|ui| {
                    ui.add_space(8.0);
                    ui.label(
                        egui::RichText::new("进程")
                            .size(SIDEBAR_SECTION_SIZE)
                            .color(section_color),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.add_space(4.0);
                        for &(label, idx) in &[("命令", 2u8), ("CPU", 1u8), ("内存", 0u8)] {
                            let active = *process_tab == idx;
                            let color = if active {
                                egui::Color32::from_rgb(0x00, 0xd4, 0xff)
                            } else {
                                egui::Color32::from_rgb(0x48, 0x4f, 0x58)
                            };
                            let bg = if active {
                                egui::Color32::from_rgba_unmultiplied(0x00, 0xd4, 0xff, 0x26)
                            } else {
                                egui::Color32::TRANSPARENT
                            };
                            if ui
                                .add(
                                    egui::Button::new(
                                        egui::RichText::new(label)
                                            .size(SIDEBAR_META_SIZE)
                                            .color(color),
                                    )
                                    .fill(bg)
                                    .stroke(egui::Stroke::NONE)
                                    .corner_radius(3.0)
                                    .min_size(egui::vec2(0.0, 16.0)),
                                )
                                .clicked()
                            {
                                *process_tab = idx;
                            }
                        }
                    });
                });
                let (sep, _) = ui.allocate_exact_size(
                    egui::vec2(ui.available_width(), 1.0),
                    egui::Sense::hover(),
                );
                ui.painter().rect_filled(sep, 0.0, card_border);

                for (i, p) in sorted.iter().take(8).enumerate() {
                    let open_action = process_manager_open_action(active_key);
                    let (row_rect, row_response) = ui.allocate_exact_size(
                        process_row_size(ui.available_width()),
                        if open_action.is_some() {
                            egui::Sense::click()
                        } else {
                            egui::Sense::hover()
                        },
                    );
                    let row_bg = if row_response.hovered() {
                        egui::Color32::from_rgba_unmultiplied(0x00, 0xd4, 0xff, 0x0f)
                    } else if i % 2 == 1 {
                        egui::Color32::from_rgba_unmultiplied(255, 255, 255, 4)
                    } else {
                        egui::Color32::TRANSPARENT
                    };
                    ui.painter().rect_filled(row_rect, 0.0, row_bg);
                    render_process_row_content(
                        ui,
                        row_rect,
                        &p.mem_mb,
                        p.cpu,
                        &p.name,
                        section_color,
                    );
                    if row_response.clicked() {
                        *open_process_manager = open_action;
                    }
                }
            });
        }

        // ── 网络 ──
        if !mon.net_interfaces.is_empty() {
            let sel = selected_iface
                .clone()
                .filter(|selected| {
                    mon.net_interfaces
                        .iter()
                        .any(|interface| interface.name == *selected)
                })
                .or_else(|| automatic_network_interface(mon))
                .unwrap_or_else(|| mon.net_interfaces[0].name.clone());
            let iface_data = mon.net_interfaces.iter().find(|n| n.name == sel);
            let (tx_rate, rx_rate) = iface_data.map(|d| (d.tx_rate, d.rx_rate)).unwrap_or((0, 0));

            ui.add_space(4.0);
            let _ = show_sidebar_monitor_card(ui, geometry, card_bg, card_border, 0.0, |ui| {
                ui.horizontal(|ui| {
                    ui.add_space(8.0);
                    ui.label(
                        egui::RichText::new("网络")
                            .size(SIDEBAR_SECTION_SIZE)
                            .color(section_color),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.add_space(4.0);
                        let iface_names: Vec<String> =
                            mon.net_interfaces.iter().map(|n| n.name.clone()).collect();
                        let mut changed_iface: Option<String> = None;
                        egui::ComboBox::from_id_salt("net_iface_sel")
                            .selected_text(&sel)
                            .width(80.0)
                            .show_ui(ui, |ui| {
                                for name in &iface_names {
                                    if ui.selectable_label(&sel == name, name).clicked() {
                                        changed_iface = Some(name.clone());
                                    }
                                }
                            });
                        if let Some(name) = changed_iface {
                            *selected_iface = Some(name);
                            *interface_selection_manual = true;
                            // 切换网卡后清空折线，等下一次 MonitorUpdate 重新采样
                            net_rx_history.clear();
                            net_tx_history.clear();
                            // last_chart_iface 由 on_monitor_update 在下次采样时更新
                        }
                    });
                });
                let (sep, _) = ui.allocate_exact_size(
                    egui::vec2(ui.available_width(), 1.0),
                    egui::Sense::hover(),
                );
                ui.painter().rect_filled(sep, 0.0, card_border);

                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    ui.add_space(8.0);
                    ui.label(
                        egui::RichText::new(format!("↑ {}/s", format_speed(tx_rate)))
                            .size(SIDEBAR_BODY_SIZE)
                            .color(egui::Color32::from_rgb(0x3f, 0xb9, 0x50)),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.add_space(8.0);
                        ui.label(
                            egui::RichText::new(format!("↓ {}/s", format_speed(rx_rate)))
                                .size(SIDEBAR_BODY_SIZE)
                                .color(egui::Color32::from_rgb(0x58, 0xa6, 0xff)),
                        );
                    });
                });

                // Mini chart（历史由 on_monitor_update 写入）
                ui.add_space(2.0);
                ui.horizontal(|ui| {
                    ui.add_space(8.0);
                    let cw = cpu_text_width;
                    let ch = 44.0;
                    let (rect, response) =
                        ui.allocate_exact_size(egui::vec2(cw, ch), egui::Sense::click());
                    ui.painter()
                        .rect_filled(rect, 4.0, egui::Color32::from_rgb(0x0d, 0x11, 0x17));

                    let chart_max = net_tx_history
                        .iter()
                        .chain(net_rx_history.iter())
                        .copied()
                        .fold(1.0_f64, f64::max);
                    for (data, color) in [
                        (
                            net_tx_history.as_slice(),
                            egui::Color32::from_rgb(0x3f, 0xb9, 0x50),
                        ),
                        (
                            net_rx_history.as_slice(),
                            egui::Color32::from_rgb(0x58, 0xa6, 0xff),
                        ),
                    ] {
                        let points = network_line_points(data, rect, chart_max);
                        for segment in points.windows(2) {
                            ui.painter().line_segment(
                                [segment[0], segment[1]],
                                egui::Stroke::new(1.5, color),
                            );
                        }
                    }
                    if response.hovered() {
                        response
                            .clone()
                            .on_hover_cursor(egui::CursorIcon::PointingHand);
                    }
                    if response.clicked() {
                        *open_network_detail = Some(OpenNetworkDetailAction {
                            key: active_key.clone(),
                            initial_iface: Some(sel.clone()),
                        });
                    }
                });
                ui.add_space(4.0);
            });
        }

        // ── 磁盘 ──
        if !mon.disk_items.is_empty() {
            ui.add_space(4.0);
            let _ = show_sidebar_monitor_card(ui, geometry, card_bg, card_border, 0.0, |ui| {
                ui.horizontal(|ui| {
                    ui.add_space(8.0);
                    ui.label(
                        egui::RichText::new("磁盘")
                            .size(SIDEBAR_SECTION_SIZE)
                            .color(section_color),
                    );
                });
                let (sep, _) = ui.allocate_exact_size(
                    egui::vec2(ui.available_width(), 1.0),
                    egui::Sense::hover(),
                );
                ui.painter().rect_filled(sep, 0.0, card_border);

                // Header
                let (header_rect, _) = ui.allocate_exact_size(
                    egui::vec2(ui.available_width(), DISK_ROW_HEIGHT),
                    egui::Sense::hover(),
                );
                render_disk_row_content(
                    ui,
                    header_rect,
                    "挂载点",
                    "使用率",
                    "可用/总量",
                    DiskRowColors {
                        mount: label_color,
                        percent: label_color,
                        capacity: label_color,
                    },
                );

                for (i, d) in mon.disk_items.iter().enumerate() {
                    let row_bg = if i % 2 == 1 {
                        egui::Color32::from_rgba_unmultiplied(255, 255, 255, 4)
                    } else {
                        egui::Color32::TRANSPARENT
                    };
                    let (row_rect, _) = ui.allocate_exact_size(
                        egui::vec2(ui.available_width(), DISK_ROW_HEIGHT),
                        egui::Sense::hover(),
                    );
                    ui.painter().rect_filled(row_rect, 0.0, row_bg);
                    let percent_color = if d.percent > 90 {
                        egui::Color32::from_rgb(0xf8, 0x51, 0x49)
                    } else if d.percent > 70 {
                        egui::Color32::from_rgb(0xd2, 0x99, 0x22)
                    } else {
                        section_color
                    };
                    let percent_text = format!("{}%", d.percent);
                    let capacity_text = format!("{}/{}", d.avail, d.size);
                    render_disk_row_content(
                        ui,
                        row_rect,
                        &d.mount,
                        &percent_text,
                        &capacity_text,
                        DiskRowColors {
                            mount: egui::Color32::from_rgb(0xc9, 0xd1, 0xd9),
                            percent: percent_color,
                            capacity: section_color,
                        },
                    );
                }
            });
        }
        ui.add_space(8.0);
    }

    fn draw_gauge(ui: &mut egui::Ui, w: f32, pct: f32, bar_bg: egui::Color32) {
        ui.horizontal(|ui| {
            ui.add_space(8.0);
            let (rect, _) = ui.allocate_exact_size(egui::vec2(w, 4.0), egui::Sense::hover());
            ui.painter().rect_filled(rect, 2.0, bar_bg);
            let fill_w = (rect.width() * (pct / 100.0).min(1.0)).max(0.0);
            let color = if pct > 90.0 {
                egui::Color32::from_rgb(0xf8, 0x51, 0x49)
            } else if pct > 70.0 {
                egui::Color32::from_rgb(0xd2, 0x99, 0x22)
            } else {
                egui::Color32::from_rgb(0x00, 0xd4, 0xff)
            };
            ui.painter().rect_filled(
                egui::Rect::from_min_size(rect.min, egui::vec2(fill_w, 4.0)),
                2.0,
                color,
            );
        });
    }
}
