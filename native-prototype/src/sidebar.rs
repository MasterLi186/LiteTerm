use egui;
use crate::connections::{ConnectionStore, HostConfig, AuthMethod};
use std::collections::HashMap;

#[derive(Debug, PartialEq, Eq)]
struct MonitorSourcePresentation {
    dot: MonitorDot,
    title: String,
    detail: String,
    message: String,
    warning: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum MonitorDot {
    Local,
    Remote,
}

impl MonitorDot {
    fn color(self) -> egui::Color32 {
        match self {
            Self::Local => egui::Color32::from_rgb(0x58, 0xa6, 0xff),
            Self::Remote => egui::Color32::from_rgb(0x3f, 0xb9, 0x50),
        }
    }
}

#[derive(Debug)]
struct MonitorViewState {
    selected_iface: Option<String>,
    process_tab: u8,
    net_rx_history: Vec<f64>,
    net_tx_history: Vec<f64>,
    last_chart_iface: Option<String>,
}

impl Default for MonitorViewState {
    fn default() -> Self {
        Self {
            selected_iface: None,
            process_tab: 1,
            net_rx_history: Vec::new(),
            net_tx_history: Vec::new(),
            last_chart_iface: None,
        }
    }
}

fn safe_monitor_text(value: &str, max_chars: usize) -> String {
    value.chars().filter(|character| !character.is_control()).take(max_chars).collect()
}

fn monitor_source_presentation(
    key: &crate::monitor::MonitorKey,
    snapshot: Option<&crate::monitor::MonitorData>,
    error: Option<&str>,
) -> MonitorSourcePresentation {
    let (dot, title, detail) = match key {
        crate::monitor::MonitorKey::Local => {
            (MonitorDot::Local, "本机".into(), String::new())
        }
        crate::monitor::MonitorKey::Remote { .. } => (
            MonitorDot::Remote,
            "已连接".into(),
            safe_monitor_text(&key.status_text(), 96),
        ),
    };
    let safe_error = error.map(|value| {
        let value = safe_monitor_text(value, 160);
        value
            .strip_prefix("监控更新失败：")
            .unwrap_or(&value)
            .to_string()
    });
    let message = match (snapshot, safe_error.as_deref()) {
        (None, Some(error)) if !error.is_empty() => format!("采集失败：{error}"),
        (None, _) => "正在采集".into(),
        (Some(data), _) => {
            let cpu = if data.cpu_percent.is_finite() {
                format!("{:.0}%", data.cpu_percent)
            } else {
                "--".into()
            };
            format!("CPU {cpu} · {}", safe_monitor_text(&data.memory_text, 48))
        }
    };
    let warning = (snapshot.is_some() && error.is_some()).then(|| "监控暂时中断".into());
    MonitorSourcePresentation {
        dot,
        title,
        detail,
        message,
        warning,
    }
}

#[derive(Clone)]
pub struct SshConnection {
    pub label: String,
    pub host: String,
    pub port: u16,
    pub user: String,
    pub auth: String,
    pub key_path: String,
    pub password: String,
    pub group: String,
    pub group_color: [u8; 3],
}

impl std::fmt::Debug for SshConnection {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("SshConnection")
            .field("label", &self.label)
            .field("host", &self.host)
            .field("port", &self.port)
            .field("user", &self.user)
            .field("auth", &self.auth)
            .field("group", &self.group)
            .field("group_color", &self.group_color)
            .finish_non_exhaustive()
    }
}

/// SSH key info (from ~/.ssh/)
struct SshKeyInfo {
    name: String,
    path: String,
    key_type: String,
    is_public: bool,
    fingerprint: String,
}

/// New connection form state
struct NewConnForm {
    label: String,
    host: String,
    port: String,
    user: String,
    auth_idx: usize, // 0=密钥, 1=密码
    key_path: String,
    password: String,
    group: String,
    new_group: String,
    status: String,
}

impl Default for NewConnForm {
    fn default() -> Self {
        Self {
            label: String::new(),
            host: String::new(),
            port: "22".to_string(),
            user: "root".to_string(),
            auth_idx: 0,
            key_path: "~/.ssh/id_rsa".to_string(),
            password: String::new(),
            group: "default".to_string(),
            new_group: String::new(),
            status: String::new(),
        }
    }
}

