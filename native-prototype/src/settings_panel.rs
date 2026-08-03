use crate::settings::{
    resolve_existing_download_directory, validate_zmodem_timeout, Settings,
    MAX_ZMODEM_TIMEOUT_SECS, MIN_ZMODEM_TIMEOUT_SECS,
};
use crate::shortcuts::{KeyChord, ShortcutAction, ShortcutSettings};
use crate::themes::{all_themes, theme_by_name};

mod ui;

const MIN_FONT_SIZE: f32 = 8.0;
const MAX_FONT_SIZE: f32 = 48.0;
const MIN_SCROLLBACK_LINES: u32 = 100;
const MAX_SCROLLBACK_LINES: u32 = 1_000_000;

fn step_font_size(current: f32, delta: f32) -> f32 {
    (current + delta).clamp(MIN_FONT_SIZE, MAX_FONT_SIZE)
}

/// 设置面板编辑草稿（未持久化）
#[derive(Debug, Clone, PartialEq)]
pub struct SettingsDraft {
    pub font_family: String,
    pub font_size: f32,
    pub theme: String,
    pub scrollback_lines: u32,
    pub cursor_blink: bool,
    pub sidebar_width: u32,
    pub file_browser_height: u32,
    pub show_sidebar: bool,
    pub show_file_browser: bool,
    pub default_download_dir: String,
    pub resume_threshold_mb: u32,
    pub max_retries: u32,
    pub concurrent_transfers: u32,
    pub keepalive_interval_secs: u32,
    pub connect_timeout_secs: u32,
    pub default_charset: String,
    pub zmodem_enabled: bool,
    pub zmodem_auto_detect: bool,
    pub zmodem_download_dir: String,
    pub zmodem_timeout_secs: u32,
    pub shortcuts: ShortcutSettings,
}

impl From<&Settings> for SettingsDraft {
    fn from(settings: &Settings) -> Self {
        Self {
            font_family: settings.terminal.font.clone(),
            font_size: settings.terminal.font_size,
            theme: settings.terminal.color_scheme.clone(),
            scrollback_lines: settings.terminal.scrollback_lines,
            cursor_blink: settings.terminal.cursor_blink,
            sidebar_width: settings.appearance.sidebar_width,
            file_browser_height: settings.appearance.file_browser_height,
            show_sidebar: settings.appearance.show_sidebar,
            show_file_browser: settings.appearance.show_file_browser,
            default_download_dir: settings.transfer.default_download_dir.clone(),
            resume_threshold_mb: settings.transfer.resume_threshold_mb,
            max_retries: settings.transfer.max_retries,
            concurrent_transfers: settings.transfer.concurrent_transfers,
            keepalive_interval_secs: settings.ssh.keepalive_interval_secs,
            connect_timeout_secs: settings.ssh.connect_timeout_secs,
            default_charset: settings.ssh.default_charset.clone(),
            zmodem_enabled: settings.zmodem.enabled,
            zmodem_auto_detect: settings.zmodem.auto_detect,
            zmodem_download_dir: settings.zmodem.download_dir.clone(),
            zmodem_timeout_secs: settings.zmodem.timeout_secs,
            shortcuts: settings.shortcuts.clone(),
        }
    }
}

impl SettingsDraft {
    pub fn validate(&self) -> Result<(), String> {
        if !self.font_size.is_finite() || !(MIN_FONT_SIZE..=MAX_FONT_SIZE).contains(&self.font_size)
        {
            return Err("字号必须在 8 到 48 之间".to_string());
        }
        if !(MIN_SCROLLBACK_LINES..=MAX_SCROLLBACK_LINES).contains(&self.scrollback_lines) {
            return Err(format!(
                "滚动历史必须在 {MIN_SCROLLBACK_LINES} 到 {MAX_SCROLLBACK_LINES} 行之间"
            ));
        }
        if !(180..=420).contains(&self.sidebar_width) {
            return Err("侧边栏宽度必须在 180 到 420 之间".into());
        }
        if !(120..=600).contains(&self.file_browser_height) {
            return Err("文件管理器高度必须在 120 到 600 之间".into());
        }
        resolve_existing_download_directory(&self.default_download_dir)
            .map_err(|error| format!("默认下载目录无效：{error}"))?;
        if !(1..=8).contains(&self.concurrent_transfers) {
            return Err("并发传输数必须在 1 到 8 之间".into());
        }
        if self.connect_timeout_secs == 0 || self.connect_timeout_secs > 300 {
            return Err("SSH 连接超时必须在 1 到 300 秒之间".into());
        }
        if self.keepalive_interval_secs > 3600 {
            return Err("SSH Keepalive 间隔不能超过 3600 秒".into());
        }
        if !matches!(self.default_charset.as_str(), "UTF-8" | "GBK" | "GB2312") {
            return Err("SSH 默认字符集无效".into());
        }
        validate_zmodem_timeout(self.zmodem_timeout_secs)?;
        resolve_existing_download_directory(&self.zmodem_download_dir)
            .map_err(|error| format!("ZMODEM 下载目录无效：{error}"))?;
        self.shortcuts.validate()
    }
}

