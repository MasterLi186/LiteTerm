use super::*;

const BACKGROUND: egui::Color32 = egui::Color32::from_rgb(0x0b, 0x0f, 0x15);
const NAV_BACKGROUND: egui::Color32 = egui::Color32::from_rgb(0x0f, 0x13, 0x1a);
const PANEL: egui::Color32 = egui::Color32::from_rgb(0x11, 0x16, 0x1e);
const PANEL_RAISED: egui::Color32 = egui::Color32::from_rgb(0x17, 0x1d, 0x26);
const INPUT: egui::Color32 = egui::Color32::from_rgb(0x0b, 0x10, 0x16);
const BORDER: egui::Color32 = egui::Color32::from_rgb(0x26, 0x30, 0x3b);
const BORDER_HOVER: egui::Color32 = egui::Color32::from_rgb(0x3b, 0x4a, 0x58);
const TEXT: egui::Color32 = egui::Color32::from_rgb(0xe3, 0xe9, 0xef);
const MUTED: egui::Color32 = egui::Color32::from_rgb(0x7d, 0x89, 0x97);
const DIM: egui::Color32 = egui::Color32::from_rgb(0x53, 0x5e, 0x6b);
const CYAN: egui::Color32 = egui::Color32::from_rgb(0x40, 0xcb, 0xd9);
const CYAN_HOVER: egui::Color32 = egui::Color32::from_rgb(0x57, 0xdc, 0xe9);
const GREEN: egui::Color32 = egui::Color32::from_rgb(0x40, 0xd9, 0x87);
const RED: egui::Color32 = egui::Color32::from_rgb(0xf0, 0x6a, 0x6a);
const NAV_WIDTH: f32 = 220.0;

const SECTIONS: &[(SettingsSection, &str, &str)] = &[
    (SettingsSection::Terminal, "终端", "字体、配色与行为"),
    (SettingsSection::Appearance, "外观", "界面与布局"),
    (SettingsSection::Shortcuts, "快捷键", "键盘操作"),
    (SettingsSection::Ssh, "SSH", "连接与字符集"),
    (SettingsSection::Transfer, "文件传输", "下载与并发"),
    (SettingsSection::Zmodem, "ZMODEM", "终端文件传输"),
    (SettingsSection::About, "关于", "版本与运行环境"),
];

pub(super) fn show_page(panel: &mut SettingsPanel, ctx: &egui::Context) -> SettingsPanelAction {
    if !panel.visible {
        return SettingsPanelAction::None;
    }

    handle_shortcut_capture(panel, ctx);
    let mut action = SettingsPanelAction::None;

    egui::CentralPanel::default()
        .frame(
            egui::Frame::new()
                .fill(BACKGROUND)
                .inner_margin(egui::Margin::same(0)),
        )
        .show(ctx, |ui| {
            apply_page_style(ui);
            let full = ui.available_rect_before_wrap();
            let nav_width = NAV_WIDTH.min((full.width() * 0.32).max(132.0));
            let nav_rect = egui::Rect::from_min_max(
                full.min,
                egui::pos2(full.left() + nav_width, full.bottom()),
            );
            let content_rect =
                egui::Rect::from_min_max(egui::pos2(nav_rect.right(), full.top()), full.max);

            ui.painter().rect_filled(nav_rect, 0.0, NAV_BACKGROUND);
            ui.painter().line_segment(
                [nav_rect.right_top(), nav_rect.right_bottom()],
                egui::Stroke::new(1.0, BORDER),
            );

            ui.scope_builder(egui::UiBuilder::new().max_rect(nav_rect), |ui| {
                render_navigation(ui, panel);
            });
            ui.scope_builder(egui::UiBuilder::new().max_rect(content_rect), |ui| {
                render_content(ui, panel, &mut action);
            });
        });

    action
}

