use crate::sidebar::SshConnection;
use std::collections::HashSet;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};

const SHELLS_FILE: &str = "/etc/shells";
const MODAL_MARGIN: f32 = 16.0;
const MODAL_MAX_WIDTH: f32 = 520.0;
const MODAL_HEIGHT_FRACTION: f32 = 0.8;
const MODAL_HEADER_HEIGHT: f32 = 42.0;
const MODAL_PADDING: f32 = 16.0;
const MODAL_BORDER_RADIUS: f32 = 8.0;
const BACKDROP_ALPHA: u8 = 153;
const MODAL_BACKGROUND: egui::Color32 = egui::Color32::from_rgb(0x16, 0x1b, 0x22);
const MODAL_BORDER: egui::Color32 = egui::Color32::from_rgb(0x30, 0x36, 0x3d);
const MODAL_TEXT: egui::Color32 = egui::Color32::from_rgb(0xe6, 0xed, 0xf3);
const MUTED_TEXT: egui::Color32 = egui::Color32::from_rgb(0x8b, 0x94, 0x9e);
const WEAK_TEXT: egui::Color32 = egui::Color32::from_rgb(0x6e, 0x76, 0x81);
const ACCENT_CYAN: egui::Color32 = egui::Color32::from_rgb(0x00, 0xd4, 0xff);
const ACCENT_GREEN: egui::Color32 = egui::Color32::from_rgb(0x00, 0xff, 0x9f);
const ACCENT_YELLOW: egui::Color32 = egui::Color32::from_rgb(0xf1, 0xfa, 0x8c);
const ROW_HOVER: egui::Color32 = egui::Color32::from_rgb(0x21, 0x26, 0x2d);
const SHELL_BACKGROUND: egui::Color32 = egui::Color32::from_rgb(0x0d, 0x11, 0x17);

#[derive(Clone, PartialEq, Eq)]
pub enum NewTabAction {
    None,
    Close,
    OpenShell(PathBuf),
    OpenSsh(String),
    OpenSerial(crate::serial::SerialSpec),
    RefreshSerial(u64),
    NewSsh,
}

impl std::fmt::Debug for NewTabAction {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::None => formatter.write_str("None"),
            Self::Close => formatter.write_str("Close"),
            Self::OpenShell(path) => formatter.debug_tuple("OpenShell").field(path).finish(),
            Self::OpenSsh(_) => formatter
                .debug_tuple("OpenSsh")
                .field(&"<redacted>")
                .finish(),
            Self::OpenSerial(spec) => formatter.debug_tuple("OpenSerial").field(spec).finish(),
            Self::RefreshSerial(generation) => formatter
                .debug_tuple("RefreshSerial")
                .field(generation)
                .finish(),
            Self::NewSsh => formatter.write_str("NewSsh"),
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct SshRow<'a> {
    snapshot_index: usize,
    connection: &'a SshConnection,
}

#[derive(Debug)]
struct SshGroup<'a> {
    label: &'a str,
    color: [u8; 3],
    rows: Vec<SshRow<'a>>,
}

#[derive(Clone, Debug)]
enum SerialScanState {
    Idle,
    Loading,
    Ready(Vec<crate::serial::SerialPortInfo>),
    Error(String),
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct ModalGeometry {
    rect: egui::Rect,
}

#[derive(Debug)]
pub struct NewTabSelector {
    visible: bool,
    shells: Vec<PathBuf>,
    serial_scan_generation: u64,
    serial_scan: SerialScanState,
    serial_baud_rate: u32,
    ssh_expanded: bool,
    serial_expanded: bool,
}

impl Default for NewTabSelector {
    fn default() -> Self {
        Self::new()
    }
}

impl NewTabSelector {
    pub fn new() -> Self {
        Self {
            visible: false,
            shells: Vec::new(),
            serial_scan_generation: 0,
            serial_scan: SerialScanState::Idle,
            serial_baud_rate: crate::serial::DEFAULT_BAUD_RATE,
            ssh_expanded: true,
            serial_expanded: true,
        }
    }

    pub fn open(&mut self) -> u64 {
        self.shells = load_shell_candidates();
        self.visible = true;
        self.begin_serial_scan()
    }