pub struct Sidebar {
    pub visible: bool,
    pub width: f32,
    pub connections: Vec<SshConnection>,
    pub selected: Option<usize>,
    pub on_connect: Option<SshConnection>,
    pub connections_visible: bool,
    collapsed_groups: std::collections::HashSet<String>,
    // Dialog visibility
    show_new_connection: bool,
    show_key_manager: bool,
    show_export: bool,
    show_import: bool,
    // New connection form
    new_conn: NewConnForm,
    // SSH key manager
    ssh_keys: Vec<SshKeyInfo>,
    ssh_keys_loaded: bool,
    keygen_type: String,
    keygen_comment: String,
    keygen_status: String,
    // Import/export
    io_path: String,
    io_status: String,
    monitor_views: HashMap<crate::monitor::MonitorKey, MonitorViewState>,
}

fn parse_hex_color(s: &str) -> [u8; 3] {
    let s = s.trim_start_matches('#');
    if s.len() >= 6 {
        let r = u8::from_str_radix(&s[0..2], 16).unwrap_or(0x58);
        let g = u8::from_str_radix(&s[2..4], 16).unwrap_or(0xa6);
        let b = u8::from_str_radix(&s[4..6], 16).unwrap_or(0xff);
        [r, g, b]
    } else {
        [0x58, 0xa6, 0xff]
    }
}

impl Sidebar {
    pub fn new() -> Self {
        let connections = Self::load_connections();
        Self {
            visible: true,
            width: 220.0,
            connections,
            selected: None,
            on_connect: None,
            connections_visible: true,
            collapsed_groups: std::collections::HashSet::new(),
            show_new_connection: false,
            show_key_manager: false,
            show_export: false,
            show_import: false,
            new_conn: NewConnForm::default(),
            ssh_keys: Vec::new(),
            ssh_keys_loaded: false,
            keygen_type: "ed25519".to_string(),
            keygen_comment: String::new(),
            keygen_status: String::new(),
            io_path: String::new(),
            io_status: String::new(),
            monitor_views: HashMap::new(),
        }
    }

    fn load_connections() -> Vec<SshConnection> {
        let store = ConnectionStore::load();
        let mut connections = Vec::new();
        for (_group_id, group) in &store.groups {
            let color = parse_hex_color(&group.color);
            for (_host_id, host) in &group.hosts {
                connections.push(SshConnection {
                    label: host.label.clone(),
                    host: host.host.clone(),
                    port: host.port,
                    user: host.user.clone(),
                    auth: host.auth.to_string(),
                    key_path: host.key_path.clone(),
                    password: String::new(),
                    group: group.label.clone(),
                    group_color: color,
                });
            }
        }
        connections
    }

    pub fn reload(&mut self) {
        self.connections = Self::load_connections();
    }

    pub fn take_connect(&mut self) -> Option<SshConnection> {
        self.on_connect.take()
    }

    pub fn on_monitor_update(
        &mut self,
        key: &crate::monitor::MonitorKey,
        monitor: &crate::monitor::MonitorData,
    ) {
        if monitor.net_interfaces.is_empty() {
            return;
        }
        let view = self.monitor_views.entry(key.clone()).or_default();
        let selected = view.selected_iface.clone().unwrap_or_else(|| {
            monitor
                .net_interfaces
                .iter()
                .find(|interface| {
                    !interface.name.starts_with("br-")
                        && !interface.name.starts_with("docker")
                        && !interface.name.starts_with("veth")
                })
                .unwrap_or(&monitor.net_interfaces[0])
                .name
                .clone()
        });
        if view.selected_iface.is_none() {
            view.selected_iface = Some(selected.clone());
        }
        if view.last_chart_iface.as_ref() != Some(&selected) {
            view.net_rx_history.clear();
            view.net_tx_history.clear();
            view.last_chart_iface = Some(selected.clone());
        }
        let (tx_rate, rx_rate) = monitor
            .net_interfaces
            .iter()
            .find(|interface| interface.name == selected)
            .map(|interface| (interface.tx_rate, interface.rx_rate))
            .unwrap_or((0, 0));
        view.net_tx_history.push(tx_rate as f64);
        view.net_rx_history.push(rx_rate as f64);
        if view.net_tx_history.len() > 60 {
            view.net_tx_history.remove(0);
        }
        if view.net_rx_history.len() > 60 {
            view.net_rx_history.remove(0);
        }
    }

