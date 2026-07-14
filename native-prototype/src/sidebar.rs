use egui;
use crate::connections::ConnectionStore;

/// SSH 连接配置（扁平化，用于 UI 显示）
#[derive(Clone, Debug)]
pub struct SshConnection {
    pub label: String,
    pub host: String,
    pub port: u16,
    pub user: String,
    pub auth: String,
    pub key_path: String,
    pub group: String,
    pub group_color: [u8; 3],
}

/// 侧边栏状态
pub struct Sidebar {
    pub visible: bool,
    pub width: f32,
    pub connections: Vec<SshConnection>,
    pub selected: Option<usize>,
    pub on_connect: Option<SshConnection>,
    collapsed_groups: std::collections::HashSet<String>,
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
        let store = ConnectionStore::load();
        let mut connections = Vec::new();

        for (group_id, group) in &store.groups {
            let color = parse_hex_color(&group.color);
            for (_host_id, host) in &group.hosts {
                connections.push(SshConnection {
                    label: host.label.clone(),
                    host: host.host.clone(),
                    port: host.port,
                    user: host.user.clone(),
                    auth: host.auth.to_string(),
                    key_path: host.key_path.clone(),
                    group: group.label.clone(),
                    group_color: color,
                });
            }
        }

        Self {
            visible: true,
            width: 220.0,
            connections,
            selected: None,
            on_connect: None,
            collapsed_groups: std::collections::HashSet::new(),
        }
    }

    pub fn take_connect(&mut self) -> Option<SshConnection> {
        self.on_connect.take()
    }

    pub fn ui(&mut self, ctx: &egui::Context) -> f32 {
        if !self.visible {
            return 0.0;
        }

        // Don't clear on_connect here — it's taken by the caller via take_connect()
        let panel_width = self.width;

        egui::SidePanel::left("sidebar")
            .exact_width(panel_width)
            .resizable(false)
            .frame(egui::Frame::new()
                .fill(egui::Color32::from_rgb(0x0d, 0x11, 0x17))
                .inner_margin(egui::Margin::same(0)))
            .show(ctx, |ui| {
                ui.style_mut().visuals.override_text_color = Some(egui::Color32::from_rgb(0x8b, 0x94, 0x9e));

                // 标题栏
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    ui.add_space(12.0);
                    ui.label(egui::RichText::new("连接管理")
                        .size(11.0)
                        .color(egui::Color32::from_rgb(0x8b, 0x94, 0x9e)));
                });
                ui.add_space(4.0);
                ui.separator();

                // 连接列表
                egui::ScrollArea::vertical().show(ui, |ui| {
                    let mut current_group = String::new();

                    for (i, conn) in self.connections.iter().enumerate() {
                        // 分组标题
                        if conn.group != current_group {
                            current_group = conn.group.clone();
                            let is_collapsed = self.collapsed_groups.contains(&current_group);

                            ui.add_space(6.0);
                            let group_response = ui.horizontal(|ui| {
                                ui.add_space(8.0);
                                let arrow = if is_collapsed { "▶" } else { "▼" };
                                ui.label(egui::RichText::new(arrow).size(8.0).color(egui::Color32::from_rgb(0x48, 0x4f, 0x58)));

                                let dot_color = egui::Color32::from_rgb(
                                    conn.group_color[0], conn.group_color[1], conn.group_color[2]
                                );
                                let (rect, _) = ui.allocate_exact_size(egui::vec2(8.0, 8.0), egui::Sense::hover());
                                ui.painter().circle_filled(rect.center(), 4.0, dot_color);

                                let resp = ui.label(egui::RichText::new(&current_group)
                                    .size(10.0)
                                    .color(egui::Color32::from_rgb(0x48, 0x4f, 0x58)));
                                resp
                            });

                            if group_response.inner.clicked() {
                                if is_collapsed {
                                    self.collapsed_groups.remove(&current_group);
                                } else {
                                    self.collapsed_groups.insert(current_group.clone());
                                }
                            }
                            ui.add_space(2.0);

                            if is_collapsed {
                                // Skip connections in collapsed group
                                continue;
                            }
                        }

                        // Skip if group is collapsed
                        if self.collapsed_groups.contains(&conn.group) {
                            continue;
                        }

                        // 连接项
                        let is_selected = self.selected == Some(i);

                        let response = ui.horizontal(|ui| {
                            ui.add_space(4.0);
                            let (rect, response) = ui.allocate_exact_size(
                                egui::vec2(panel_width - 12.0, 24.0),
                                egui::Sense::click(),
                            );

                            let bg = if is_selected {
                                egui::Color32::from_rgb(0x1c, 0x20, 0x28)
                            } else if response.hovered() {
                                egui::Color32::from_rgba_premultiplied(0x30, 0x36, 0x3d, 0x60)
                            } else {
                                egui::Color32::TRANSPARENT
                            };
                            ui.painter().rect_filled(rect, 3.0, bg);

                            // Label
                            let text_rect = rect.shrink2(egui::vec2(16.0, 0.0));
                            ui.painter().text(
                                text_rect.left_center(),
                                egui::Align2::LEFT_CENTER,
                                &conn.label,
                                egui::FontId::proportional(11.0),
                                egui::Color32::from_rgb(0xc9, 0xd1, 0xd9),
                            );

                            // host:port (right aligned)
                            let info = format!("{}:{}", conn.host, conn.port);
                            ui.painter().text(
                                text_rect.right_center() - egui::vec2(2.0, 0.0),
                                egui::Align2::RIGHT_CENTER,
                                &info,
                                egui::FontId::proportional(9.0),
                                egui::Color32::from_rgb(0x48, 0x4f, 0x58),
                            );

                            response
                        });

                        if response.inner.double_clicked() {
                            self.on_connect = Some(conn.clone());
                            self.selected = Some(i);
                        } else if response.inner.clicked() {
                            self.selected = Some(i);
                        }
                    }

                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        ui.add_space(12.0);
                        if ui.small_button("+ 新建 SSH 连接").clicked() {
                            // TODO
                        }
                    });
                });

                // 底部：本机
                ui.with_layout(egui::Layout::bottom_up(egui::Align::LEFT), |ui| {
                    ui.add_space(8.0);
                    ui.separator();
                    ui.horizontal(|ui| {
                        ui.add_space(12.0);
                        let dot = egui::Color32::from_rgb(0x58, 0xa6, 0xff);
                        let (rect, _) = ui.allocate_exact_size(egui::vec2(8.0, 8.0), egui::Sense::hover());
                        ui.painter().circle_filled(rect.center(), 4.0, dot);
                        ui.label(egui::RichText::new("本机").size(11.0).color(egui::Color32::from_rgb(0xe6, 0xed, 0xf3)));
                    });
                });
            });

        panel_width
    }
}
