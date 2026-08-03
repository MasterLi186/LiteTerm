use std::fmt;

use crate::sidebar::SshConnection;
use crate::tunnel::{TunnelId, TunnelInfo, TunnelSpec, TunnelStatus};

const MODAL_BACKGROUND: egui::Color32 = egui::Color32::from_rgb(0x16, 0x1b, 0x22);
const MODAL_BORDER: egui::Color32 = egui::Color32::from_rgb(0x30, 0x36, 0x3d);
const FIELD_BACKGROUND: egui::Color32 = egui::Color32::from_rgb(0x0d, 0x11, 0x17);
const TEXT: egui::Color32 = egui::Color32::from_rgb(0xe6, 0xed, 0xf3);
const MUTED_TEXT: egui::Color32 = egui::Color32::from_rgb(0x8b, 0x94, 0x9e);
const ACCENT: egui::Color32 = egui::Color32::from_rgb(0x00, 0xd4, 0xff);
const SUCCESS: egui::Color32 = egui::Color32::from_rgb(0x3f, 0xb9, 0x50);
const WARNING: egui::Color32 = egui::Color32::from_rgb(0xd2, 0x99, 0x22);
const ERROR: egui::Color32 = egui::Color32::from_rgb(0xf8, 0x51, 0x49);

#[derive(Clone, PartialEq, Eq)]
pub enum TunnelManagerAction {
    None,
    Dismiss,
    Create(TunnelSpec),
    Close(TunnelId),
}

impl fmt::Debug for TunnelManagerAction {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::None => formatter.write_str("None"),
            Self::Dismiss => formatter.write_str("Dismiss"),
            Self::Create(spec) => formatter.debug_tuple("Create").field(spec).finish(),
            Self::Close(id) => formatter.debug_tuple("Close").field(id).finish(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct TunnelDraft {
    connection_index: usize,
    local_port: String,
    remote_host: String,
    remote_port: String,
}

impl Default for TunnelDraft {
    fn default() -> Self {
        Self {
            connection_index: 0,
            local_port: String::new(),
            remote_host: "127.0.0.1".to_string(),
            remote_port: String::new(),
        }
    }
}

pub struct TunnelManager {
    visible: bool,
    draft: TunnelDraft,
    error: Option<String>,
}

impl fmt::Debug for TunnelManager {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TunnelManager")
            .field("visible", &self.visible)
            .field("draft", &self.draft)
            .field("has_error", &self.error.is_some())
            .finish()
    }
}

impl Default for TunnelManager {
    fn default() -> Self {
        Self::new()
    }
}

impl TunnelManager {
    pub fn new() -> Self {
        Self {
            visible: false,
            draft: TunnelDraft::default(),
            error: None,
        }
    }

    pub fn open(&mut self) {
        self.visible = true;
        self.error = None;
    }

    pub fn close(&mut self) {
        self.visible = false;
        self.error = None;
    }

    pub fn is_open(&self) -> bool {
        self.visible
    }

    pub fn set_error(&mut self, error: impl Into<String>) {
        self.error = Some(error.into());
    }

    pub fn show(
        &mut self,
        ctx: &egui::Context,
        connections: &[SshConnection],
        tunnels: &[TunnelInfo],
    ) -> TunnelManagerAction {
        if !self.visible {
            return TunnelManagerAction::None;
        }
        if ctx.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
            self.close();
            return TunnelManagerAction::Dismiss;
        }

        let screen = ctx.input(|input| input.screen_rect());
        let backdrop_clicked = egui::Area::new(egui::Id::new("tunnel_manager_backdrop"))
            .order(egui::Order::Foreground)
            .fixed_pos(screen.min)
            .sense(egui::Sense::click())
            .show(ctx, |ui| {
                let (rect, response) = ui.allocate_exact_size(screen.size(), egui::Sense::click());
                ui.painter().rect_filled(
                    rect,
                    0.0,
                    egui::Color32::from_rgba_unmultiplied(0, 0, 0, 145),
                );
                response.clicked()
            })
            .inner;