    pub fn close(&mut self) {
        self.visible = false;
    }

    pub fn is_open(&self) -> bool {
        self.visible
    }

    pub fn begin_serial_scan(&mut self) -> u64 {
        self.serial_scan_generation = self.serial_scan_generation.wrapping_add(1).max(1);
        self.serial_scan = SerialScanState::Loading;
        self.serial_scan_generation
    }

    pub fn apply_serial_scan(
        &mut self,
        generation: u64,
        result: Result<Vec<crate::serial::SerialPortInfo>, String>,
    ) -> bool {
        if generation != self.serial_scan_generation {
            return false;
        }
        self.serial_scan = match result {
            Ok(ports) => SerialScanState::Ready(ports),
            Err(error) => SerialScanState::Error(error),
        };
        true
    }

    pub fn show(&mut self, ctx: &egui::Context, connections: &[SshConnection]) -> NewTabAction {
        if !self.visible {
            return NewTabAction::None;
        }

        if ctx.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
            return self.apply_action(NewTabAction::Close);
        }

        let screen = ctx.input(|input| input.screen_rect());
        let Some(geometry) = modal_geometry(screen) else {
            return NewTabAction::None;
        };
        let backdrop_layer = egui::LayerId::new(
            egui::Order::Foreground,
            egui::Id::new("new_tab_selector_backdrop"),
        );
        let modal_layer = egui::LayerId::new(
            egui::Order::Foreground,
            egui::Id::new("new_tab_selector_window"),
        );
        ctx.set_sublayer(backdrop_layer, modal_layer);

        let backdrop_clicked = egui::Area::new(backdrop_layer.id)
            .order(backdrop_layer.order)
            .fixed_pos(screen.min)
            .sense(egui::Sense::hover())
            .show(ctx, |ui| {
                let (rect, response) = ui.allocate_exact_size(screen.size(), egui::Sense::click());
                ui.painter().rect_filled(
                    rect,
                    0.0,
                    egui::Color32::from_rgba_unmultiplied(0, 0, 0, BACKDROP_ALPHA),
                );
                response.clicked()
            })
            .inner;