fn apply_page_style(ui: &mut egui::Ui) {
    let text_styles = &mut ui.style_mut().text_styles;
    text_styles.insert(egui::TextStyle::Body, egui::FontId::proportional(14.0));
    text_styles.insert(egui::TextStyle::Button, egui::FontId::proportional(13.0));
    text_styles.insert(egui::TextStyle::Small, egui::FontId::proportional(12.0));
    text_styles.insert(egui::TextStyle::Monospace, egui::FontId::monospace(13.0));
    let visuals = &mut ui.style_mut().visuals;
    visuals.override_text_color = Some(TEXT);
    visuals.widgets.inactive.bg_fill = INPUT;
    visuals.widgets.inactive.bg_stroke = egui::Stroke::new(1.0, BORDER);
    visuals.widgets.inactive.corner_radius = 6.0.into();
    visuals.widgets.hovered.bg_fill = PANEL_RAISED;
    visuals.widgets.hovered.bg_stroke = egui::Stroke::new(1.0, BORDER_HOVER);
    visuals.widgets.hovered.corner_radius = 6.0.into();
    visuals.widgets.active.bg_fill = egui::Color32::from_rgb(0x1b, 0x2c, 0x33);
    visuals.widgets.active.bg_stroke = egui::Stroke::new(1.0, CYAN);
    visuals.widgets.active.corner_radius = 6.0.into();
    visuals.selection.bg_fill = egui::Color32::from_rgba_unmultiplied(0x40, 0xcb, 0xd9, 70);
    visuals.selection.stroke = egui::Stroke::new(1.0, CYAN);
    ui.spacing_mut().item_spacing = egui::vec2(10.0, 10.0);
}

fn render_navigation(ui: &mut egui::Ui, panel: &mut SettingsPanel) {
    ui.set_min_size(ui.available_size());
    ui.add_space(22.0);
    ui.horizontal(|ui| {
        ui.add_space(16.0);
        let (logo_rect, _) = ui.allocate_exact_size(egui::vec2(42.0, 42.0), egui::Sense::hover());
        ui.painter().rect(
            logo_rect,
            8.0,
            egui::Color32::from_rgb(0x17, 0x20, 0x2a),
            egui::Stroke::new(1.0, egui::Color32::from_rgb(0x2c, 0x39, 0x45)),
            egui::epaint::StrokeKind::Inside,
        );
        ui.painter().text(
            logo_rect.center(),
            egui::Align2::CENTER_CENTER,
            "LT",
            egui::FontId::proportional(15.0),
            CYAN,
        );
        ui.vertical(|ui| {
            ui.label(egui::RichText::new("LiteTerm").size(17.0).color(TEXT));
            ui.label(egui::RichText::new("NATIVE SETTINGS").size(11.0).color(DIM));
        });
    });
    ui.add_space(20.0);

    for (section, title, subtitle) in SECTIONS {
        let selected = panel.section == *section;
        let available = (ui.available_width() - 16.0).max(80.0);
        ui.horizontal(|ui| {
            ui.add_space(8.0);
            let (rect, response) =
                ui.allocate_exact_size(egui::vec2(available, 54.0), egui::Sense::click());
            let hovered = response.hovered();
            if selected || hovered {
                ui.painter().rect(
                    rect,
                    6.0,
                    if selected {
                        egui::Color32::from_rgb(0x17, 0x24, 0x2a)
                    } else {
                        PANEL
                    },
                    egui::Stroke::new(
                        1.0,
                        if selected {
                            egui::Color32::from_rgb(0x28, 0x3c, 0x45)
                        } else {
                            BORDER
                        },
                    ),
                    egui::epaint::StrokeKind::Inside,
                );
            }
            if selected {
                ui.painter().rect_filled(
                    egui::Rect::from_min_max(
                        egui::pos2(rect.left(), rect.top() + 10.0),
                        egui::pos2(rect.left() + 3.0, rect.bottom() - 10.0),
                    ),
                    1.0,
                    CYAN,
                );
            }
            ui.painter().text(
                egui::pos2(rect.left() + 15.0, rect.top() + 17.0),
                egui::Align2::LEFT_CENTER,
                *title,
                egui::FontId::proportional(14.0),
                if selected { CYAN_HOVER } else { TEXT },
            );
            ui.painter().text(
                egui::pos2(rect.left() + 15.0, rect.top() + 39.0),
                egui::Align2::LEFT_CENTER,
                *subtitle,
                egui::FontId::proportional(11.0),
                DIM,
            );
            if response.clicked() {
                panel.section = *section;
                panel.error = None;
                panel.feedback = None;
                panel.capturing_shortcut = None;
            }
        });
    }
}