#[derive(Debug, Clone)]
pub enum SettingsPanelAction {
    None,
    Apply(Settings),
    Cancel,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
enum SettingsSection {
    #[default]
    Terminal,
    Appearance,
    Shortcuts,
    Ssh,
    Transfer,
    Zmodem,
    About,
}

#[derive(Debug, Clone)]
pub struct SettingsPanel {
    pub visible: bool,
    draft: SettingsDraft,
    error: Option<String>,
    theme_filter: String,
    font_filter: String,
    font_families: Vec<String>,
    capturing_shortcut: Option<ShortcutAction>,
    base: Settings,
    section: SettingsSection,
    feedback: Option<String>,
}

impl Default for SettingsPanel {
    fn default() -> Self {
        let base = Settings::default();
        Self {
            visible: false,
            draft: SettingsDraft::from(&base),
            error: None,
            theme_filter: String::new(),
            font_filter: String::new(),
            font_families: Vec::new(),
            capturing_shortcut: None,
            base,
            section: SettingsSection::Terminal,
            feedback: None,
        }
    }
}

impl SettingsPanel {
    pub fn open(&mut self, current: &Settings) {
        self.base = current.clone();
        self.draft = SettingsDraft::from(current);
        self.error = None;
        self.theme_filter.clear();
        self.font_filter.clear();
        self.capturing_shortcut = None;
        self.section = SettingsSection::Terminal;
        self.feedback = None;
        self.visible = true;
    }

    pub fn close(&mut self) {
        self.visible = false;
        self.capturing_shortcut = None;
    }

    pub fn set_font_families(&mut self, families: impl IntoIterator<Item = String>) {
        let mut families = families
            .into_iter()
            .map(|family| family.trim().to_string())
            .filter(|family| !family.is_empty())
            .collect::<std::collections::BTreeSet<_>>();
        let configured = self.draft.font_family.trim();
        if !configured.is_empty() {
            families.insert(configured.to_string());
        }
        self.font_families = families.into_iter().collect();
    }

    pub fn set_error(&mut self, message: String) {
        self.error = Some(message);
        self.feedback = None;
    }

    pub fn mark_saved(&mut self, settings: &Settings) {
        self.base = settings.clone();
        self.draft = SettingsDraft::from(settings);
        self.error = None;
        self.feedback = Some("设置已保存并应用".into());
    }

    fn reset_draft(&mut self) {
        self.draft = SettingsDraft::from(&self.base);
        self.error = None;
        self.feedback = Some("未保存的修改已撤销".into());
        self.capturing_shortcut = None;
    }

    fn is_dirty(&self) -> bool {
        self.draft != SettingsDraft::from(&self.base)
    }

    pub fn build_settings(&self, base: &Settings) -> Result<Settings, String> {
        self.draft.validate()?;

        let font = self.draft.font_family.trim();
        if font.is_empty() {
            return Err("字体族不能为空".to_string());
        }

        if theme_by_name(&self.draft.theme).is_none() {
            return Err(format!("未找到配色方案：{}", self.draft.theme));
        }

        let mut settings = base.clone();
        settings.terminal.font = font.to_string();
        settings.terminal.font_size = self.draft.font_size;
        settings.terminal.color_scheme = self.draft.theme.clone();
        settings.terminal.scrollback_lines = self.draft.scrollback_lines;
        settings.terminal.cursor_blink = self.draft.cursor_blink;
        settings.appearance.sidebar_width = self.draft.sidebar_width;
        settings.appearance.file_browser_height = self.draft.file_browser_height;
        settings.appearance.show_sidebar = self.draft.show_sidebar;
        settings.appearance.show_file_browser = self.draft.show_file_browser;
        settings.transfer.default_download_dir = self.draft.default_download_dir.trim().into();
        settings.transfer.resume_threshold_mb = self.draft.resume_threshold_mb;
        settings.transfer.max_retries = self.draft.max_retries;
        settings.transfer.concurrent_transfers = self.draft.concurrent_transfers;
        settings.ssh.keepalive_interval_secs = self.draft.keepalive_interval_secs;
        settings.ssh.connect_timeout_secs = self.draft.connect_timeout_secs;
        settings.ssh.default_charset = self.draft.default_charset.clone();
        settings.zmodem.enabled = self.draft.zmodem_enabled;
        settings.zmodem.auto_detect = self.draft.zmodem_auto_detect;
        settings.zmodem.download_dir = self.draft.zmodem_download_dir.trim().to_string();
        settings.zmodem.timeout_secs = self.draft.zmodem_timeout_secs;
        settings.shortcuts = self.draft.shortcuts.clone();
        Ok(settings)
    }