        let mut window_open = true;
        let mut action = TunnelManagerAction::None;
        let mut create_clicked = false;
        egui::Window::new("SSH 本地隧道")
            .id(egui::Id::new("tunnel_manager_window"))
            .order(egui::Order::Foreground)
            .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
            .default_width(500.0)
            .max_width((screen.width() - 32.0).max(320.0))
            .max_height((screen.height() - 32.0).max(300.0))
            .resizable(false)
            .collapsible(false)
            .open(&mut window_open)
            .frame(
                egui::Frame::window(&ctx.style())
                    .fill(MODAL_BACKGROUND)
                    .stroke(egui::Stroke::new(1.0, MODAL_BORDER))
                    .corner_radius(8.0),
            )
            .show(ctx, |ui| {
                ui.visuals_mut().override_text_color = Some(TEXT);
                ui.visuals_mut().widgets.inactive.bg_fill = FIELD_BACKGROUND;
                ui.visuals_mut().widgets.hovered.bg_fill =
                    egui::Color32::from_rgb(0x21, 0x26, 0x2d);
                ui.spacing_mut().item_spacing = egui::vec2(8.0, 7.0);

                draw_create_form(ui, &mut self.draft, connections, &mut create_clicked);

                if let Some(error) = self.error.as_deref() {
                    ui.colored_label(ERROR, safe_ui_text(error, 240));
                }

                ui.add_space(4.0);
                ui.separator();
                ui.add_space(4.0);
                ui.label(
                    egui::RichText::new(format!("隧道列表 · {}", tunnels.len()))
                        .size(12.0)
                        .strong()
                        .color(MUTED_TEXT),
                );
                egui::ScrollArea::vertical()
                    .id_salt("tunnel_manager_list")
                    .max_height(190.0)
                    .auto_shrink([false, true])
                    .show(ui, |ui| {
                        if tunnels.is_empty() {
                            ui.label(
                                egui::RichText::new("尚未创建隧道")
                                    .size(12.0)
                                    .color(MUTED_TEXT),
                            );
                        }
                        for tunnel in tunnels {
                            if let Some(id) = draw_tunnel_row(ui, tunnel) {
                                action = TunnelManagerAction::Close(id);
                            }
                        }
                    });
            });

        if !window_open || backdrop_clicked {
            self.close();
            return TunnelManagerAction::Dismiss;
        }
        if action != TunnelManagerAction::None {
            return action;
        }
        if create_clicked {
            match self.create_action(connections) {
                Ok(create) => {
                    self.error = None;
                    return create;
                }
                Err(error) => self.error = Some(error),
            }
        }
        TunnelManagerAction::None
    }

    fn create_action(&self, connections: &[SshConnection]) -> Result<TunnelManagerAction, String> {
        let connection = connections
            .get(self.draft.connection_index)
            .ok_or_else(|| "请选择一个已保存的 SSH 主机".to_string())?;
        let local_port = parse_port(&self.draft.local_port, "本地端口")?;
        let remote_port = parse_port(&self.draft.remote_port, "远端端口")?;
        let spec = TunnelSpec {
            connection: crate::ssh::ConnectionParams::from(connection),
            local_port,
            remote_host: self.draft.remote_host.trim().to_string(),
            remote_port,
        };
        spec.validate()?;
        Ok(TunnelManagerAction::Create(spec))
    }
}

fn parse_port(value: &str, label: &str) -> Result<u16, String> {
    let trimmed = value.trim();
    let port = trimmed
        .parse::<u16>()
        .map_err(|_| format!("{label}必须在 1-65535 之间"))?;
    if port == 0 {
        return Err(format!("{label}必须在 1-65535 之间"));
    }
    Ok(port)
}

fn safe_ui_text(value: &str, max_chars: usize) -> String {
    value
        .chars()
        .filter(|character| !character.is_control())
        .take(max_chars)
        .collect()
}

fn draw_create_form(
    ui: &mut egui::Ui,
    draft: &mut TunnelDraft,
    connections: &[SshConnection],
    create_clicked: &mut bool,
) {
    ui.label(
        egui::RichText::new("创建本地端口转发")
            .size(12.0)
            .strong()
            .color(MUTED_TEXT),
    );
    egui::Grid::new("tunnel_manager_form")
        .num_columns(2)
        .spacing([10.0, 7.0])
        .show(ui, |ui| {
            ui.label("SSH 主机");
            let selected = connections
                .get(draft.connection_index)
                .map(connection_label)
                .unwrap_or_else(|| "无可用主机".to_string());
            egui::ComboBox::from_id_salt("tunnel_manager_host")
                .selected_text(selected)
                .width(300.0)
                .show_ui(ui, |ui| {
                    for (index, connection) in connections.iter().enumerate() {
                        ui.selectable_value(
                            &mut draft.connection_index,
                            index,
                            connection_label(connection),
                        );
                    }
                });
            ui.end_row();

            ui.label("本地端口");
            ui.add(
                egui::TextEdit::singleline(&mut draft.local_port)
                    .desired_width(300.0)
                    .hint_text("例如 8080"),
            );
            ui.end_row();

            ui.label("远端主机");
            ui.add(
                egui::TextEdit::singleline(&mut draft.remote_host)
                    .desired_width(300.0)
                    .hint_text("127.0.0.1"),
            );
            ui.end_row();

            ui.label("远端端口");
            ui.add(
                egui::TextEdit::singleline(&mut draft.remote_port)
                    .desired_width(300.0)
                    .hint_text("例如 80"),
            );
            ui.end_row();
        });
    ui.horizontal(|ui| {
        ui.label(
            egui::RichText::new("仅监听 127.0.0.1")
                .size(11.0)
                .color(MUTED_TEXT),
        );
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let button = egui::Button::new(egui::RichText::new("创建").color(ACCENT))
                .min_size(egui::vec2(72.0, 24.0));
            if ui.add_enabled(!connections.is_empty(), button).clicked() {
                *create_clicked = true;
            }
        });
    });
}