fn render_content(ui: &mut egui::Ui, panel: &mut SettingsPanel, action: &mut SettingsPanelAction) {
    let size = ui.available_size();
    ui.set_min_size(size);
    let compact = size.x < 620.0;

    egui::TopBottomPanel::top("settings_page_header")
        .exact_height(if compact { 116.0 } else { 100.0 })
        .frame(
            egui::Frame::new()
                .fill(BACKGROUND)
                .inner_margin(egui::Margin::symmetric(24, 14)),
        )
        .show_inside(ui, |ui| render_header(ui, panel, action, compact));

    egui::CentralPanel::default()
        .frame(
            egui::Frame::new()
                .fill(BACKGROUND)
                .inner_margin(egui::Margin::symmetric(24, 10)),
        )
        .show_inside(ui, |ui| {
            egui::ScrollArea::vertical()
                .id_salt(("settings_section_scroll", panel.section))
                .animated(false)
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    match panel.section {
                        SettingsSection::Terminal => render_terminal(ui, panel),
                        SettingsSection::Appearance => render_appearance(ui, panel),
                        SettingsSection::Shortcuts => render_shortcuts(ui, panel),
                        SettingsSection::Ssh => render_ssh(ui, panel),
                        SettingsSection::Transfer => render_transfer(ui, panel),
                        SettingsSection::Zmodem => render_zmodem(ui, panel),
                        SettingsSection::About => render_about(ui),
                    }
                    ui.add_space(24.0);
                });
        });
}

fn render_header(
    ui: &mut egui::Ui,
    panel: &mut SettingsPanel,
    action: &mut SettingsPanelAction,
    compact: bool,
) {
    let (title, description) = section_heading(panel.section);
    if compact {
        ui.vertical(|ui| {
            ui.label(egui::RichText::new(title).size(26.0).color(TEXT));
            ui.label(egui::RichText::new(description).size(13.0).color(MUTED));
            ui.horizontal(|ui| render_header_actions(ui, panel, action));
        });
    } else {
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.label(egui::RichText::new(title).size(26.0).color(TEXT));
                ui.label(egui::RichText::new(description).size(13.0).color(MUTED));
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                render_header_actions(ui, panel, action);
            });
        });
    }
    ui.add_space(6.0);
    if let Some(error) = &panel.error {
        ui.label(egui::RichText::new(error).size(12.5).color(RED));
    } else if let Some(feedback) = &panel.feedback {
        ui.label(egui::RichText::new(feedback).size(12.5).color(GREEN));
    } else if panel.is_dirty() {
        ui.label(
            egui::RichText::new("● 有尚未保存的修改")
                .size(11.5)
                .color(egui::Color32::from_rgb(0xd2, 0x99, 0x22)),
        );
    }
}

fn render_header_actions(
    ui: &mut egui::Ui,
    panel: &mut SettingsPanel,
    action: &mut SettingsPanelAction,
) {
    let dirty = panel.is_dirty();
    if ui
        .add_enabled(
            dirty,
            egui::Button::new(egui::RichText::new("保存设置").size(13.0).color(BACKGROUND))
                .fill(CYAN)
                .stroke(egui::Stroke::NONE)
                .corner_radius(6.0)
                .min_size(egui::vec2(100.0, 36.0)),
        )
        .clicked()
    {
        match panel.build_settings(&panel.base) {
            Ok(settings) => *action = SettingsPanelAction::Apply(settings),
            Err(error) => panel.set_error(error),
        }
    }
    if ui
        .add_enabled(
            dirty,
            egui::Button::new(egui::RichText::new("撤销修改").size(13.0).color(MUTED))
                .fill(PANEL)
                .stroke(egui::Stroke::new(1.0, BORDER))
                .corner_radius(6.0)
                .min_size(egui::vec2(96.0, 36.0)),
        )
        .clicked()
    {
        panel.reset_draft();
    }
}

fn section_heading(section: SettingsSection) -> (&'static str, &'static str) {
    match section {
        SettingsSection::Terminal => ("终端体验", "管理字体、字号、配色和终端行为。"),
        SettingsSection::Appearance => ("界面外观", "调整侧边栏与文件管理器的布局。"),
        SettingsSection::Shortcuts => ("快捷键", "点击录入新的组合键，冲突会在保存前提示。"),
        SettingsSection::Ssh => ("SSH 连接", "配置新连接默认使用的超时、保活与字符集。"),
        SettingsSection::Transfer => ("文件传输", "管理默认下载位置、并发任务和失败重试。"),
        SettingsSection::Zmodem => ("ZMODEM", "配置终端内文件传输检测和接收行为。"),
        SettingsSection::About => ("关于 LiteTerm", "Native 多协议终端与远程管理工具。"),
    }
}