    pub fn show_page(&mut self, ctx: &egui::Context) -> SettingsPanelAction {
        ui::show_page(self, ctx)
    }

    pub fn show(&mut self, ctx: &egui::Context) -> SettingsPanelAction {
        if !self.visible {
            return SettingsPanelAction::None;
        }

        // 本帧是否已由捕获逻辑消费 Escape（防止同帧通用 Escape 再关面板）
        let mut escape_consumed_by_capture = false;

        // 快捷键捕获：仅 pressed 且非 repeat
        if self.capturing_shortcut.is_some() {
            let mut cancel_capture = false;
            let mut captured: Option<Result<String, String>> = None;

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
                        if *key == egui::Key::Escape {
                            cancel_capture = true;
                            break;
                        }
                        captured = Some(shortcut_from_egui_key(*key, *modifiers));
                        break;
                    }
                }
            });

            if cancel_capture {
                self.capturing_shortcut = None;
                self.error = None;
                escape_consumed_by_capture = true;
            } else if let Some(result) = captured {
                match result {
                    Ok(binding) => {
                        if let Some(action) = self.capturing_shortcut.take() {
                            self.draft.shortcuts.set_binding(action, binding);
                        }
                        self.error = None;
                    }
                    Err(err) => {
                        self.error = Some(err);
                    }
                }
            }
        }

        let screen = ctx.input(|i| i.screen_rect());

        egui::Area::new(egui::Id::new("settings_panel_backdrop"))
            .order(egui::Order::Foreground)
            .fixed_pos(screen.min)
            .sense(egui::Sense::click())
            .show(ctx, |ui| {
                let (rect, _) = ui.allocate_exact_size(screen.size(), egui::Sense::click());
                ui.painter().rect_filled(
                    rect,
                    0.0,
                    egui::Color32::from_rgba_unmultiplied(0, 0, 0, 120),
                );
            });

        let mut open = true;
        let mut action = SettingsPanelAction::None;
        let mut cancel_clicked = false;
        let mut save_clicked = false;
        let capturing = self.capturing_shortcut;

        // 预取主题列表过滤（避免在闭包中反复借用 self 冲突）
        let filter_lower = self.theme_filter.to_ascii_lowercase();

        egui::Window::new("设置")
            .id(egui::Id::new("settings_panel_window"))
            .order(egui::Order::Foreground)
            .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
            .default_size(egui::vec2(640.0, 560.0))
            .min_size(egui::vec2(420.0, 360.0))
            .max_height((screen.height() - 40.0).max(320.0))
            .max_width((screen.width() - 40.0).max(400.0))
            .resizable(true)
            .collapsible(false)
            .open(&mut open)
            .show(ctx, |ui| {
                ui.heading("终端外观");
                ui.add_space(6.0);

                egui::Grid::new("settings_font_grid")
                    .num_columns(2)
                    .spacing([12.0, 8.0])
                    .show(ui, |ui| {
                        ui.label("字体族");
                        ui.add(
                            egui::TextEdit::singleline(&mut self.draft.font_family)
                                .desired_width(280.0)
                                .hint_text("例如 Ubuntu Mono"),
                        );
                        ui.end_row();

                        ui.label("字号");
                        ui.horizontal(|ui| {
                            let decrease = ui
                                .add_enabled(
                                    self.draft.font_size > MIN_FONT_SIZE,
                                    egui::Button::new(egui::RichText::new("−").size(16.0))
                                        .min_size(egui::vec2(28.0, 24.0)),
                                )
                                .on_hover_text("字号减小 1px");
                            if decrease.clicked() {
                                self.draft.font_size = step_font_size(self.draft.font_size, -1.0);
                            }

                            ui.add(
                                egui::Slider::new(
                                    &mut self.draft.font_size,
                                    MIN_FONT_SIZE..=MAX_FONT_SIZE,
                                )
                                .show_value(false),
                            );

                            let increase = ui
                                .add_enabled(
                                    self.draft.font_size < MAX_FONT_SIZE,
                                    egui::Button::new(egui::RichText::new("+").size(16.0))
                                        .min_size(egui::vec2(28.0, 24.0)),
                                )
                                .on_hover_text("字号增大 1px");
                            if increase.clicked() {
                                self.draft.font_size = step_font_size(self.draft.font_size, 1.0);
                            }

                            ui.add(
                                egui::DragValue::new(&mut self.draft.font_size)
                                    .range(MIN_FONT_SIZE..=MAX_FONT_SIZE)
                                    .speed(0.5)
                                    .suffix(" px"),
                            );
                        });
                        ui.end_row();
                    });

                ui.add_space(10.0);
                ui.heading("配色方案");
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    ui.label("搜索");
                    ui.add(
                        egui::TextEdit::singleline(&mut self.theme_filter)
                            .desired_width(240.0)
                            .hint_text("过滤主题名称"),
                    );
                });
                ui.add_space(4.0);

                let theme_height = (ui.available_height() * 0.45).clamp(140.0, 240.0);
                egui::ScrollArea::vertical()
                    .id_salt("settings_theme_list")
                    .max_height(theme_height)
                    .show(ui, |ui| {
                        for theme in all_themes() {
                            if !filter_lower.is_empty()
                                && !theme.name.to_ascii_lowercase().contains(&filter_lower)
                            {
                                continue;
                            }

                            let selected = self.draft.theme == theme.name;
                            ui.horizontal(|ui| {
                                let (swatch_rect, _) = ui.allocate_exact_size(
                                    egui::vec2(18.0, 18.0),
                                    egui::Sense::hover(),
                                );
                                ui.painter().rect_filled(
                                    swatch_rect,
                                    2.0,
                                    rgb_to_color32(theme.background),
                                );
                                ui.painter().rect_stroke(
                                    swatch_rect,
                                    2.0,
                                    egui::Stroke::new(1.0, rgb_to_color32(theme.foreground)),
                                    egui::epaint::StrokeKind::Outside,
                                );

                                let (fg_rect, _) = ui.allocate_exact_size(
                                    egui::vec2(12.0, 12.0),
                                    egui::Sense::hover(),
                                );
                                ui.painter().rect_filled(
                                    fg_rect,
                                    2.0,
                                    rgb_to_color32(theme.foreground),
                                );

                                for i in 0..4 {
                                    let (ansi_rect, _) = ui.allocate_exact_size(
                                        egui::vec2(10.0, 10.0),
                                        egui::Sense::hover(),
                                    );
                                    ui.painter().rect_filled(
                                        ansi_rect,
                                        1.0,
                                        rgb_to_color32(theme.ansi[i]),
                                    );
                                }

                                let label = if selected {
                                    format!("● {}", theme.name)
                                } else {
                                    format!("○ {}", theme.name)
                                };
                                if ui.selectable_label(selected, label).clicked() {
                                    self.draft.theme = theme.name.to_string();
                                }
                            });
                        }
                    });

                ui.add_space(10.0);
                ui.heading("ZMODEM 文件传输");
                ui.add_space(4.0);
                egui::Grid::new("settings_zmodem_grid")
                    .num_columns(2)
                    .spacing([12.0, 6.0])
                    .show(ui, |ui| {
                        ui.label("启用");
                        ui.checkbox(&mut self.draft.zmodem_enabled, "允许发送和接收");
                        ui.end_row();

                        ui.label("自动接收");
                        ui.add_enabled(
                            self.draft.zmodem_enabled,
                            egui::Checkbox::new(&mut self.draft.zmodem_auto_detect, "检测远端 sz"),
                        );
                        ui.end_row();

                        ui.label("下载目录");
                        ui.horizontal(|ui| {
                            ui.add_enabled(
                                self.draft.zmodem_enabled,
                                egui::TextEdit::singleline(&mut self.draft.zmodem_download_dir)
                                    .desired_width(210.0)
                                    .hint_text("~/Downloads"),
                            )
                            .on_hover_text("支持 ~；保存时要求展开后为绝对路径");
                            if ui
                                .add_enabled(self.draft.zmodem_enabled, egui::Button::new("浏览…"))
                                .clicked()
                            {
                                let current = shellexpand::tilde(&self.draft.zmodem_download_dir)
                                    .into_owned();
                                let mut dialog =
                                    rfd::FileDialog::new().set_title("选择 ZMODEM 下载目录");
                                if std::path::Path::new(&current).is_dir() {
                                    dialog = dialog.set_directory(current);
                                }
                                if let Some(path) = dialog.pick_folder() {
                                    self.draft.zmodem_download_dir =
                                        path.to_string_lossy().into_owned();
                                }
                            }
                        });
                        ui.end_row();

                        ui.label("超时");
                        ui.add_enabled(
                            self.draft.zmodem_enabled,
                            egui::DragValue::new(&mut self.draft.zmodem_timeout_secs)
                                .range(MIN_ZMODEM_TIMEOUT_SECS..=MAX_ZMODEM_TIMEOUT_SECS)
                                .speed(1.0)
                                .suffix(" 秒"),
                        );
                        ui.end_row();
                    });

                ui.add_space(10.0);
                ui.heading("快捷键");
                ui.add_space(4.0);

                for action in ShortcutAction::all() {
                    ui.horizontal(|ui| {
                        ui.label(action.label());
                        ui.label(self.draft.shortcuts.binding(action));
                        let is_capturing = capturing == Some(action);
                        let button_text = if is_capturing {
                            "按键中…"
                        } else {
                            "点击录入"
                        };
                        if ui.button(button_text).clicked() {
                            self.capturing_shortcut = Some(action);
                            self.error = None;
                        }
                    });
                }

                ui.add_space(8.0);
                if let Some(err) = &self.error {
                    ui.colored_label(egui::Color32::from_rgb(220, 80, 80), err);
                } else {
                    ui.add_space(16.0);
                }

                ui.separator();
                ui.horizontal(|ui| {
                    if ui.button("取消").clicked() {
                        cancel_clicked = true;
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button("保存").clicked() {
                            save_clicked = true;
                        }
                    });
                });
            });

        if !open || cancel_clicked {
            self.close();
            return SettingsPanelAction::Cancel;
        }

        // Escape：捕获中已在上方处理；非捕获且本帧未消费则关闭
        if self.capturing_shortcut.is_none()
            && !escape_consumed_by_capture
            && ctx.input(|i| i.key_pressed(egui::Key::Escape))
        {
            self.close();
            return SettingsPanelAction::Cancel;
        }

        if save_clicked {
            match self.build_settings(&self.base) {
                Ok(settings) => {
                    action = SettingsPanelAction::Apply(settings);
                }
                Err(err) => {
                    self.error = Some(err);
                }
            }
        }

        action
    }
}