    pub fn remove_monitor_view(&mut self, key: &crate::monitor::MonitorKey) {
        if !matches!(key, crate::monitor::MonitorKey::Local) {
            self.monitor_views.remove(key);
        }
    }

    #[cfg(test)]
    fn new_for_test() -> Self {
        Self::new()
    }

    #[cfg(test)]
    fn monitor_view(&self, key: &crate::monitor::MonitorKey) -> &MonitorViewState {
        self.monitor_views
            .get(key)
            .expect("monitor view should exist")
    }

    #[cfg(test)]
    pub(crate) fn monitor_history_for_test(
        &self,
        key: &crate::monitor::MonitorKey,
    ) -> Option<(&[f64], &[f64])> {
        self.monitor_views.get(key).map(|view| {
            (
                view.net_rx_history.as_slice(),
                view.net_tx_history.as_slice(),
            )
        })
    }

    pub fn ui_with_monitor(
        &mut self,
        ctx: &egui::Context,
        active_key: &crate::monitor::MonitorKey,
        snapshot: Option<&crate::monitor::MonitorData>,
        error: Option<&str>,
    ) -> f32 {
        if !self.visible { return 0.0; }
        let panel_width = self.width;
        self.monitor_views.entry(active_key.clone()).or_default();
        let presentation = monitor_source_presentation(active_key, snapshot, error);

        egui::SidePanel::left("sidebar")
            .exact_width(panel_width)
            .resizable(false)
            .frame(egui::Frame::new()
                .fill(egui::Color32::from_rgb(0x0d, 0x11, 0x17))
                .inner_margin(egui::Margin::same(0)))
            .show(ctx, |ui| {
                ui.style_mut().visuals.override_text_color = Some(egui::Color32::from_rgb(0x8b, 0x94, 0x9e));

                // Header + toolbar
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    ui.add_space(6.0);
                    let arrow = if self.connections_visible { "▼" } else { "▶" };
                    let toggle = ui.add(egui::Button::new(
                        egui::RichText::new(format!("{} 连接管理", arrow))
                            .size(11.0).color(egui::Color32::from_rgb(0x8b, 0x94, 0x9e))
                    ).frame(false));
                    if toggle.clicked() { self.connections_visible = !self.connections_visible; }

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.spacing_mut().item_spacing.x = 4.0;
                        let normal = egui::Color32::from_rgb(0x8b, 0x94, 0x9e);
                        let cyan = egui::Color32::from_rgb(0x00, 0xd4, 0xff);

                        let r1 = ui.add(egui::Button::new(egui::RichText::new("+").size(14.0).color(cyan)).frame(false).min_size(egui::vec2(20.0, 18.0)));
                        r1.clone().on_hover_text("新建连接");
                        if r1.clicked() { self.show_new_connection = true; self.new_conn = NewConnForm::default(); }

                        let r2 = ui.add(egui::Button::new(egui::RichText::new("⚿").size(13.0).color(normal)).frame(false).min_size(egui::vec2(20.0, 18.0)));
                        r2.clone().on_hover_text("SSH 密钥管理");
                        if r2.clicked() { self.show_key_manager = true; self.ssh_keys_loaded = false; }

                        let r3 = ui.add(egui::Button::new(egui::RichText::new("⬆").size(12.0).color(normal)).frame(false).min_size(egui::vec2(20.0, 18.0)));
                        r3.clone().on_hover_text("导出配置");
                        if r3.clicked() { self.show_export = true; self.io_path.clear(); self.io_status.clear(); }

                        let r4 = ui.add(egui::Button::new(egui::RichText::new("⬇").size(12.0).color(normal)).frame(false).min_size(egui::vec2(20.0, 18.0)));
                        r4.clone().on_hover_text("导入配置");
                        if r4.clicked() { self.show_import = true; self.io_path.clear(); self.io_status.clear(); }
                    });
                });
                ui.add_space(2.0);
                ui.separator();

                if self.connections_visible {
                    egui::ScrollArea::vertical().show(ui, |ui| {
                        let mut current_group = String::new();
                        for (i, conn) in self.connections.iter().enumerate() {
                            if conn.group != current_group {
                                current_group = conn.group.clone();
                                let is_collapsed = self.collapsed_groups.contains(&current_group);
                                ui.add_space(6.0);
                                let gr = ui.horizontal(|ui| {
                                    ui.add_space(8.0);
                                    let arrow = if is_collapsed { "▶" } else { "▼" };
                                    ui.label(egui::RichText::new(arrow).size(8.0).color(egui::Color32::from_rgb(0x48, 0x4f, 0x58)));
                                    let dot = egui::Color32::from_rgb(conn.group_color[0], conn.group_color[1], conn.group_color[2]);
                                    let (r, _) = ui.allocate_exact_size(egui::vec2(8.0, 8.0), egui::Sense::hover());
                                    ui.painter().circle_filled(r.center(), 4.0, dot);
                                    ui.label(egui::RichText::new(&current_group).size(10.0).color(egui::Color32::from_rgb(0x48, 0x4f, 0x58)))
                                });
                                if gr.inner.clicked() {
                                    if is_collapsed { self.collapsed_groups.remove(&current_group); }
                                    else { self.collapsed_groups.insert(current_group.clone()); }
                                }
                                ui.add_space(2.0);
                                if is_collapsed { continue; }
                            }
                            if self.collapsed_groups.contains(&conn.group) { continue; }

                            let is_selected = self.selected == Some(i);
                            let resp = ui.horizontal(|ui| {
                                ui.add_space(16.0);
                                let (rect, resp) = ui.allocate_exact_size(egui::vec2(panel_width - 20.0, 18.0), egui::Sense::click());
                                if is_selected || resp.hovered() {
                                    let bg = if is_selected { egui::Color32::from_rgb(0x1c, 0x20, 0x28) }
                                    else { egui::Color32::from_rgba_unmultiplied(0x30, 0x36, 0x3d, 0x60) };
                                    ui.painter().rect_filled(rect, 3.0, bg);
                                }
                                let tr = rect.shrink2(egui::vec2(4.0, 0.0));
                                let g = ui.painter().layout(conn.label.clone(), egui::FontId::proportional(11.0), egui::Color32::from_rgb(0xc9, 0xd1, 0xd9), tr.width());
                                ui.painter().galley(tr.left_center() - egui::vec2(0.0, g.size().y / 2.0), g, egui::Color32::from_rgb(0xc9, 0xd1, 0xd9));
                                resp
                            });
                            if resp.inner.clicked() {
                                self.selected = Some(i);
                                self.on_connect = Some(conn.clone());
                            }
                        }
                        ui.add_space(4.0);
                    });
                }

                // Bottom: 当前标签监控来源
                ui.with_layout(egui::Layout::bottom_up(egui::Align::LEFT), |ui| {
                    ui.add_space(8.0);
                    ui.separator();
                    ui.horizontal(|ui| {
                        ui.add_space(12.0);
                        let (r, _) = ui.allocate_exact_size(egui::vec2(8.0, 8.0), egui::Sense::hover());
                        ui.painter().circle_filled(r.center(), 4.0, presentation.dot.color());
                        ui.label(egui::RichText::new(&presentation.title).size(11.0).color(egui::Color32::from_rgb(0xe6, 0xed, 0xf3)));
                        if !presentation.detail.is_empty() {
                            ui.label(egui::RichText::new(&presentation.detail).size(10.0).color(egui::Color32::from_rgb(0x8b, 0x94, 0x9e)));
                        }
                    });
                    if let Some(warning) = &presentation.warning {
                        ui.label(egui::RichText::new(warning).size(10.0).color(egui::Color32::from_rgb(0xd2, 0x99, 0x22)));
                    }
                    ui.label(egui::RichText::new(&presentation.message).size(10.0).color(egui::Color32::from_rgb(0x8b, 0x94, 0x9e)));
                });
            });

        self.render_dialogs(ctx);
        panel_width
    }

    fn render_dialogs(&mut self, ctx: &egui::Context) {
        self.dialog_new_connection(ctx);
        self.dialog_key_manager(ctx);
        self.dialog_export(ctx);
        self.dialog_import(ctx);
    }

    // ── 新建连接 ──
    fn dialog_new_connection(&mut self, ctx: &egui::Context) {
        if !self.show_new_connection { return; }
        let mut open = true;
        let mut save_and_connect = false;
        let mut save_only = false;

        egui::Window::new("新建 SSH 连接")

            .resizable(false)
            .default_width(380.0)
            .show(ctx, |ui| {
                let label_w = 60.0;
                ui.add_space(4.0);

                egui::Grid::new("new_conn_grid").num_columns(2).spacing([8.0, 6.0]).show(ui, |ui| {
                    ui.label("标签:");
                    ui.add(egui::TextEdit::singleline(&mut self.new_conn.label).desired_width(260.0));
                    ui.end_row();

                    ui.label("主机:");
                    ui.add(egui::TextEdit::singleline(&mut self.new_conn.host).desired_width(260.0).hint_text("192.168.1.1"));
                    ui.end_row();

                    ui.label("端口:");
                    ui.add(egui::TextEdit::singleline(&mut self.new_conn.port).desired_width(80.0));
                    ui.end_row();

                    ui.label("用户名:");
                    ui.add(egui::TextEdit::singleline(&mut self.new_conn.user).desired_width(260.0));
                    ui.end_row();

                    ui.label("认证:");
                    ui.horizontal(|ui| {
                        ui.selectable_value(&mut self.new_conn.auth_idx, 0, "密钥");
                        ui.selectable_value(&mut self.new_conn.auth_idx, 1, "密码");
                    });
                    ui.end_row();

                    if self.new_conn.auth_idx == 0 {
                        ui.label("密钥路径:");
                        ui.add(egui::TextEdit::singleline(&mut self.new_conn.key_path).desired_width(260.0));
                        ui.end_row();
                    } else {
                        ui.label("密码:");
                        ui.add(egui::TextEdit::singleline(&mut self.new_conn.password).password(true).desired_width(260.0));
                        ui.end_row();
                    }

                    ui.label("分组:");
                    let store = ConnectionStore::load();
                    let groups: Vec<String> = store.groups.keys().cloned().collect();
                    egui::ComboBox::from_id_salt("group_combo")
                        .selected_text(&self.new_conn.group)
                        .show_ui(ui, |ui| {
                            for g in &groups {
                                ui.selectable_value(&mut self.new_conn.group, g.clone(), g);
                            }
                            ui.selectable_value(&mut self.new_conn.group, "__new__".to_string(), "+ 新建分组");
                        });
                    ui.end_row();

                    if self.new_conn.group == "__new__" {
                        ui.label("新分组名:");
                        ui.add(egui::TextEdit::singleline(&mut self.new_conn.new_group).desired_width(260.0));
                        ui.end_row();
                    }
                });

                if !self.new_conn.status.is_empty() {
                    ui.add_space(4.0);
                    ui.colored_label(egui::Color32::from_rgb(0xf8, 0x51, 0x49), &self.new_conn.status);
                }

                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button("连接并保存").clicked() { save_and_connect = true; }
                    if ui.button("仅保存").clicked() { save_only = true; }
                    if ui.button("取消").clicked() { open = false; }
                });
            });

        if save_and_connect || save_only {
            if let Err(e) = self.do_save_connection() {
                self.new_conn.status = e;
            } else {
                self.reload();
                if save_and_connect {
                    // Trigger connection
                    let port = self.new_conn.port.parse().unwrap_or(22);
                    let auth = if self.new_conn.auth_idx == 0 { "key" } else { "password" };
                    self.on_connect = Some(SshConnection {
                        label: self.new_conn.label.clone(),
                        host: self.new_conn.host.clone(),
                        port,
                        user: self.new_conn.user.clone(),
                        auth: auth.to_string(),
                        key_path: self.new_conn.key_path.clone(),
                        password: self.new_conn.password.clone(),
                        group: self.new_conn.group.clone(),
                        group_color: [0x58, 0xa6, 0xff],
                    });
                }
                open = false;
            }
        }
        if !open { self.show_new_connection = false; }
    }

    fn do_save_connection(&self) -> Result<(), String> {
        let f = &self.new_conn;
        if f.host.is_empty() { return Err("主机不能为空".into()); }
        let port: u16 = f.port.parse().map_err(|_| "端口格式无效")?;
        let label = if f.label.is_empty() { f.host.clone() } else { f.label.clone() };
        let group_id = if f.group == "__new__" {
            if f.new_group.is_empty() { return Err("分组名不能为空".into()); }
            f.new_group.clone()
        } else {
            f.group.clone()
        };

        let auth = if f.auth_idx == 0 { AuthMethod::Key } else { AuthMethod::Password };
        let host_id = format!("{}:{}", f.host, port);

        let mut store = ConnectionStore::load();
        if !store.groups.contains_key(&group_id) {
            store.add_group(&group_id, &group_id, "#58a6ff");
        }
        store.add_host(&group_id, &host_id, HostConfig {
            label,
            host: f.host.clone(),
            port,
            user: f.user.clone(),
            auth,
            key_path: f.key_path.clone(),
            charset: "UTF-8".to_string(),
            proxy_jump: String::new(),
        });
        store.save()
    }

    // ── SSH 密钥管理 ──
    fn dialog_key_manager(&mut self, ctx: &egui::Context) {
        if !self.show_key_manager { return; }

        // Load keys on first show
        if !self.ssh_keys_loaded {
            self.ssh_keys = list_ssh_keys();
            self.ssh_keys_loaded = true;
            self.keygen_status.clear();
        }

        let mut open = true;
        egui::Window::new("SSH 密钥管理")

            .resizable(true)
            .default_width(500.0)
            .default_height(400.0)
            .show(ctx, |ui| {
                // Key list
                ui.heading("已有密钥");
                egui::ScrollArea::vertical().max_height(250.0).show(ui, |ui| {
                    for key in &self.ssh_keys {
                        ui.horizontal(|ui| {
                            let icon = if key.is_public { "🔓" } else { "🔑" };
                            ui.label(egui::RichText::new(icon).size(12.0));
                            ui.label(egui::RichText::new(&key.name).size(11.0).color(egui::Color32::from_rgb(0xc9, 0xd1, 0xd9)));
                            ui.label(egui::RichText::new(&key.key_type).size(10.0).color(egui::Color32::from_rgb(0x8b, 0x94, 0x9e)));
                            if !key.fingerprint.is_empty() {
                                ui.label(egui::RichText::new(&key.fingerprint).size(9.0).color(egui::Color32::from_rgb(0x48, 0x4f, 0x58)));
                            }
                            if key.is_public {
                                if ui.small_button("复制").clicked() {
                                    if let Ok(content) = std::fs::read_to_string(&key.path) {
                                        if let Ok(mut cb) = arboard::Clipboard::new() {
                                            let _ = cb.set_text(&content);
                                        }
                                    }
                                }
                            }
                        });
                    }
                });

                ui.separator();
                ui.heading("生成新密钥");
                ui.horizontal(|ui| {
                    ui.label("类型:");
                    egui::ComboBox::from_id_salt("keygen_type")
                        .selected_text(&self.keygen_type)
                        .show_ui(ui, |ui| {
                            ui.selectable_value(&mut self.keygen_type, "ed25519".to_string(), "ed25519 (推荐)");
                            ui.selectable_value(&mut self.keygen_type, "rsa".to_string(), "rsa");
                            ui.selectable_value(&mut self.keygen_type, "ecdsa".to_string(), "ecdsa");
                        });
                    ui.label("备注:");
                    ui.add(egui::TextEdit::singleline(&mut self.keygen_comment).desired_width(150.0).hint_text("user@host"));
                });
                ui.horizontal(|ui| {
                    if ui.button("生成密钥").clicked() {
                        match generate_ssh_key(&self.keygen_type, &self.keygen_comment) {
                            Ok(pub_key) => {
                                self.keygen_status = format!("✓ 已生成 id_{}\n{}", self.keygen_type, pub_key.trim());
                                self.ssh_keys = list_ssh_keys(); // reload
                            }
                            Err(e) => { self.keygen_status = format!("✗ {}", e); }
                        }
                    }
                });
                if !self.keygen_status.is_empty() {
                    ui.label(egui::RichText::new(&self.keygen_status).size(10.0));
                }
                ui.add_space(8.0);
                if ui.button("关闭").clicked() { open = false; }
            });
        if !open { self.show_key_manager = false; }
    }

    // ── 导出配置 ──
    fn dialog_export(&mut self, ctx: &egui::Context) {
        if !self.show_export { return; }
        let mut open = true;
        egui::Window::new("导出配置")

            .resizable(false)
            .default_width(400.0)
            .show(ctx, |ui| {
                ui.label("将连接配置导出到文件:");
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    ui.label("路径:");
                    ui.add(egui::TextEdit::singleline(&mut self.io_path).desired_width(280.0).hint_text("~/connections_backup.toml"));
                });
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    if ui.button("导出").clicked() {
                        let src = ConnectionStore::config_path();
                        let dst = shellexpand::tilde(&self.io_path).to_string();
                        if dst.is_empty() {
                            self.io_status = "请输入导出路径".to_string();
                        } else {
                            match std::fs::copy(&src, &dst) {
                                Ok(_) => { self.io_status = format!("✓ 已导出到 {}", dst); }
                                Err(e) => { self.io_status = format!("✗ {}", e); }
                            }
                        }
                    }
                    if ui.button("取消").clicked() { open = false; }
                });
                if !self.io_status.is_empty() {
                    ui.add_space(4.0);
                    ui.label(&self.io_status);
                }
            });
        if !open { self.show_export = false; }
    }

    // ── 导入配置 ──
    fn dialog_import(&mut self, ctx: &egui::Context) {
        if !self.show_import { return; }
        let mut open = true;
        egui::Window::new("导入配置")

            .resizable(false)
            .default_width(400.0)
            .show(ctx, |ui| {
                ui.label("从文件导入连接配置（将覆盖现有配置）:");
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    ui.label("路径:");
                    ui.add(egui::TextEdit::singleline(&mut self.io_path).desired_width(280.0).hint_text("~/connections_backup.toml"));
                });
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    if ui.button("导入").clicked() {
                        let path = shellexpand::tilde(&self.io_path).to_string();
                        if path.is_empty() {
                            self.io_status = "请输入文件路径".to_string();
                        } else {
                            match std::fs::read_to_string(&path) {
                                Ok(content) => {
                                    match toml::from_str::<ConnectionStore>(&content) {
                                        Ok(store) => {
                                            match store.save() {
                                                Ok(()) => {
                                                    self.io_status = "✓ 导入成功，已重载连接列表".to_string();
                                                    self.connections = Self::load_connections();
                                                }
                                                Err(e) => { self.io_status = format!("✗ 保存失败: {}", e); }
                                            }
                                        }
                                        Err(e) => { self.io_status = format!("✗ 配置格式无效: {}", e); }
                                    }
                                }
                                Err(e) => { self.io_status = format!("✗ 读取失败: {}", e); }
                            }
                        }
                    }
                    if ui.button("取消").clicked() { open = false; }
                });
                if !self.io_status.is_empty() {
                    ui.add_space(4.0);
                    ui.label(&self.io_status);
                }
            });
        if !open { self.show_import = false; }
    }
}