fn card<R>(
    ui: &mut egui::Ui,
    title: &str,
    description: &str,
    add_contents: impl FnOnce(&mut egui::Ui) -> R,
) -> R {
    let response = egui::Frame::new()
        .fill(PANEL)
        .stroke(egui::Stroke::new(1.0, BORDER))
        .corner_radius(10.0)
        .inner_margin(egui::Margin::same(20))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(egui::RichText::new(title).size(16.0).color(TEXT));
            ui.label(egui::RichText::new(description).size(12.5).color(MUTED));
            ui.add_space(16.0);
            add_contents(ui)
        });
    response.inner
}

fn render_terminal(ui: &mut egui::Ui, panel: &mut SettingsPanel) {
    card(
        ui,
        "字体与字号",
        "选择适合长时间使用的等宽字体。",
        |ui| {
            let columns = if ui.available_width() >= 620.0 { 2 } else { 1 };
            ui.columns(columns, |columns_ui| {
                setting_label(&mut columns_ui[0], "字体族", "用于终端网格渲染");
                font_family_picker(&mut columns_ui[0], panel);
                let index = columns.saturating_sub(1);
                setting_label(&mut columns_ui[index], "字号", "8–48 px");
                columns_ui[index].horizontal(|ui| {
                    if small_button(ui, "−", panel.draft.font_size > MIN_FONT_SIZE).clicked() {
                        panel.draft.font_size = step_font_size(panel.draft.font_size, -1.0);
                    }
                    ui.add(
                        egui::Slider::new(
                            &mut panel.draft.font_size,
                            MIN_FONT_SIZE..=MAX_FONT_SIZE,
                        )
                        .show_value(false),
                    );
                    ui.label(
                        egui::RichText::new(format!("{:.0} px", panel.draft.font_size))
                            .size(13.0)
                            .color(TEXT),
                    );
                    if small_button(ui, "+", panel.draft.font_size < MAX_FONT_SIZE).clicked() {
                        panel.draft.font_size = step_font_size(panel.draft.font_size, 1.0);
                    }
                });
            });
        },
    );
    ui.add_space(14.0);

    card(
        ui,
        "终端配色",
        "搜索并预览 191 套内置终端主题。",
        |ui| {
            ui.add_sized(
                [ui.available_width().min(420.0), 36.0],
                egui::TextEdit::singleline(&mut panel.theme_filter)
                    .hint_text("搜索配色方案…")
                    .vertical_align(egui::Align::Center)
                    .margin(egui::Margin::symmetric(10, 7)),
            );
            ui.add_space(10.0);
            let filter = panel.theme_filter.trim().to_ascii_lowercase();
            let themes = all_themes()
                .iter()
                .filter(|theme| {
                    filter.is_empty() || theme.name.to_ascii_lowercase().contains(&filter)
                })
                .collect::<Vec<_>>();
            let columns = if ui.available_width() >= 700.0 { 2 } else { 1 };
            let rows = themes.len().div_ceil(columns);
            render_theme_catalog(ui, &themes, columns, rows, &mut panel.draft.theme);
        },
    );
    ui.add_space(14.0);

    card(
        ui,
        "终端行为",
        "控制光标状态与本地滚动历史。",
        |ui| {
            toggle_row(
                ui,
                "光标闪烁",
                "让当前输入位置更加醒目",
                &mut panel.draft.cursor_blink,
            );
            divider(ui);
            value_row(
                ui,
                "滚动历史",
                "终端保留的最大历史行数",
                |ui| {
                    ui.add(
                        egui::DragValue::new(&mut panel.draft.scrollback_lines)
                            .range(MIN_SCROLLBACK_LINES..=MAX_SCROLLBACK_LINES)
                            .suffix(" 行"),
                    );
                },
            );
        },
    );
}