fn rgb_to_color32(rgb: [u8; 3]) -> egui::Color32 {
    egui::Color32::from_rgb(rgb[0], rgb[1], rgb[2])
}

/// 将 egui 按键 + 修饰键映射为规范快捷键字符串（经 KeyChord Display）。
/// 仅 `mac_cmd` 映射为 Super；Linux/Windows 上的 `command`（等同 Ctrl）不计入 Super。
pub fn shortcut_from_egui_key(
    key: egui::Key,
    modifiers: egui::Modifiers,
) -> Result<String, String> {
    let key_token = match key {
        egui::Key::A => "A",
        egui::Key::B => "B",
        egui::Key::C => "C",
        egui::Key::D => "D",
        egui::Key::E => "E",
        egui::Key::F => "F",
        egui::Key::G => "G",
        egui::Key::H => "H",
        egui::Key::I => "I",
        egui::Key::J => "J",
        egui::Key::K => "K",
        egui::Key::L => "L",
        egui::Key::M => "M",
        egui::Key::N => "N",
        egui::Key::O => "O",
        egui::Key::P => "P",
        egui::Key::Q => "Q",
        egui::Key::R => "R",
        egui::Key::S => "S",
        egui::Key::T => "T",
        egui::Key::U => "U",
        egui::Key::V => "V",
        egui::Key::W => "W",
        egui::Key::X => "X",
        egui::Key::Y => "Y",
        egui::Key::Z => "Z",
        egui::Key::Num0 => "0",
        egui::Key::Num1 => "1",
        egui::Key::Num2 => "2",
        egui::Key::Num3 => "3",
        egui::Key::Num4 => "4",
        egui::Key::Num5 => "5",
        egui::Key::Num6 => "6",
        egui::Key::Num7 => "7",
        egui::Key::Num8 => "8",
        egui::Key::Num9 => "9",
        egui::Key::Tab => "Tab",
        egui::Key::F1 => "F1",
        egui::Key::F2 => "F2",
        egui::Key::F3 => "F3",
        egui::Key::F4 => "F4",
        egui::Key::F5 => "F5",
        egui::Key::F6 => "F6",
        egui::Key::F7 => "F7",
        egui::Key::F8 => "F8",
        egui::Key::F9 => "F9",
        egui::Key::F10 => "F10",
        egui::Key::F11 => "F11",
        egui::Key::F12 => "F12",
        other => {
            return Err(format!("不支持的按键：{other:?}"));
        }
    };

    let chord = KeyChord {
        ctrl: modifiers.ctrl,
        shift: modifiers.shift,
        alt: modifiers.alt,
        // 仅 mac_cmd → Super；忽略 command，避免 Linux/Windows 将 Ctrl 误当 Super
        super_key: modifiers.mac_cmd,
        key: key_token.to_string(),
    };
    Ok(chord.to_string())
}