        let mut action = NewTabAction::None;
        let mut close_clicked = false;
        egui::Area::new(modal_layer.id)
            .order(modal_layer.order)
            .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
            .sense(modal_root_sense())
            .constrain(false)
            .show(ctx, |ui| {
                let modal_width = geometry.rect.width();
                let modal_max_height = geometry.rect.height();
                let border_width = 1.0_f32.min(modal_width / 2.0);
                let modal_content_width = (modal_width - border_width * 2.0).max(0.0);
                let body_padding = MODAL_PADDING.min(modal_content_width / 2.0).floor();
                egui::Frame::new()
                    .fill(MODAL_BACKGROUND)
                    .stroke(egui::Stroke::new(border_width, MODAL_BORDER))
                    .corner_radius(MODAL_BORDER_RADIUS)
                    .shadow(egui::Shadow {
                        offset: [0, 8],
                        blur: 24,
                        spread: 0,
                        color: egui::Color32::from_black_alpha(160),
                    })
                    .show(ui, |ui| {
                        ui.set_width(modal_content_width);
                        ui.spacing_mut().item_spacing.y = 0.0;
                        let modal_content_max_height =
                            (modal_max_height - border_width * 2.0).max(0.0);
                        let header_height = MODAL_HEADER_HEIGHT.min(modal_content_max_height);
                        let (header_rect, _) = ui.allocate_exact_size(
                            egui::vec2(modal_content_width, header_height),
                            egui::Sense::hover(),
                        );
                        ui.painter().line_segment(
                            [header_rect.left_bottom(), header_rect.right_bottom()],
                            egui::Stroke::new(1.0, MODAL_BORDER),
                        );

                        let close_width = 36.0_f32.min(header_rect.width());
                        let close_rect = egui::Rect::from_min_max(
                            egui::pos2(header_rect.right() - close_width, header_rect.top()),
                            header_rect.max,
                        );
                        let close_response = ui.interact(
                            close_rect,
                            egui::Id::new("new_tab_selector_close"),
                            egui::Sense::click(),
                        );
                        let title_rect = egui::Rect::from_min_max(
                            header_rect.min + egui::vec2(MODAL_PADDING, 0.0),
                            egui::pos2(
                                (close_rect.left() - 4.0).max(header_rect.left()),
                                header_rect.bottom(),
                            ),
                        );
                        if title_rect.width() >= 64.0 {
                            let mut title_ui = ui.new_child(
                                egui::UiBuilder::new()
                                    .max_rect(title_rect)
                                    .layout(egui::Layout::left_to_right(egui::Align::Center)),
                            );
                            title_ui.add(egui::Label::new(
                                egui::RichText::new("新建标签页")
                                    .size(14.0)
                                    .strong()
                                    .color(MODAL_TEXT),
                            ));
                        }
                        ui.painter().text(
                            close_rect.center(),
                            egui::Align2::CENTER_CENTER,
                            "×",
                            egui::FontId::proportional(18.0),
                            if close_response.hovered() {
                                egui::Color32::WHITE
                            } else {
                                WEAK_TEXT
                            },
                        );
                        close_clicked = close_response.clicked();

                        let body_max_height = (modal_content_max_height - header_height).max(0.0);
                        if body_max_height <= 0.0 {
                            return;
                        }
                        let body_content_width =
                            (modal_content_width - body_padding * 2.0).max(0.0);
                        if body_content_width <= 0.0 {
                            return;
                        }
                        egui::Frame::new()
                            .inner_margin(egui::Margin::symmetric(body_padding as i8, 0))
                            .show(ui, |ui| {
                                egui::ScrollArea::vertical()
                                    .max_width(body_content_width)
                                    .max_height(body_max_height)
                                    .auto_shrink([false, true])
                                    .show(ui, |ui| {
                                        let content_width = ui.available_width();
                                        ui.set_width(content_width);
                                        ui.add_space(MODAL_PADDING);
                                        draw_shell_section(ui, &self.shells, &mut action);
                                        ui.add_space(20.0);
                                        draw_ssh_section_collapsible(
                                            ui,
                                            connections,
                                            &mut self.ssh_expanded,
                                            &mut action,
                                        );
                                        ui.add_space(20.0);
                                        draw_serial_section_collapsible(
                                            ui,
                                            &self.serial_scan,
                                            &mut self.serial_baud_rate,
                                            &mut self.serial_expanded,
                                            &mut action,
                                        );
                                        ui.add_space(MODAL_PADDING);
                                    });
                            });
                    });
            });

        if backdrop_clicked || close_clicked {
            return self.apply_action(NewTabAction::Close);
        }

        self.apply_action(action)
    }

    fn apply_action(&mut self, action: NewTabAction) -> NewTabAction {
        if action == NewTabAction::Close {
            self.close();
        }
        action
    }
}

fn modal_geometry(viewport: egui::Rect) -> Option<ModalGeometry> {
    if !viewport.is_finite() || !viewport.is_positive() {
        return None;
    }

    let width = if viewport.width() > MODAL_MARGIN * 2.0 {
        MODAL_MAX_WIDTH.min(viewport.width() - MODAL_MARGIN * 2.0)
    } else {
        viewport.width()
    };
    let height = viewport.height() * MODAL_HEIGHT_FRACTION;

    Some(ModalGeometry {
        rect: egui::Rect::from_center_size(viewport.center(), egui::vec2(width, height)),
    })
}

fn modal_root_sense() -> egui::Sense {
    egui::Sense::click()
}

fn draw_section_header(ui: &mut egui::Ui, marker: &str, marker_color: egui::Color32, label: &str) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        ui.label(
            egui::RichText::new(marker)
                .size(12.0)
                .strong()
                .color(marker_color),
        );
        ui.label(
            egui::RichText::new(label)
                .size(12.0)
                .strong()
                .color(MUTED_TEXT),
        );
    });
}