fn render_appearance(ui: &mut egui::Ui, panel: &mut SettingsPanel) {
    card(
        ui,
        "工作区布局",
        "尺寸修改将在保存后应用。",
        |ui| {
            toggle_row(
                ui,
                "显示侧边栏",
                "连接与系统监控区域",
                &mut panel.draft.show_sidebar,
            );
            divider(ui);
            value_row(ui, "侧边栏宽度", "建议范围 200–320 px", |ui| {
                ui.add(egui::Slider::new(&mut panel.draft.sidebar_width, 180..=420).suffix(" px"));
            });
            divider(ui);
            toggle_row(
                ui,
                "显示文件管理器",
                "SSH 标签底部的本地/远端双栏",
                &mut panel.draft.show_file_browser,
            );
            divider(ui);
            value_row(
                ui,
                "文件管理器高度",
                "展开后的默认高度",
                |ui| {
                    ui.add(
                        egui::Slider::new(&mut panel.draft.file_browser_height, 120..=600)
                            .suffix(" px"),
                    );
                },
            );
        },
    );
}

fn render_shortcuts(ui: &mut egui::Ui, panel: &mut SettingsPanel) {
    card(
        ui,
        "键盘映射",
        "点击右侧按钮后直接按下新的组合键；Esc 取消录入。",
        |ui| {
            for action in ShortcutAction::all() {
                let capturing = panel.capturing_shortcut == Some(action);
                value_row(
                    ui,
                    action.label(),
                    panel.draft.shortcuts.binding(action),
                    |ui| {
                        let label = if capturing {
                            "等待按键…"
                        } else {
                            "重新录入"
                        };
                        if ui
                            .add(
                                egui::Button::new(
                                    egui::RichText::new(label).size(12.5).color(if capturing {
                                        CYAN
                                    } else {
                                        TEXT
                                    }),
                                )
                                .fill(if capturing { PANEL_RAISED } else { INPUT })
                                .stroke(egui::Stroke::new(
                                    1.0,
                                    if capturing { CYAN } else { BORDER },
                                ))
                                .corner_radius(6.0),
                            )
                            .clicked()
                        {
                            panel.capturing_shortcut = Some(action);
                            panel.error = None;
                            panel.feedback = None;
                        }
                    },
                );
                divider(ui);
            }
        },
    );
}

fn render_ssh(ui: &mut egui::Ui, panel: &mut SettingsPanel) {
    card(
        ui,
        "连接默认值",
        "仅影响保存后新建立的 SSH 会话。",
        |ui| {
            value_row(
                ui,
                "连接超时",
                "建立 TCP/SSH 会话的最长等待时间",
                |ui| {
                    ui.add(
                        egui::DragValue::new(&mut panel.draft.connect_timeout_secs)
                            .range(1..=300)
                            .suffix(" 秒"),
                    );
                },
            );
            divider(ui);
            value_row(ui, "Keepalive", "0 表示禁用应用层保活", |ui| {
                ui.add(
                    egui::DragValue::new(&mut panel.draft.keepalive_interval_secs)
                        .range(0..=3600)
                        .suffix(" 秒"),
                );
            });
            divider(ui);
            value_row(
                ui,
                "默认字符集",
                "新建连接使用的字符编码",
                |ui| {
                    egui::ComboBox::from_id_salt("settings_ssh_charset")
                        .selected_text(&panel.draft.default_charset)
                        .show_ui(ui, |ui| {
                            for charset in ["UTF-8", "GBK", "GB2312"] {
                                ui.selectable_value(
                                    &mut panel.draft.default_charset,
                                    charset.to_string(),
                                    charset,
                                );
                            }
                        });
                },
            );
        },
    );
}

fn render_transfer(ui: &mut egui::Ui, panel: &mut SettingsPanel) {
    card(
        ui,
        "传输策略",
        "适用于文件管理器与批量传输任务。",
        |ui| {
            setting_label(ui, "默认下载目录", "本地接收文件的默认位置");
            path_picker(
                ui,
                &mut panel.draft.default_download_dir,
                "选择默认下载目录",
            );
            divider(ui);
            value_row(
                ui,
                "断点续传阈值",
                "超过该大小时启用续传检查",
                |ui| {
                    ui.add(
                        egui::DragValue::new(&mut panel.draft.resume_threshold_mb)
                            .range(0..=102_400)
                            .suffix(" MB"),
                    );
                },
            );
            divider(ui);
            value_row(
                ui,
                "最大重试次数",
                "单个任务失败后的自动重试",
                |ui| {
                    ui.add(egui::DragValue::new(&mut panel.draft.max_retries).range(0..=20));
                },
            );
            divider(ui);
            value_row(ui, "并发传输数", "同时运行的文件任务", |ui| {
                ui.add(egui::DragValue::new(&mut panel.draft.concurrent_transfers).range(1..=8));
            });
        },
    );
}