#[cfg(test)]
mod tests {
    use crate::settings::Settings;
    use crate::shortcuts::ShortcutAction;

    use super::{
        shortcut_from_egui_key, step_font_size, SettingsDraft, SettingsPanel, SettingsPanelAction,
    };

    #[test]
    fn font_size_buttons_step_one_pixel_and_clamp_to_slider_bounds() {
        assert_eq!(step_font_size(22.0, -1.0), 21.0);
        assert_eq!(step_font_size(22.0, 1.0), 23.0);
        assert_eq!(step_font_size(8.0, -1.0), 8.0);
        assert_eq!(step_font_size(48.0, 1.0), 48.0);
        assert_eq!(step_font_size(18.5, 1.0), 19.5);
    }

    #[test]
    fn settings_draft_rejects_font_size_above_max() {
        let settings = Settings::default();
        let mut draft = SettingsDraft::from(&settings);
        draft.font_size = 49.0;
        assert_eq!(draft.validate().unwrap_err(), "字号必须在 8 到 48 之间");
    }

    #[test]
    fn settings_draft_rejects_font_size_below_min() {
        let settings = Settings::default();
        let mut draft = SettingsDraft::from(&settings);
        draft.font_size = 7.5;
        assert_eq!(draft.validate().unwrap_err(), "字号必须在 8 到 48 之间");
    }