// ── SSH key helpers (from guishell ssh_keys.rs) ──

fn list_ssh_keys() -> Vec<SshKeyInfo> {
    let ssh_dir = match dirs::home_dir() {
        Some(h) => h.join(".ssh"),
        None => return Vec::new(),
    };
    if !ssh_dir.exists() { return Vec::new(); }

    let mut keys = Vec::new();
    let entries = match std::fs::read_dir(&ssh_dir) {
        Ok(e) => e,
        Err(_) => return Vec::new(),
    };

    for entry in entries.flatten() {
        let path = entry.path();
        let name = path.file_name().unwrap_or_default().to_string_lossy().to_string();
        let is_public = name.ends_with(".pub");
        let is_private = name.starts_with("id_") && !is_public;
        if !is_public && !is_private { continue; }

        let key_type = if name.contains("ed25519") { "ed25519" }
            else if name.contains("ecdsa") { "ecdsa" }
            else if name.contains("rsa") { "rsa" }
            else if name.contains("dsa") { "dsa" }
            else { "unknown" }.to_string();

        let fingerprint = if is_public {
            get_fingerprint(&path)
        } else {
            let pub_path = path.with_extension("pub");
            if pub_path.exists() { get_fingerprint(&pub_path) } else { String::new() }
        };

        keys.push(SshKeyInfo { name, path: path.to_string_lossy().to_string(), key_type, is_public, fingerprint });
    }
    keys.sort_by(|a, b| a.name.cmp(&b.name));
    keys
}