fn connection_label(connection: &SshConnection) -> String {
    format!(
        "{} · {}@{}:{}",
        safe_ui_text(&connection.label, 48),
        safe_ui_text(&connection.user, 48),
        safe_ui_text(&connection.host, 96),
        connection.port
    )
}

fn draw_tunnel_row(ui: &mut egui::Ui, tunnel: &TunnelInfo) -> Option<TunnelId> {
    let mut close = None;
    egui::Frame::new()
        .fill(FIELD_BACKGROUND)
        .stroke(egui::Stroke::new(1.0, MODAL_BORDER))
        .corner_radius(4.0)
        .inner_margin(egui::Margin::symmetric(8, 6))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                let color = status_color(&tunnel.status);
                ui.label(egui::RichText::new("●").size(9.0).color(color));
                ui.vertical(|ui| {
                    ui.label(
                        egui::RichText::new(format!(
                            "127.0.0.1:{} → {}:{}",
                            tunnel.spec.local_port,
                            safe_ui_text(&tunnel.spec.remote_host, 96),
                            tunnel.spec.remote_port
                        ))
                        .size(12.0)
                        .color(TEXT),
                    );
                    let status = match tunnel.status.error() {
                        Some(error) => {
                            format!("{} · {}", tunnel.status.label(), safe_ui_text(error, 140))
                        }
                        None => tunnel.status.label().to_string(),
                    };
                    ui.label(egui::RichText::new(status).size(11.0).color(color));
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let enabled = !matches!(tunnel.status, TunnelStatus::Closing);
                    let label = if tunnel.status.is_terminal() {
                        "移除"
                    } else {
                        "关闭"
                    };
                    if ui
                        .add_enabled(enabled, egui::Button::new(label).small())
                        .clicked()
                    {
                        close = Some(tunnel.id);
                    }
                });
            });
        });
    ui.add_space(4.0);
    close
}

fn status_color(status: &TunnelStatus) -> egui::Color32 {
    match status {
        TunnelStatus::Connecting => WARNING,
        TunnelStatus::Active => SUCCESS,
        TunnelStatus::Closing | TunnelStatus::Stopped => MUTED_TEXT,
        TunnelStatus::Failed(_) => ERROR,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn connection() -> SshConnection {
        SshConnection {
            label: "测试".to_string(),
            host: "ssh.example.test".to_string(),
            port: 22,
            user: "root".to_string(),
            auth: "password".to_string(),
            key_path: "/secret/id".to_string(),
            password: "secret-password".to_string(),
            group: "default".to_string(),
            group_color: [1, 2, 3],
        }
    }

    #[test]
    fn draft_defaults_remote_host_to_loopback() {
        assert_eq!(TunnelDraft::default().remote_host, "127.0.0.1");
    }

    #[test]
    fn form_builds_create_action() {
        let mut manager = TunnelManager::new();
        manager.draft.local_port = "8080".to_string();
        manager.draft.remote_port = "80".to_string();
        let action = manager.create_action(&[connection()]).unwrap();
        let TunnelManagerAction::Create(spec) = action else {
            panic!("expected create action");
        };
        assert_eq!(spec.local_port, 8080);
        assert_eq!(spec.remote_host, "127.0.0.1");
        assert_eq!(spec.remote_port, 80);
        assert_eq!(spec.connection.host, "ssh.example.test");
    }

    #[test]
    fn form_rejects_invalid_ports_and_missing_host() {
        let mut manager = TunnelManager::new();
        manager.draft.local_port = "0".to_string();
        manager.draft.remote_port = "80".to_string();
        assert!(manager
            .create_action(&[connection()])
            .unwrap_err()
            .contains("本地端口"));
        manager.draft.local_port = "8080".to_string();
        assert!(manager.create_action(&[]).unwrap_err().contains("SSH 主机"));
    }

    #[test]
    fn manager_and_action_debug_do_not_expose_credentials_or_errors() {
        let mut manager = TunnelManager::new();
        manager.set_error("/secret/id secret-password");
        let debug = format!("{manager:?}");
        assert!(!debug.contains("/secret/id"));
        assert!(!debug.contains("secret-password"));

        manager.draft.local_port = "8080".to_string();
        manager.draft.remote_port = "80".to_string();
        let action = manager.create_action(&[connection()]).unwrap();
        let debug = format!("{action:?}");
        assert!(!debug.contains("/secret/id"));
        assert!(!debug.contains("secret-password"));
    }
}