    #[test]
    fn settings_draft_accepts_font_size_boundaries() {
        let settings = Settings::default();

        let mut draft_min = SettingsDraft::from(&settings);
        draft_min.font_size = 8.0;
        assert!(draft_min.validate().is_ok(), "字号 8.0 应通过字号范围校验");

        let mut draft_max = SettingsDraft::from(&settings);
        draft_max.font_size = 48.0;
        assert!(draft_max.validate().is_ok(), "字号 48.0 应通过字号范围校验");
    }

    #[test]
    fn font_family_catalog_is_trimmed_sorted_deduplicated_and_keeps_configured_font() {
        let mut panel = SettingsPanel::default();
        panel.draft.font_family = "Custom Mono".into();

        panel.set_font_families([
            " Zeta Mono ".to_string(),
            "Alpha Mono".to_string(),
            "Zeta Mono".to_string(),
            "".to_string(),
        ]);

        assert_eq!(
            panel.font_families,
            vec!["Alpha Mono", "Custom Mono", "Zeta Mono"]
        );
    }

    #[test]
    fn settings_draft_validates_zmodem_timeout_and_download_dir() {
        let settings = Settings::default();
        let mut draft = SettingsDraft::from(&settings);
        draft.zmodem_download_dir = std::env::temp_dir().to_string_lossy().into_owned();
        draft.zmodem_timeout_secs = 5;
        assert!(draft.validate().is_ok());

        draft.zmodem_timeout_secs = 4;
        assert!(draft.validate().unwrap_err().contains("ZMODEM 超时"));

        draft.zmodem_timeout_secs = 60;
        draft.zmodem_download_dir = "relative/downloads".into();
        assert!(draft.validate().unwrap_err().contains("必须是绝对路径"));

        draft.zmodem_download_dir = "~".into();
        draft.zmodem_timeout_secs = 3600;
        assert!(draft.validate().is_ok());
    }

    #[test]
    fn settings_draft_rejects_duplicate_shortcuts() {
        let settings = Settings::default();
        let mut draft = SettingsDraft::from(&settings);
        draft.shortcuts.close_tab = draft.shortcuts.new_tab.clone();
        let err = draft.validate().unwrap_err();
        assert!(
            err.contains("快捷键冲突"),
            "期望快捷键冲突错误，实际：{err}"
        );
    }