fn get_fingerprint(pub_key_path: &std::path::Path) -> String {
    let output = match std::process::Command::new("ssh-keygen").args(["-lf", &pub_key_path.to_string_lossy()]).output() {
        Ok(o) => o,
        Err(_) => return String::new(),
    };
    if output.status.success() {
        let line = String::from_utf8_lossy(&output.stdout);
        let parts: Vec<&str> = line.trim().splitn(3, ' ').collect();
        if parts.len() >= 2 { return parts[1].to_string(); }
    }
    String::new()
}

fn generate_ssh_key(key_type: &str, comment: &str) -> Result<String, String> {
    let ssh_dir = dirs::home_dir().ok_or("无法获取用户目录")?.join(".ssh");
    std::fs::create_dir_all(&ssh_dir).map_err(|e| format!("创建 .ssh 失败: {}", e))?;

    let key_name = format!("id_{}", key_type);
    let key_path = ssh_dir.join(&key_name);
    if key_path.exists() { return Err(format!("密钥 {} 已存在", key_name)); }

    let comment = if comment.is_empty() { "generated-by-liteterm" } else { comment };
    let output = std::process::Command::new("ssh-keygen")
        .args(["-t", key_type, "-C", comment, "-f", &key_path.to_string_lossy(), "-N", ""])
        .output()
        .map_err(|e| format!("执行 ssh-keygen 失败: {}", e))?;

    if !output.status.success() {
        return Err(format!("ssh-keygen 失败: {}", String::from_utf8_lossy(&output.stderr)));
    }
    std::fs::read_to_string(key_path.with_extension("pub")).map_err(|e| format!("读取公钥失败: {}", e))
}