fn render_zmodem(ui: &mut egui::Ui, panel: &mut SettingsPanel) {
    card(
        ui,
        "接收与检测",
        "ZMODEM 在终端输出中识别 sz/rz 协议帧。",
        |ui| {
            toggle_row(
                ui,
                "启用 ZMODEM",
                "允许终端发送和接收文件",
                &mut panel.draft.zmodem_enabled,
            );
            divider(ui);
            ui.add_enabled_ui(panel.draft.zmodem_enabled, |ui| {
                toggle_row(
                    ui,
                    "自动接收",
                    "检测远端 sz 后打开接收流程",
                    &mut panel.draft.zmodem_auto_detect,
                );
                divider(ui);
                setting_label(ui, "下载目录", "支持使用 ~/Downloads 形式");
                path_picker(
                    ui,
                    &mut panel.draft.zmodem_download_dir,
                    "选择 ZMODEM 下载目录",
                );
                divider(ui);
                value_row(ui, "传输超时", "无数据活动时中止任务", |ui| {
                    ui.add(
                        egui::DragValue::new(&mut panel.draft.zmodem_timeout_secs)
                            .range(MIN_ZMODEM_TIMEOUT_SECS..=MAX_ZMODEM_TIMEOUT_SECS)
                            .suffix(" 秒"),
                    );
                });
            });
        },
    );
}

fn render_about(ui: &mut egui::Ui) {
    card(
        ui,
        "LiteTerm Native",
        "轻量、跨平台的多协议终端工作区。",
        |ui| {
            ui.horizontal(|ui| {
                let (logo, _) =
                    ui.allocate_exact_size(egui::vec2(58.0, 58.0), egui::Sense::hover());
                ui.painter().rect(
                    logo,
                    12.0,
                    egui::Color32::from_rgb(0x17, 0x24, 0x2a),
                    egui::Stroke::new(1.0, egui::Color32::from_rgb(0x2c, 0x42, 0x4a)),
                    egui::epaint::StrokeKind::Inside,
                );
                ui.painter().text(
                    logo.center(),
                    egui::Align2::CENTER_CENTER,
                    ">_",
                    egui::FontId::monospace(20.0),
                    CYAN,
                );
                ui.vertical(|ui| {
                    ui.label(
                        egui::RichText::new(format!("版本 {}", env!("CARGO_PKG_VERSION")))
                            .size(15.0)
                            .color(TEXT),
                    );
                    ui.label(
                        egui::RichText::new(format!(
                            "{} · {}",
                            std::env::consts::OS,
                            std::env::consts::ARCH
                        ))
                        .size(13.0)
                        .color(MUTED),
                    );
                    ui.label(
                        egui::RichText::new("Rust · WGPU · egui")
                            .size(12.0)
                            .color(DIM),
                    );
                });
            });
            ui.add_space(14.0);
            ui.label(
            egui::RichText::new("配置保存在用户配置目录中，写入采用临时文件与原子替换，避免异常退出造成配置损坏。")
                .size(13.0)
                .color(MUTED),
        );
        },
    );
}

fn render_theme_catalog(
    ui: &mut egui::Ui,
    themes: &[&crate::themes::TerminalTheme],
    columns: usize,
    rows: usize,
    selected_theme: &mut String,
) {
    const ROW_HEIGHT: f32 = 132.0;
    for row in 0..rows {
        let predicted = egui::Rect::from_min_size(
            ui.next_widget_position(),
            egui::vec2(ui.available_width(), ROW_HEIGHT),
        );
        if ui.clip_rect().expand(ROW_HEIGHT).intersects(predicted) {
            ui.columns(columns, |column_ui| {
                for (column, target) in column_ui.iter_mut().enumerate() {
                    let index = row * columns + column;
                    if let Some(theme) = themes.get(index) {
                        if theme_preview(target, theme, *selected_theme == theme.name) {
                            *selected_theme = theme.name.to_string();
                        }
                    } else {
                        target.allocate_space(egui::vec2(target.available_width(), ROW_HEIGHT));
                    }
                }
            });
        } else {
            ui.allocate_space(egui::vec2(ui.available_width(), ROW_HEIGHT));
        }
    }
}