    /// P0 Task 3 RED: open 后可见，draft 精确来自 Settings，瞬时 UI 状态清空；
    /// 再次 open 另一份 Settings 必须完全重置（不残留脏 draft）。
    #[test]
    fn open_sets_visible_draft_and_clears_transient_state_then_resets_on_reopen() {
        let mut panel = SettingsPanel::default();

        let mut settings_a = Settings::default();
        settings_a.terminal.font = "Noto Sans Mono".to_string();
        settings_a.terminal.font_size = 18.0;
        settings_a.terminal.color_scheme = "3024 Day".to_string();
        settings_a.zmodem.enabled = false;
        settings_a.zmodem.auto_detect = false;
        settings_a.zmodem.download_dir = "/var/tmp/zmodem-a".into();
        settings_a.zmodem.timeout_secs = 90;
        settings_a.shortcuts.search = "Ctrl+F".to_string();

        panel.open(&settings_a);

        assert!(panel.visible, "open 后 visible 应为 true");
        assert_eq!(panel.draft.font_family, "Noto Sans Mono");
        assert_eq!(panel.draft.font_size, 18.0);
        assert_eq!(panel.draft.theme, "3024 Day");
        assert!(!panel.draft.zmodem_enabled);
        assert!(!panel.draft.zmodem_auto_detect);
        assert_eq!(panel.draft.zmodem_download_dir, "/var/tmp/zmodem-a");
        assert_eq!(panel.draft.zmodem_timeout_secs, 90);
        assert_eq!(panel.draft.shortcuts.search, "Ctrl+F");
        assert_eq!(panel.draft.shortcuts, settings_a.shortcuts);
        assert!(panel.error.is_none(), "open 应清空 error");
        assert_eq!(panel.theme_filter, "", "open 应清空 theme_filter");
        assert!(
            panel.capturing_shortcut.is_none(),
            "open 应清空 capturing_shortcut"
        );

        // 污染 draft / 瞬时状态，验证再次 open 完全重置
        panel.draft.font_family = "Dirty Font".to_string();
        panel.draft.font_size = 11.0;
        panel.draft.theme = "DirtyTheme".to_string();
        panel.draft.zmodem_enabled = true;
        panel.draft.zmodem_auto_detect = true;
        panel.draft.zmodem_download_dir = "/dirty".into();
        panel.draft.zmodem_timeout_secs = 999;
        panel.draft.shortcuts.copy = "Ctrl+Alt+C".to_string();
        panel.error = Some("残留错误".to_string());
        panel.theme_filter = "day".to_string();
        panel.capturing_shortcut = Some(ShortcutAction::Search);
        panel.visible = false;

        let mut settings_b = Settings::default();
        settings_b.terminal.font = "Ubuntu Mono".to_string();
        settings_b.terminal.font_size = 26.0;
        settings_b.terminal.color_scheme = "AdventureTime".to_string();
        settings_b.zmodem.enabled = true;
        settings_b.zmodem.auto_detect = false;
        settings_b.zmodem.download_dir = "/var/tmp/zmodem-b".into();
        settings_b.zmodem.timeout_secs = 180;
        settings_b.shortcuts.paste = "Ctrl+V".to_string();

        panel.open(&settings_b);

        assert!(panel.visible, "再次 open 后 visible 应为 true");
        assert_eq!(panel.draft.font_family, settings_b.terminal.font);
        assert_eq!(panel.draft.font_size, settings_b.terminal.font_size);
        assert_eq!(panel.draft.theme, settings_b.terminal.color_scheme);
        assert_eq!(panel.draft.zmodem_enabled, settings_b.zmodem.enabled);
        assert_eq!(
            panel.draft.zmodem_auto_detect,
            settings_b.zmodem.auto_detect
        );
        assert_eq!(
            panel.draft.zmodem_download_dir,
            settings_b.zmodem.download_dir
        );
        assert_eq!(
            panel.draft.zmodem_timeout_secs,
            settings_b.zmodem.timeout_secs
        );
        assert_eq!(panel.draft.shortcuts, settings_b.shortcuts);
        assert!(panel.error.is_none(), "再次 open 应清空 error");
        assert_eq!(panel.theme_filter, "", "再次 open 应清空 theme_filter");
        assert!(
            panel.capturing_shortcut.is_none(),
            "再次 open 应清空 capturing_shortcut"
        );
    }

    /// build_settings 把终端/ZMODEM/快捷键草稿写回克隆 Settings，
    /// 保留 appearance/transfer/ssh；未知主题返回含「未找到配色方案」的错误。
    #[test]
    fn build_settings_writes_draft_preserves_other_sections_and_rejects_unknown_theme() {
        let mut base = Settings::default();
        base.appearance.theme = "CustomUI".to_string();
        base.appearance.sidebar_width = 300;
        base.transfer.max_retries = 9;
        base.ssh.connect_timeout_secs = 42;
        base.zmodem.timeout_secs = 120;
        base.terminal.font = "OldFont".to_string();
        base.terminal.font_size = 14.0;
        base.terminal.color_scheme = "AdventureTime".to_string();
        base.terminal.scrollback_lines = 5000;
        base.terminal.cursor_blink = false;
        base.shortcuts.search = "Ctrl+F".to_string();

        let mut panel = SettingsPanel::default();
        panel.open(&base);
        panel.draft.font_family = "Noto Sans Mono".to_string();
        panel.draft.font_size = 20.0;
        panel.draft.theme = "3024 Day".to_string();
        panel.draft.zmodem_enabled = false;
        panel.draft.zmodem_auto_detect = false;
        let existing_download_dir = std::env::temp_dir().to_string_lossy().into_owned();
        panel.draft.zmodem_download_dir = format!("  {existing_download_dir}  ");
        panel.draft.zmodem_timeout_secs = 240;
        panel.draft.shortcuts.search = "Ctrl+F".to_string();
        panel.draft.shortcuts.copy = "Ctrl+C".to_string();

        let built = panel
            .build_settings(&base)
            .expect("已知主题 3024 Day 应成功 build_settings");

        assert_eq!(built.terminal.font, "Noto Sans Mono");
        assert_eq!(built.terminal.font_size, 20.0);
        assert_eq!(built.terminal.color_scheme, "3024 Day");
        assert_eq!(built.shortcuts.search, "Ctrl+F");
        assert_eq!(built.shortcuts.copy, "Ctrl+C");
        assert!(!built.zmodem.enabled);
        assert!(!built.zmodem.auto_detect);
        assert_eq!(built.zmodem.download_dir, existing_download_dir);
        assert_eq!(built.zmodem.timeout_secs, 240);
        // 非 draft 覆盖的终端字段应保留 base
        assert_eq!(built.terminal.scrollback_lines, 5000);
        assert!(!built.terminal.cursor_blink);
        // appearance / transfer / ssh 完整保留
        assert_eq!(built.appearance.theme, "CustomUI");
        assert_eq!(built.appearance.sidebar_width, 300);
        assert_eq!(built.transfer.max_retries, 9);
        assert_eq!(built.ssh.connect_timeout_secs, 42);

        panel.draft.theme = "___no_such_theme___".to_string();
        let err = panel.build_settings(&base).expect_err("未知主题应返回错误");
        assert!(
            err.contains("未找到配色方案"),
            "期望包含「未找到配色方案」，实际：{err}"
        );
    }