#[cfg(test)]
mod tests {
    use super::SshConnection;

    fn monitor_with_rate(
        iface: &str,
        rx_rate: u64,
        tx_rate: u64,
    ) -> crate::monitor::MonitorData {
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
            net_interfaces: vec![crate::monitor::NetIfaceInfo {
                name: iface.into(),
                rx_rate,
                tx_rate,
            }],
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
    fn monitor_presentation_uses_placeholder_for_non_finite_cpu() {
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
            net_interfaces: Vec::new(),
        };

        let presentation = super::monitor_source_presentation(
            &crate::monitor::MonitorKey::Local,
            Some(&snapshot),
            None,
        );

        assert_eq!(presentation.message, "CPU -- · 1G / 2G");
        assert!(!presentation.message.contains("NaN"));
    }

    #[test]
    fn network_history_is_isolated_between_remote_monitor_keys() {
        let mut sidebar = super::Sidebar::new_for_test();
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
        let mut sidebar = super::Sidebar::new_for_test();
        let key = crate::monitor::MonitorKey::remote("alice", "alpha.example", 22);

        sidebar.on_monitor_update(&key, &monitor_with_rate("eth0", 100, 200));
        sidebar.on_monitor_update(&key, &monitor_with_rate("eth0", 150, 250));

        assert_eq!(sidebar.monitor_view(&key).net_rx_history, [100.0, 150.0]);
        assert_eq!(sidebar.monitor_view(&key).net_tx_history, [200.0, 250.0]);
    }