fn theme_preview(ui: &mut egui::Ui, theme: &crate::themes::TerminalTheme, selected: bool) -> bool {
    let width = ui.available_width().max(180.0);
    let (rect, response) = ui.allocate_exact_size(egui::vec2(width, 132.0), egui::Sense::click());
    let stroke = if selected || response.hovered() {
        egui::Stroke::new(1.0, if selected { CYAN } else { BORDER_HOVER })
    } else {
        egui::Stroke::new(1.0, BORDER)
    };
    ui.painter().rect(
        rect,
        8.0,
        PANEL_RAISED,
        stroke,
        egui::epaint::StrokeKind::Inside,
    );
    ui.painter().text(
        egui::pos2(rect.left() + 12.0, rect.top() + 16.0),
        egui::Align2::LEFT_CENTER,
        if selected {
            format!("✓ {}", theme.name)
        } else {
            theme.name.to_string()
        },
        egui::FontId::proportional(13.0),
        if selected { CYAN } else { TEXT },
    );
    let dot_y = rect.top() + 16.0;
    for (index, color) in theme.ansi.iter().take(8).enumerate() {
        let x = rect.right() - 12.0 - (7 - index) as f32 * 10.0;
        ui.painter()
            .circle_filled(egui::pos2(x, dot_y), 3.0, rgb_to_color32(*color));
    }
    let preview = egui::Rect::from_min_max(
        egui::pos2(rect.left() + 8.0, rect.top() + 32.0),
        egui::pos2(rect.right() - 8.0, rect.bottom() - 8.0),
    );
    ui.painter()
        .rect_filled(preview, 5.0, rgb_to_color32(theme.background));
    let font = egui::FontId::monospace(12.0);
    ui.painter().text(
        egui::pos2(preview.left() + 10.0, preview.top() + 18.0),
        egui::Align2::LEFT_CENTER,
        "lfl@host:~/workspace$ ls -al",
        font.clone(),
        rgb_to_color32(theme.ansi[2]),
    );
    ui.painter().text(
        egui::pos2(preview.left() + 10.0, preview.top() + 42.0),
        egui::Align2::LEFT_CENTER,
        "drwxr-xr-x  Documents",
        font.clone(),
        rgb_to_color32(theme.foreground),
    );
    ui.painter().text(
        egui::pos2(preview.left() + 10.0, preview.top() + 66.0),
        egui::Align2::LEFT_CENTER,
        "-rw-r--r--  README.md",
        font,
        rgb_to_color32(theme.ansi[6]),
    );
    response.clicked()
}

fn handle_shortcut_capture(panel: &mut SettingsPanel, ctx: &egui::Context) {
    if panel.capturing_shortcut.is_none() {
        return;
    }
    let mut captured = None;
    ctx.input(|input| {
        for event in &input.events {
            if let egui::Event::Key {
                key,
                pressed,
                repeat,
                modifiers,
                ..
            } = event
            {
                if !*pressed || *repeat {
                    continue;
                }
                captured = Some(if *key == egui::Key::Escape {
                    None
                } else {
                    Some(shortcut_from_egui_key(*key, *modifiers))
                });
                break;
            }
        }
    });
    match captured {
        Some(None) => {
            panel.capturing_shortcut = None;
            panel.error = None;
        }
        Some(Some(Ok(binding))) => {
            if let Some(action) = panel.capturing_shortcut.take() {
                panel.draft.shortcuts.set_binding(action, binding);
            }
            panel.error = None;
            panel.feedback = None;
        }
        Some(Some(Err(error))) => panel.set_error(error),
        None => {}
    }
}

fn setting_label(ui: &mut egui::Ui, title: &str, description: &str) {
    ui.label(egui::RichText::new(title).size(14.0).color(TEXT));
    ui.label(egui::RichText::new(description).size(11.5).color(MUTED));
    ui.add_space(6.0);
}