    /// P0 Task 3 RED: egui 按键 + 修饰键映射为规范快捷键字符串。
    #[test]
    fn shortcut_from_egui_key_maps_supported_chords_and_rejects_arrows() {
        let ctrl_shift = egui::Modifiers {
            ctrl: true,
            shift: true,
            ..Default::default()
        };
        assert_eq!(
            shortcut_from_egui_key(egui::Key::F, ctrl_shift).unwrap(),
            "Ctrl+Shift+F"
        );

        let ctrl_only = egui::Modifiers {
            ctrl: true,
            ..Default::default()
        };
        assert_eq!(
            shortcut_from_egui_key(egui::Key::Tab, ctrl_only).unwrap(),
            "Ctrl+Tab"
        );

        assert_eq!(
            shortcut_from_egui_key(egui::Key::F12, egui::Modifiers::default()).unwrap(),
            "F12"
        );

        assert!(
            shortcut_from_egui_key(egui::Key::ArrowUp, egui::Modifiers::default()).is_err(),
            "方向键 ArrowUp 应返回 Err"
        );
    }

    /// P0 Task 3 RED: 编译期锁定 `SettingsPanel::show` 精确签名，
    /// 并验证 action enum 可携带 `Apply(Settings)`。
    fn accept_settings_panel_show_fn(
        _f: fn(&mut SettingsPanel, &egui::Context) -> SettingsPanelAction,
    ) {
    }

    #[test]
    fn show_signature_and_apply_action_carry_settings() {
        accept_settings_panel_show_fn(SettingsPanel::show);

        let settings = Settings::default();
        let action = SettingsPanelAction::Apply(settings.clone());
        match action {
            SettingsPanelAction::Apply(applied) => {
                assert_eq!(applied.terminal.font, settings.terminal.font);
                assert_eq!(applied.terminal.font_size, settings.terminal.font_size);
                assert_eq!(
                    applied.terminal.color_scheme,
                    settings.terminal.color_scheme
                );
            }
            SettingsPanelAction::None | SettingsPanelAction::Cancel => {
                panic!("期望 Apply 变体携带 Settings");
            }
        }
    }

    /// 回归：快捷键捕获中按 Escape 只应退出捕获，不得同一帧关闭整个设置面板。
    /// 根因：cancel_capture 先把 capturing_shortcut 清为 None，随后通用 Escape 分支
    /// 看到 None 又返回 Cancel 并 close()。
    #[test]
    fn escape_while_capturing_shortcut_cancels_capture_only_not_panel() {
        let mut panel = SettingsPanel::default();
        panel.open(&Settings::default());
        panel.capturing_shortcut = Some(ShortcutAction::Search);

        let mut action = SettingsPanelAction::Cancel; // 哨兵：show 必须写出
        let ctx = egui::Context::default();
        let _ = ctx.run(
            egui::RawInput {
                events: vec![egui::Event::Key {
                    key: egui::Key::Escape,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: egui::Modifiers::NONE,
                }],
                ..Default::default()
            },
            |ctx| {
                action = panel.show(ctx);
            },
        );

        match action {
            SettingsPanelAction::None => {}
            SettingsPanelAction::Cancel => {
                panic!("捕获中 Escape 不应返回 Cancel（应仅退出捕获）");
            }
            SettingsPanelAction::Apply(_) => {
                panic!("捕获中 Escape 不应返回 Apply");
            }
        }
        assert!(
            panel.visible,
            "捕获中 Escape 后面板应仍可见，实际 visible=false"
        );
        assert!(
            panel.capturing_shortcut.is_none(),
            "捕获中 Escape 后 capturing_shortcut 应变 None"
        );
    }
}
