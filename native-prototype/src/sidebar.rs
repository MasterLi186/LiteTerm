use egui;

/// SSH 连接配置
#[derive(Clone, Debug)]
pub struct SshConnection {
    pub label: String,
    pub host: String,
    pub port: u16,
    pub user: String,
    pub group: String,
    pub group_color: [u8; 3],
}

/// 侧边栏状态
pub struct Sidebar {
    pub visible: bool,
    pub width: f32,
    pub connections: Vec<SshConnection>,
    pub selected: Option<usize>,
    pub on_connect: Option<usize>, // 请求连接的索引
}

impl Sidebar {
    pub fn new() -> Self {
        // 预填一些示例连接（后续从 guishell 的 connections.toml 读取）
        let connections = vec![
            SshConnection {
                label: "155bmc".into(), host: "192.168.110.155".into(),
                port: 22, user: "bmc".into(), group: "default".into(),
                group_color: [0x3f, 0xb9, 0x50],
            },
            SshConnection {
                label: "156bmc".into(), host: "192.168.110.156".into(),
                port: 22, user: "bmc".into(), group: "default".into(),
                group_color: [0x3f, 0xb9, 0x50],
            },
            SshConnection {
                label: "192.168.110.81".into(), host: "192.168.110.81".into(),
                port: 22, user: "lfl".into(), group: "default".into(),
                group_color: [0x3f, 0xb9, 0x50],
            },
        ];

        Self {
            visible: true,
            width: 220.0,
            connections,
            selected: None,
            on_connect: None,
        }
    }

    /// 绘制侧边栏，返回消耗的宽度
    pub fn ui(&mut self, ctx: &egui::Context) -> f32 {
        if !self.visible {
            return 0.0;
        }

        self.on_connect = None;

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
                            ui.add_space(8.0);
                            ui.horizontal(|ui| {
                                ui.add_space(12.0);
                                let dot_color = egui::Color32::from_rgb(
                                    conn.group_color[0], conn.group_color[1], conn.group_color[2]
                                );
                                let (rect, _) = ui.allocate_exact_size(
                                    egui::vec2(8.0, 8.0), egui::Sense::hover()
                                );
                                ui.painter().circle_filled(rect.center(), 4.0, dot_color);
                                ui.label(egui::RichText::new(&current_group)
                                    .size(10.0)
                                    .color(egui::Color32::from_rgb(0x48, 0x4f, 0x58)));
                            });
                            ui.add_space(2.0);
                        }

                        // 连接项
                        let is_selected = self.selected == Some(i);
                        let bg = if is_selected {
                            egui::Color32::from_rgb(0x1c, 0x20, 0x28)
                        } else {
                            egui::Color32::TRANSPARENT
                        };

                        let response = ui.horizontal(|ui| {
                            let (rect, response) = ui.allocate_exact_size(
                                egui::vec2(panel_width - 8.0, 28.0),
                                egui::Sense::click(),
                            );
                            if response.hovered() || is_selected {
                                ui.painter().rect_filled(rect, 4.0, bg);
                            }
                            if response.hovered() {
                                ui.painter().rect_filled(
                                    rect, 4.0,
                                    egui::Color32::from_rgba_premultiplied(0x30, 0x36, 0x3d, 0x80)
                                );
                            }

                            // 标签
                            let text_rect = rect.shrink2(egui::vec2(20.0, 0.0));
                            ui.painter().text(
                                text_rect.left_center(),
                                egui::Align2::LEFT_CENTER,
                                &conn.label,
                                egui::FontId::proportional(12.0),
                                egui::Color32::from_rgb(0xc9, 0xd1, 0xd9),
                            );

                            // 右侧 host:port
                            let info = format!("{}:{}", conn.host, conn.port);
                            ui.painter().text(
                                text_rect.right_center() - egui::vec2(4.0, 0.0),
                                egui::Align2::RIGHT_CENTER,
                                &info,
                                egui::FontId::proportional(10.0),
                                egui::Color32::from_rgb(0x48, 0x4f, 0x58),
                            );

                            response
                        });

                        if response.inner.clicked() {
                            self.selected = Some(i);
                        }
                        if response.inner.double_clicked() {
                            self.on_connect = Some(i);
                        }
                    }

                    ui.add_space(8.0);

                    // + 新建连接
                    ui.horizontal(|ui| {
                        ui.add_space(12.0);
                        if ui.small_button("+ 新建 SSH 连接").clicked() {
                            // TODO: 打开连接对话框
                        }
                    });
                });

                // 底部：本机信息
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