fn font_family_picker(ui: &mut egui::Ui, panel: &mut SettingsPanel) {
    let selected = panel.draft.font_family.clone();
    egui::ComboBox::from_id_salt("settings_terminal_font_family")
        .selected_text(egui::RichText::new(selected).size(14.0).color(TEXT))
        .width(ui.available_width())
        .height(320.0)
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
        .show_ui(ui, |ui| {
            ui.set_min_width(320.0_f32.min(ui.available_width().max(220.0)));
            ui.add_sized(
                [ui.available_width(), 34.0],
                egui::TextEdit::singleline(&mut panel.font_filter)
                    .hint_text("搜索已安装的等宽字体…")
                    .vertical_align(egui::Align::Center)
                    .margin(egui::Margin::symmetric(9, 6)),
            );
            ui.separator();

            let filter = panel.font_filter.trim().to_lowercase();
            let visible = panel
                .font_families
                .iter()
                .filter(|family| filter.is_empty() || family.to_lowercase().contains(&filter))
                .cloned()
                .collect::<Vec<_>>();
            if visible.is_empty() {
                ui.label(
                    egui::RichText::new("没有匹配的系统等宽字体")
                        .size(12.0)
                        .color(MUTED),
                );
            } else {
                for family in visible {
                    if ui
                        .selectable_label(panel.draft.font_family == family, &family)
                        .clicked()
                    {
                        panel.draft.font_family = family;
                        panel.font_filter.clear();
                        ui.close_menu();
                    }
                }
            }

            ui.separator();
            ui.label(
                egui::RichText::new("自定义字体族（系统列表未收录时使用）")
                    .size(11.5)
                    .color(MUTED),
            );
            ui.add_sized(
                [ui.available_width(), 34.0],
                egui::TextEdit::singleline(&mut panel.draft.font_family)
                    .hint_text("例如 Ubuntu Mono")
                    .vertical_align(egui::Align::Center)
                    .margin(egui::Margin::symmetric(9, 6)),
            );
        });
}

fn value_row(
    ui: &mut egui::Ui,
    title: &str,
    description: &str,
    add_value: impl FnOnce(&mut egui::Ui),
) {
    ui.horizontal(|ui| {
        ui.vertical(|ui| setting_label(ui, title, description));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), add_value);
    });
}

fn toggle_row(ui: &mut egui::Ui, title: &str, description: &str, value: &mut bool) {
    value_row(ui, title, description, |ui| {
        let (rect, response) = ui.allocate_exact_size(egui::vec2(40.0, 22.0), egui::Sense::click());
        if response.clicked() {
            *value = !*value;
        }
        ui.painter().rect_filled(
            rect,
            11.0,
            if *value {
                CYAN
            } else {
                egui::Color32::from_rgb(0x30, 0x39, 0x45)
            },
        );
        let center_x = if *value {
            rect.right() - 11.0
        } else {
            rect.left() + 11.0
        };
        ui.painter().circle_filled(
            egui::pos2(center_x, rect.center().y),
            8.0,
            if *value { BACKGROUND } else { MUTED },
        );
    });
}

fn divider(ui: &mut egui::Ui) {
    ui.add_space(7.0);
    let rect = ui.available_rect_before_wrap();
    ui.painter().line_segment(
        [
            egui::pos2(rect.left(), rect.top()),
            egui::pos2(rect.right(), rect.top()),
        ],
        egui::Stroke::new(1.0, egui::Color32::from_rgb(0x20, 0x28, 0x32)),
    );
    ui.add_space(8.0);
}

fn small_button(ui: &mut egui::Ui, label: &str, enabled: bool) -> egui::Response {
    ui.add_enabled(
        enabled,
        egui::Button::new(egui::RichText::new(label).size(14.0).color(TEXT))
            .fill(INPUT)
            .stroke(egui::Stroke::new(1.0, BORDER))
            .corner_radius(5.0)
            .min_size(egui::vec2(28.0, 26.0)),
    )
}

fn path_picker(ui: &mut egui::Ui, value: &mut String, title: &str) {
    ui.horizontal(|ui| {
        let button_width = 68.0;
        ui.add_sized(
            [(ui.available_width() - button_width - 8.0).max(100.0), 36.0],
            egui::TextEdit::singleline(value)
                .hint_text("选择一个已存在的目录")
                .vertical_align(egui::Align::Center)
                .margin(egui::Margin::symmetric(10, 7)),
        );
        if ui
            .add_sized([button_width, 36.0], egui::Button::new("浏览…"))
            .clicked()
        {
            let current = shellexpand::tilde(value).into_owned();
            let mut dialog = rfd::FileDialog::new().set_title(title);
            if std::path::Path::new(&current).is_dir() {
                dialog = dialog.set_directory(current);
            }
            if let Some(path) = dialog.pick_folder() {
                *value = path.to_string_lossy().into_owned();
            }
        }
    });
}