fn draw_collapsible_section_header(
    ui: &mut egui::Ui,
    marker: &str,
    marker_color: egui::Color32,
    label: &str,
    expanded: &mut bool,
) {
    let (rect, response) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), 26.0), egui::Sense::click());
    if response.hovered() {
        ui.painter().rect_filled(rect, 5.0, ROW_HOVER);
    }
    ui.painter().text(
        egui::pos2(rect.left() + 4.0, rect.center().y),
        egui::Align2::LEFT_CENTER,
        marker,
        egui::FontId::proportional(12.0),
        marker_color,
    );
    ui.painter().text(
        egui::pos2(rect.left() + 20.0, rect.center().y),
        egui::Align2::LEFT_CENTER,
        label,
        egui::FontId::proportional(12.0),
        if response.hovered() {
            MODAL_TEXT
        } else {
            MUTED_TEXT
        },
    );
    let center = egui::pos2(rect.right() - 13.0, rect.center().y);
    let stroke = egui::Stroke::new(
        1.4,
        if response.hovered() {
            ACCENT_CYAN
        } else {
            WEAK_TEXT
        },
    );
    let points = if *expanded {
        [
            center + egui::vec2(-4.0, -2.0),
            center,
            center + egui::vec2(4.0, -2.0),
        ]
    } else {
        [
            center + egui::vec2(-2.0, -4.0),
            center,
            center + egui::vec2(-2.0, 4.0),
        ]
    };
    ui.painter().line_segment([points[0], points[1]], stroke);
    ui.painter().line_segment([points[1], points[2]], stroke);
    if response.clicked() {
        *expanded = !*expanded;
    }
}

fn draw_shell_section(ui: &mut egui::Ui, shells: &[PathBuf], action: &mut NewTabAction) {
    draw_section_header(ui, "$", ACCENT_CYAN, "Shell 环境");
    ui.add_space(8.0);

    if shells.is_empty() {
        ui.label(
            egui::RichText::new("未检测到可用 Shell")
                .size(12.0)
                .color(WEAK_TEXT),
        );
        return;
    }

    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = egui::vec2(8.0, 8.0);
        ui.spacing_mut().button_padding = egui::vec2(12.0, 6.0);
        for shell in shells {
            let label = shell_display_name(shell);
            let text_width = ui
                .painter()
                .layout_no_wrap(
                    label.to_string(),
                    egui::FontId::proportional(12.0),
                    MODAL_TEXT,
                )
                .size()
                .x;
            let response = ui
                .scope(|ui| {
                    let visuals = &mut ui.style_mut().visuals.widgets;
                    visuals.inactive.weak_bg_fill = SHELL_BACKGROUND;
                    visuals.inactive.bg_stroke = egui::Stroke::new(1.0, MODAL_BORDER);
                    visuals.inactive.fg_stroke = egui::Stroke::new(1.0, MODAL_TEXT);
                    visuals.hovered.weak_bg_fill = SHELL_BACKGROUND;
                    visuals.hovered.bg_stroke = egui::Stroke::new(1.0, ACCENT_CYAN);
                    visuals.hovered.fg_stroke = egui::Stroke::new(1.0, ACCENT_CYAN);
                    visuals.active.weak_bg_fill = SHELL_BACKGROUND;
                    visuals.active.bg_stroke = egui::Stroke::new(1.0, ACCENT_CYAN);
                    visuals.active.fg_stroke = egui::Stroke::new(1.0, ACCENT_CYAN);
                    ui.add(
                        egui::Button::new(label)
                            .wrap_mode(egui::TextWrapMode::Extend)
                            .min_size(egui::vec2(text_width + 24.0, 28.0))
                            .corner_radius(4.0),
                    )
                })
                .inner;
            if response.clicked() {
                *action = NewTabAction::OpenShell(shell.clone());
            }
        }
    });
}

fn draw_ssh_section(ui: &mut egui::Ui, connections: &[SshConnection], action: &mut NewTabAction) {
    draw_section_header(ui, "@", ACCENT_GREEN, "SSH 连接");
    ui.add_space(8.0);
    draw_ssh_section_body(ui, connections, action);
}

fn draw_ssh_section_collapsible(
    ui: &mut egui::Ui,
    connections: &[SshConnection],
    expanded: &mut bool,
    action: &mut NewTabAction,
) {
    draw_collapsible_section_header(ui, "@", ACCENT_GREEN, "SSH 连接", expanded);
    if !*expanded {
        return;
    }
    ui.add_space(8.0);
    draw_ssh_section_body(ui, connections, action);
}