    #[test]
    fn process_tab_and_interface_selection_are_isolated_by_monitor_key() {
        let mut sidebar = super::Sidebar::new_for_test();
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
        let error_only = super::monitor_source_presentation(
            &key,
            None,
            Some("监控更新失败：连接暂时不可用\n"),
        );
        assert_eq!(error_only.message, "采集失败：连接暂时不可用");
        assert_eq!(error_only.warning, None);

        let snapshot = monitor_with_rate("eth0", 100, 200);
        let retained =
            super::monitor_source_presentation(&key, Some(&snapshot), Some("连接暂时不可用"));
        assert_eq!(retained.message, "CPU 10% · 1G / 2G");
        assert_eq!(retained.warning.as_deref(), Some("监控暂时中断"));
    }

    #[test]
    fn remove_monitor_view_only_removes_the_target_key() {
        let mut sidebar = super::Sidebar::new_for_test();
        let a = crate::monitor::MonitorKey::remote("alice", "alpha.example", 22);
        let b = crate::monitor::MonitorKey::remote("alice", "beta.example", 22);
        sidebar.on_monitor_update(&a, &monitor_with_rate("eth0", 100, 200));
        sidebar.on_monitor_update(&b, &monitor_with_rate("ens3", 300, 400));

        sidebar.remove_monitor_view(&a);

        assert!(!sidebar.monitor_views.contains_key(&a));
        assert_eq!(sidebar.monitor_view(&b).net_rx_history, [300.0]);
    }
}