fn draw_ssh_section_body(
    ui: &mut egui::Ui,
    connections: &[SshConnection],
    action: &mut NewTabAction,
) {
    let groups = group_ssh_connections(connections);
    if groups.is_empty() {
        ui.label(
            egui::RichText::new("暂无保存的连接")
                .size(12.0)
                .color(WEAK_TEXT),
        );
        ui.add_space(4.0);
    } else {
        for (group_index, group) in groups.iter().enumerate() {
            if group_index > 0 {
                ui.add_space(4.0);
            }
            draw_ssh_group_header(ui, group);
            for row in &group.rows {
                if draw_ssh_row(ui, row.connection).clicked() {
                    *action = NewTabAction::OpenSsh(ssh_connection_key(
                        row.snapshot_index,
                        row.connection,
                    ));
                }
            }
        }
        ui.add_space(4.0);
    }

    let response = ui
        .scope(|ui| {
            let visuals = &mut ui.style_mut().visuals.widgets;
            visuals.inactive.fg_stroke = egui::Stroke::new(1.0, ACCENT_CYAN);
            visuals.hovered.fg_stroke = egui::Stroke::new(1.0, egui::Color32::WHITE);
            visuals.active.fg_stroke = egui::Stroke::new(1.0, egui::Color32::WHITE);
            ui.add(egui::Button::new("+ 新建 SSH 连接").small().frame(false))
        })
        .inner;
    if response.clicked() {
        *action = NewTabAction::NewSsh;
    }
}

fn draw_ssh_group_header(ui: &mut egui::Ui, group: &SshGroup<'_>) {
    let (rect, _) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), 18.0), egui::Sense::hover());
    let center = egui::pos2(rect.left() + 5.0, rect.center().y);
    ui.painter().circle_filled(
        center,
        4.0,
        egui::Color32::from_rgb(group.color[0], group.color[1], group.color[2]),
    );
    ui.painter().text(
        egui::pos2(rect.left() + 14.0, rect.center().y),
        egui::Align2::LEFT_CENTER,
        group.label,
        egui::FontId::proportional(12.0),
        WEAK_TEXT,
    );
}

fn draw_ssh_row(ui: &mut egui::Ui, connection: &SshConnection) -> egui::Response {
    let (rect, response) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), 26.0), egui::Sense::click());
    if response.hovered() {
        ui.painter().rect_filled(rect, 4.0, ROW_HOVER);
    }

    let left = rect.left() + 20.0;
    let right = rect.right() - 8.0;
    if right <= left {
        return response;
    }

    let endpoint = ssh_endpoint_label(connection);
    let endpoint_color = if response.hovered() {
        ACCENT_GREEN
    } else {
        WEAK_TEXT
    };
    let endpoint_galley =
        ui.painter()
            .layout_no_wrap(endpoint, egui::FontId::proportional(12.0), endpoint_color);
    let content_width = right - left;
    let endpoint_width = endpoint_galley.size().x.min(content_width * 0.45);
    let endpoint_rect = egui::Rect::from_min_max(
        egui::pos2(right - endpoint_width, rect.top()),
        egui::pos2(right, rect.bottom()),
    );
    let endpoint_left = (right - endpoint_galley.size().x).max(endpoint_rect.left());
    ui.painter().with_clip_rect(endpoint_rect).galley(
        egui::pos2(
            endpoint_left,
            rect.center().y - endpoint_galley.size().y / 2.0,
        ),
        endpoint_galley,
        endpoint_color,
    );

    let label_right = (endpoint_rect.left() - 8.0).max(left);
    let label_painter = ui.painter().with_clip_rect(egui::Rect::from_min_max(
        egui::pos2(left, rect.top()),
        egui::pos2(label_right, rect.bottom()),
    ));
    let label = if connection.label.trim().is_empty() {
        connection.host.as_str()
    } else {
        connection.label.as_str()
    };
    label_painter.text(
        egui::pos2(left, rect.center().y),
        egui::Align2::LEFT_CENTER,
        label,
        egui::FontId::proportional(14.0),
        MODAL_TEXT,
    );

    response
}

fn draw_serial_section(
    ui: &mut egui::Ui,
    scan: &SerialScanState,
    baud_rate: &mut u32,
    action: &mut NewTabAction,
) {
    draw_section_header(ui, "~", ACCENT_YELLOW, "串口设备");
    ui.add_space(8.0);
    draw_serial_section_body(ui, scan, baud_rate, action);
}

fn draw_serial_section_collapsible(
    ui: &mut egui::Ui,
    scan: &SerialScanState,
    baud_rate: &mut u32,
    expanded: &mut bool,
    action: &mut NewTabAction,
) {
    draw_collapsible_section_header(ui, "~", ACCENT_YELLOW, "串口设备", expanded);
    if !*expanded {
        return;
    }
    ui.add_space(8.0);
    draw_serial_section_body(ui, scan, baud_rate, action);
}

fn draw_serial_section_body(
    ui: &mut egui::Ui,
    scan: &SerialScanState,
    baud_rate: &mut u32,
    action: &mut NewTabAction,
) {
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new("波特率").size(11.0).color(MUTED_TEXT));
        egui::ComboBox::from_id_salt("serial_baud_rate")
            .selected_text(baud_rate.to_string())
            .show_ui(ui, |ui| {
                for rate in crate::serial::BAUD_RATES {
                    ui.selectable_value(baud_rate, rate, rate.to_string());
                }
            });
        if ui.small_button("刷新").clicked() {
            *action = NewTabAction::RefreshSerial(0);
        }
    });
    match scan {
        SerialScanState::Idle | SerialScanState::Loading => {
            ui.label(
                egui::RichText::new("正在扫描串口…")
                    .size(12.0)
                    .color(WEAK_TEXT),
            );
        }
        SerialScanState::Error(error) => {
            ui.label(
                egui::RichText::new(error.chars().take(160).collect::<String>())
                    .size(12.0)
                    .color(egui::Color32::from_rgb(0xf8, 0x51, 0x49)),
            );
        }
        SerialScanState::Ready(ports) if ports.is_empty() => {
            ui.label(
                egui::RichText::new("未发现串口设备")
                    .size(12.0)
                    .color(WEAK_TEXT),
            );
        }
        SerialScanState::Ready(ports) => {
            draw_serial_column_header(ui);
            for port in ports {
                let response = draw_serial_row(ui, port);
                if response.clicked() {
                    *action = NewTabAction::OpenSerial(crate::serial::SerialSpec {
                        device: port.path.clone(),
                        display_name: port.name.clone(),
                        serial_number: port.serial_number.clone(),
                        baud_rate: *baud_rate,
                    });
                }
            }
        }
    }
}

#[derive(Clone, Copy)]
struct SerialColumnRects {
    device: egui::Rect,
    model: egui::Rect,
    serial: egui::Rect,
    kind: egui::Rect,
}

fn serial_column_rects(rect: egui::Rect) -> SerialColumnRects {
    let inner = rect.shrink2(egui::vec2(8.0, 0.0));
    let width = inner.width().max(0.0);
    let device_right = inner.left() + width * 0.23;
    let model_right = inner.left() + width * 0.65;
    let serial_right = inner.left() + width * 0.90;
    SerialColumnRects {
        device: egui::Rect::from_min_max(inner.min, egui::pos2(device_right, inner.bottom())),
        model: egui::Rect::from_min_max(
            egui::pos2(device_right, inner.top()),
            egui::pos2(model_right, inner.bottom()),
        ),
        serial: egui::Rect::from_min_max(
            egui::pos2(model_right, inner.top()),
            egui::pos2(serial_right, inner.bottom()),
        ),
        kind: egui::Rect::from_min_max(
            egui::pos2(serial_right, inner.top()),
            egui::pos2(inner.right(), inner.bottom()),
        ),
    }
}

fn paint_serial_cell(ui: &egui::Ui, rect: egui::Rect, text: &str, size: f32, color: egui::Color32) {
    let clip = rect.shrink2(egui::vec2(4.0, 0.0));
    ui.painter().with_clip_rect(clip).text(
        clip.left_center(),
        egui::Align2::LEFT_CENTER,
        text,
        egui::FontId::proportional(size),
        color,
    );
}

fn draw_serial_column_header(ui: &mut egui::Ui) {
    let (rect, _) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), 20.0), egui::Sense::hover());
    let columns = serial_column_rects(rect);
    for (cell, label) in [
        (columns.device, "设备"),
        (columns.model, "型号"),
        (columns.serial, "硬件 SN"),
        (columns.kind, "类型"),
    ] {
        paint_serial_cell(ui, cell, label, 10.0, WEAK_TEXT);
    }
}

fn draw_serial_row(ui: &mut egui::Ui, port: &crate::serial::SerialPortInfo) -> egui::Response {
    let (rect, response) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), 34.0), egui::Sense::click());
    let fill = if response.hovered() {
        ROW_HOVER
    } else {
        SHELL_BACKGROUND
    };
    ui.painter().rect(
        rect,
        0.0,
        fill,
        egui::Stroke::new(1.0, MODAL_BORDER),
        egui::StrokeKind::Inside,
    );
    let columns = serial_column_rects(rect);
    let device_color = if response.hovered() {
        ACCENT_YELLOW
    } else {
        MODAL_TEXT
    };
    paint_serial_cell(ui, columns.device, &port.device_label(), 11.0, device_color);
    paint_serial_cell(ui, columns.model, &port.name, 11.0, MODAL_TEXT);
    paint_serial_cell(
        ui,
        columns.serial,
        port.serial_number.as_deref().unwrap_or("—"),
        10.5,
        MUTED_TEXT,
    );
    paint_serial_cell(ui, columns.kind, &port.port_type, 10.5, MUTED_TEXT);

    response.on_hover_text(format!(
        "{}\n{}\n硬件 SN：{}",
        port.path,
        port.name,
        port.serial_number.as_deref().unwrap_or("无")
    ))
}

fn shell_display_name(path: &Path) -> String {
    #[cfg(windows)]
    {
        let stem = path
            .file_stem()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        if stem.eq_ignore_ascii_case("bash")
            && path.components().any(|component| {
                component
                    .as_os_str()
                    .to_string_lossy()
                    .eq_ignore_ascii_case("git")
            })
        {
            return "Git Bash".to_owned();
        }
        if stem.eq_ignore_ascii_case("pwsh") {
            return "PowerShell 7".to_owned();
        }
        if stem.eq_ignore_ascii_case("powershell") {
            return "Windows PowerShell".to_owned();
        }
        if stem.eq_ignore_ascii_case("cmd") {
            return "命令提示符".to_owned();
        }
    }

    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| "Shell".to_owned())
}

fn group_ssh_connections(connections: &[SshConnection]) -> Vec<SshGroup<'_>> {
    let mut groups: Vec<SshGroup<'_>> = Vec::new();
    for (snapshot_index, connection) in connections.iter().enumerate() {
        let matching_group = groups
            .iter_mut()
            .find(|group| group.label == connection.group && group.color == connection.group_color);
        let row = SshRow {
            snapshot_index,
            connection,
        };
        if let Some(group) = matching_group {
            group.rows.push(row);
        } else {
            groups.push(SshGroup {
                label: connection.group.as_str(),
                color: connection.group_color,
                rows: vec![row],
            });
        }
    }
    groups
}

fn ssh_endpoint_label(connection: &SshConnection) -> String {
    format!("{}:{}", connection.host, connection.port)
}

pub fn parse_shells_with<F>(contents: &str, mut is_executable: F) -> Vec<PathBuf>
where
    F: FnMut(&Path) -> bool,
{
    parse_shells(contents, &mut is_executable)
}

pub fn shell_candidates_with_fallback<F>(
    contents: Option<&str>,
    environment_shell: Option<&OsStr>,
    mut is_executable: F,
) -> Vec<PathBuf>
where
    F: FnMut(&Path) -> bool,
{
    let mut shells = contents
        .map(|contents| parse_shells(contents, &mut is_executable))
        .unwrap_or_default();

    if shells.is_empty() {
        if let Some(shell) = environment_shell {
            let shell = PathBuf::from(shell);
            if shell.is_absolute() && is_executable(&shell) {
                shells.push(shell);
            }
        }
    }

    shells
}

pub fn ssh_connection_key(index: usize, connection: &SshConnection) -> String {
    format!("ssh-v2:{index}:{}", ssh_connection_fingerprint(connection))
}

pub fn resolve_ssh_connection<'a>(
    connections: &'a [SshConnection],
    key: &str,
) -> Option<&'a SshConnection> {
    let mut parts = key.split(':');
    let version = parts.next()?;
    let index = parts.next()?.parse::<usize>().ok()?;
    let fingerprint = parts.next()?;
    if version != "ssh-v2"
        || fingerprint.len() != 32
        || !fingerprint.bytes().all(|byte| byte.is_ascii_hexdigit())
        || parts.next().is_some()
    {
        return None;
    }

    let connection = connections.get(index)?;
    (ssh_connection_key(index, connection) == key).then_some(connection)
}

fn ssh_connection_fingerprint(connection: &SshConnection) -> String {
    const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;
    let mut hashes = [0xcbf2_9ce4_8422_2325, 0x8422_2325_cbf2_9ce4];
    let port = connection.port.to_le_bytes();
    let fields: [&[u8]; 9] = [
        connection.label.as_bytes(),
        connection.host.as_bytes(),
        &port,
        connection.user.as_bytes(),
        connection.auth.as_bytes(),
        connection.key_path.as_bytes(),
        connection.password.as_bytes(),
        connection.group.as_bytes(),
        &connection.group_color,
    ];

    for field in fields {
        let length = (field.len() as u64).to_le_bytes();
        for byte in length.into_iter().chain(field.iter().copied()) {
            hashes[0] ^= u64::from(byte);
            hashes[0] = hashes[0].wrapping_mul(FNV_PRIME);
            hashes[1] ^= u64::from(byte);
            hashes[1] = hashes[1]
                .rotate_left(5)
                .wrapping_mul(FNV_PRIME)
                .wrapping_add(0x9e37_79b9_7f4a_7c15);
        }
    }

    format!("{:016x}{:016x}", hashes[0], hashes[1])
}

fn parse_shells<F>(contents: &str, is_executable: &mut F) -> Vec<PathBuf>
where
    F: FnMut(&Path) -> bool,
{
    let mut seen = HashSet::new();
    let mut shells = Vec::new();

    for line in contents.lines() {
        let candidate = line.trim();
        if candidate.is_empty() || candidate.starts_with('#') {
            continue;
        }

        let path = PathBuf::from(candidate);
        if path.is_absolute() && is_executable(&path) && seen.insert(path.clone()) {
            shells.push(path);
        }
    }

    shells
}

fn load_shell_candidates() -> Vec<PathBuf> {
    #[cfg(windows)]
    {
        return deduplicate_windows_shell_candidates(crate::terminal::local_shell_paths());
    }

    let contents = std::fs::read_to_string(SHELLS_FILE).ok();
    let default_shell = crate::terminal::default_shell_path();
    shell_candidates_with_fallback(
        contents.as_deref(),
        Some(OsStr::new(&default_shell)),
        is_executable_file,
    )
}

#[cfg(windows)]
fn deduplicate_windows_shell_candidates(paths: Vec<PathBuf>) -> Vec<PathBuf> {
    let mut seen = HashSet::new();
    paths
        .into_iter()
        .filter(|path| seen.insert(windows_shell_identity(path)))
        .collect()
}

#[cfg(windows)]
fn windows_shell_identity(path: &Path) -> String {
    let stem = path
        .file_stem()
        .map(|name| name.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    if stem == "bash"
        && path.components().any(|component| {
            component
                .as_os_str()
                .to_string_lossy()
                .eq_ignore_ascii_case("git")
        })
    {
        return "git-bash".to_owned();
    }
    if stem == "pwsh" {
        return "powershell-7".to_owned();
    }
    if stem == "powershell" {
        return "windows-powershell".to_owned();
    }
    if stem == "cmd" {
        return "cmd".to_owned();
    }

    std::fs::canonicalize(path)
        .unwrap_or_else(|_| path.to_path_buf())
        .to_string_lossy()
        .to_ascii_lowercase()
}

fn is_executable_file(path: &Path) -> bool {
    let Ok(metadata) = std::fs::metadata(path) else {
        return false;
    };
    if !metadata.is_file() {
        return false;
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        metadata.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        true
    }
}

#[cfg(test)]
#[path = "new_tab_selector/tests.rs"]
mod tests;
