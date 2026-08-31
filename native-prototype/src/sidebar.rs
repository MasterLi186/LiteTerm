use crate::connections::{AuthMethod, ConnectionStore, HostConfig};
use egui;
use std::collections::HashMap;

const SIDEBAR_META_SIZE: f32 = 11.0;
const SIDEBAR_BODY_SIZE: f32 = 12.0;
const SIDEBAR_SECTION_SIZE: f32 = 12.0;
const SIDEBAR_VALUE_SIZE: f32 = 13.0;
const SIDEBAR_NARROW_THRESHOLD: f32 = 220.0;
const CONNECTION_ROW_MIN_HEIGHT: f32 = 22.0;
const PROCESS_ROW_MIN_HEIGHT: f32 = 24.0;
const DISK_ROW_MIN_HEIGHT: f32 = 22.0;
const SIDEBAR_CARD_STROKE_WIDTH: f32 = 1.0;
const SIDEBAR_MIN_UPTIME_COLUMN_WIDTH: f32 = 40.0;
const PROCESS_COLUMN_GAP: f32 = 8.0;
const DISK_COLUMN_GAP: f32 = 4.0;

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
    interface_selection_manual: bool,
    process_tab: u8,
    net_rx_history: Vec<f64>,
    net_tx_history: Vec<f64>,
    last_chart_iface: Option<String>,
}

impl Default for MonitorViewState {
    fn default() -> Self {
        Self {
            selected_iface: None,
            interface_selection_manual: false,
            process_tab: 1,
            net_rx_history: Vec::new(),
            net_tx_history: Vec::new(),
            last_chart_iface: None,
        }
    }
}

fn safe_monitor_text(value: &str) -> String {
    value
        .chars()
        .filter(|character| !character.is_control())
        .collect()
}

fn automatic_network_interface(mon: &crate::monitor::MonitorData) -> Option<String> {
    mon.preferred_net_interface
        .as_ref()
        .filter(|preferred| {
            mon.net_interfaces
                .iter()
                .any(|interface| interface.name == **preferred)
        })
        .cloned()
        .or_else(|| {
            mon.net_interfaces
                .iter()
                .find(|interface| {
                    !interface.name.starts_with("br-")
                        && !interface.name.starts_with("docker")
                        && !interface.name.starts_with("veth")
                        && !interface.name.starts_with("virbr")
                        && !interface.name.starts_with("tun")
                        && !interface.name.starts_with("tap")
                })
                .or_else(|| mon.net_interfaces.first())
                .map(|interface| interface.name.clone())
        })
}

fn monitor_source_presentation(
    key: &crate::monitor::MonitorKey,
    snapshot: Option<&crate::monitor::MonitorData>,
    error: Option<&str>,
) -> MonitorSourcePresentation {
    let (dot, title, detail) = match key {
        crate::monitor::MonitorKey::Local => (MonitorDot::Local, "本机".into(), String::new()),
        crate::monitor::MonitorKey::Remote { .. } => (
            MonitorDot::Remote,
            "已连接".into(),
            safe_monitor_text(&key.status_text()),
        ),
    };
    let safe_error = error.map(|value| {
        let value = safe_monitor_text(value);
        value
            .strip_prefix("监控更新失败：")
            .unwrap_or(&value)
            .to_string()
    });
    let message = match (snapshot, safe_error.as_deref()) {
        (None, Some(error)) if !error.is_empty() => format!("采集失败：{error}"),
        (None, _) => "正在采集".into(),
        // CPU 和内存已经在下方资源卡片中完整展示，这里不再重复占用一行。
        (Some(_), _) => String::new(),
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

#[derive(Clone, Copy, Debug)]
struct SidebarMonitorCardGeometry {
    card_content_width: f32,
    uptime_content_width: f32,
    uptime_inner_margin: f32,
    stroke_width: f32,
    can_render: bool,
}

#[derive(Clone, Copy, Debug)]
struct ProcessRowColumns {
    memory_width: f32,
    cpu_width: f32,
    command_width: f32,
    gap_width: f32,
}

#[derive(Clone, Copy, Debug)]
struct ProcessRowRects {
    memory: egui::Rect,
    cpu: egui::Rect,
    command: egui::Rect,
}

#[derive(Clone, Copy, Debug)]
struct DiskRowColumns {
    mount_width: f32,
    percent_width: f32,
    capacity_width: f32,
    gap_width: f32,
}

#[derive(Clone, Copy, Debug)]
struct DiskRowRects {
    mount: egui::Rect,
    percent: egui::Rect,
    capacity: egui::Rect,
}

#[derive(Clone, Copy, Debug)]
struct DiskRowColors {
    mount: egui::Color32,
    percent: egui::Color32,
    capacity: egui::Color32,
}

/// Return the memory value used by the compact sidebar process list.
///
/// Windows Task Manager's process list reports the private working set, while the full process
/// manager keeps the private-commit value as its application-memory diagnostic. Use the same
/// private working-set value in the sidebar so its number can be compared directly with Task
/// Manager.
#[cfg(target_os = "windows")]
fn sidebar_process_memory(process: &crate::monitor::ProcessInfo) -> (&str, u64) {
    (&process.resident_mem_mb, process.resident_mem_bytes)
}

#[cfg(not(target_os = "windows"))]
fn sidebar_process_memory(process: &crate::monitor::ProcessInfo) -> (&str, u64) {
    (&process.mem_mb, process.mem_bytes)
}

fn sidebar_card_inner_width(card_width: f32, margin: f32) -> f32 {
    (card_width - margin * 2.0).max(0.0)
}

fn sidebar_monitor_outer_width_and_stroke(panel_width: f32, available_width: f32) -> (f32, f32) {
    let panel_width = if panel_width.is_finite() {
        panel_width.max(0.0)
    } else {
        0.0
    };
    let available_width = if available_width.is_finite() {
        available_width.max(0.0)
    } else {
        0.0
    };
    let outer_width = panel_width.min(available_width);
    let stroke_width = SIDEBAR_CARD_STROKE_WIDTH.min(outer_width / 2.0);
    (outer_width, stroke_width)
}

fn sidebar_monitor_card_width(panel_width: f32, available_width: f32) -> f32 {
    let (outer_width, stroke_width) =
        sidebar_monitor_outer_width_and_stroke(panel_width, available_width);
    (outer_width - stroke_width * 2.0).max(0.0)
}

fn sidebar_uptime_column_width(card_width: f32) -> f32 {
    sidebar_card_inner_width(card_width, 8.0) / 2.0
}

fn sidebar_cpu_text_width(card_width: f32) -> f32 {
    sidebar_card_inner_width(card_width, 8.0)
}

fn sidebar_monitor_card_geometry(
    panel_width: f32,
    available_width: f32,
) -> SidebarMonitorCardGeometry {
    let (_, stroke_width) = sidebar_monitor_outer_width_and_stroke(panel_width, available_width);
    let card_content_width = sidebar_monitor_card_width(panel_width, available_width);
    let uptime_content_width = sidebar_cpu_text_width(card_content_width);
    let uptime_inner_margin = (card_content_width - uptime_content_width) / 2.0;
    let can_render =
        sidebar_uptime_column_width(card_content_width) >= SIDEBAR_MIN_UPTIME_COLUMN_WIDTH;

    SidebarMonitorCardGeometry {
        card_content_width,
        uptime_content_width,
        uptime_inner_margin,
        stroke_width,
        can_render,
    }
}

fn show_sidebar_monitor_card<R>(
    ui: &mut egui::Ui,
    geometry: SidebarMonitorCardGeometry,
    fill: egui::Color32,
    border: egui::Color32,
    inner_margin: f32,
    add_contents: impl FnOnce(&mut egui::Ui) -> R,
) -> Option<egui::InnerResponse<R>> {
    if !geometry.can_render {
        return None;
    }

    let frame = egui::Frame::new()
        .fill(fill)
        .corner_radius(6.0)
        .stroke(egui::Stroke::new(geometry.stroke_width, border))
        .inner_margin(inner_margin);
    let total_margin = frame.total_margin();
    let outer_width = geometry.card_content_width + geometry.stroke_width * 2.0;
    let content_width = (outer_width - total_margin.left - total_margin.right).max(0.0);
    let outer_bounds = ui.available_rect_before_wrap();
    let content_min = outer_bounds.min + total_margin.left_top();
    let content_max = egui::pos2(
        content_min.x + content_width,
        (outer_bounds.bottom() - total_margin.bottom).max(content_min.y),
    );
    let mut content_ui = ui.new_child(
        egui::UiBuilder::new().max_rect(egui::Rect::from_min_max(content_min, content_max)),
    );
    let horizontal_clip = egui::Rect::from_min_max(
        egui::pos2(content_min.x, ui.clip_rect().top()),
        egui::pos2(content_max.x, ui.clip_rect().bottom()),
    );
    content_ui.set_clip_rect(ui.clip_rect().intersect(horizontal_clip));
    content_ui.set_width(content_width);

    let background = ui.painter().add(egui::Shape::Noop);
    let inner = add_contents(&mut content_ui);
    let content_height = (content_ui.min_rect().bottom() - content_min.y).max(0.0);
    let content_rect =
        egui::Rect::from_min_size(content_min, egui::vec2(content_width, content_height));
    let outer_rect = frame.outer_rect(content_rect);
    ui.painter().set(background, frame.paint(content_rect));
    // The child UI background is registered before its widgets, so it stays behind clickable
    // process/network rows. Allocating another hover response here would register it last and
    // intercept pointer hits across the whole card.
    let response = content_ui.response().with_new_rect(outer_rect);
    drop(content_ui);
    ui.advance_cursor_after_rect(outer_rect);

    Some(egui::InnerResponse::new(inner, response))
}

fn sidebar_text_width(ui: &egui::Ui, text: &str, size: f32) -> f32 {
    ui.painter()
        .layout_no_wrap(
            text.to_owned(),
            egui::FontId::proportional(size),
            egui::Color32::WHITE,
        )
        .size()
        .x
}

/// Split an already horizontally-padded process-row content width into three equal columns.
fn process_table_columns(content_width: f32) -> ProcessRowColumns {
    let content_width = if content_width.is_finite() {
        content_width.max(0.0)
    } else {
        0.0
    };
    let gap_width = PROCESS_COLUMN_GAP.min(content_width / 4.0);
    let usable_width = (content_width - gap_width * 2.0).max(0.0);
    let column_width = usable_width / 3.0;
    let command_width = (usable_width - column_width * 2.0).max(0.0);

    ProcessRowColumns {
        memory_width: column_width,
        cpu_width: column_width,
        command_width,
        gap_width,
    }
}

fn process_row_rects(row_rect: egui::Rect, columns: ProcessRowColumns) -> ProcessRowRects {
    let horizontal_inset = 8.0_f32.min(row_rect.width().max(0.0) / 2.0);
    let content_rect = row_rect.shrink2(egui::vec2(horizontal_inset, 0.0));
    let content_right = content_rect.right();
    let memory_left = content_rect.left().min(content_right);
    let memory_right = (memory_left + columns.memory_width)
        .min(content_right)
        .max(memory_left);
    let cpu_left = (memory_right + columns.gap_width).min(content_right);
    let cpu_right = (cpu_left + columns.cpu_width)
        .min(content_right)
        .max(cpu_left);
    let command_left = (cpu_right + columns.gap_width).min(content_right);
    let command_right = (command_left + columns.command_width)
        .min(content_right)
        .max(command_left);

    ProcessRowRects {
        memory: egui::Rect::from_min_max(
            egui::pos2(memory_left, content_rect.top()),
            egui::pos2(memory_right, content_rect.bottom()),
        ),
        cpu: egui::Rect::from_min_max(
            egui::pos2(cpu_left, content_rect.top()),
            egui::pos2(cpu_right, content_rect.bottom()),
        ),
        command: egui::Rect::from_min_max(
            egui::pos2(command_left, content_rect.top()),
            egui::pos2(command_right, content_rect.bottom()),
        ),
    }
}

fn process_row_height(
    ui: &egui::Ui,
    columns: ProcessRowColumns,
    memory_text: &str,
    command_text: &str,
) -> f32 {
    let font = egui::FontId::proportional(SIDEBAR_BODY_SIZE);
    let memory_height = ui
        .painter()
        .layout(
            memory_text.to_owned(),
            font.clone(),
            egui::Color32::WHITE,
            columns.memory_width.max(1.0),
        )
        .size()
        .y;
    let command_height = ui
        .painter()
        .layout(
            command_text.to_owned(),
            font,
            egui::Color32::WHITE,
            columns.command_width.max(1.0),
        )
        .size()
        .y;
    (memory_height.max(command_height).max(14.0) + 6.0).max(PROCESS_ROW_MIN_HEIGHT)
}

/// Size the disk table from its actual values. The mount point receives the remaining width;
/// when the sidebar is too small, every column wraps instead of losing text.
fn disk_table_columns(
    ui: &egui::Ui,
    content_width: f32,
    disks: &[crate::monitor::DiskItem],
) -> DiskRowColumns {
    let content_width = if content_width.is_finite() {
        content_width.max(0.0)
    } else {
        0.0
    };
    let text_width = |text: &str| sidebar_text_width(ui, text, SIDEBAR_BODY_SIZE) + 2.0;
    let mount_header_width = text_width("挂载点");
    let percent_natural = disks
        .iter()
        .map(|disk| text_width(&format!("{}%", disk.percent)))
        .fold(text_width("使用率"), f32::max);
    let capacity_natural = disks
        .iter()
        .map(|disk| text_width(&format!("{}/{}", disk.avail, disk.size)))
        .fold(text_width("可用/总量"), f32::max);
    let gap_width = DISK_COLUMN_GAP.min(content_width / 4.0);
    let usable_width = (content_width - gap_width * 2.0).max(0.0);

    let (mount_width, percent_width, capacity_width) =
        if mount_header_width + percent_natural + capacity_natural <= usable_width {
            (
                usable_width - percent_natural - capacity_natural,
                percent_natural,
                capacity_natural,
            )
        } else {
            // A remote mount path can be arbitrarily long. It must wrap inside the first
            // column instead of stealing nearly all width from usage and capacity.
            let natural_total = mount_header_width + percent_natural + capacity_natural;
            let scale = if natural_total > 0.0 {
                (usable_width / natural_total).clamp(0.0, 1.0)
            } else {
                0.0
            };
            (
                mount_header_width * scale,
                percent_natural * scale,
                capacity_natural * scale,
            )
        };

    DiskRowColumns {
        mount_width,
        percent_width,
        capacity_width,
        gap_width,
    }
}

fn disk_row_rects(row_rect: egui::Rect, columns: DiskRowColumns) -> DiskRowRects {
    let horizontal_inset = 8.0_f32.min(row_rect.width().max(0.0) / 2.0);
    let content_rect = row_rect.shrink2(egui::vec2(horizontal_inset, 0.0));
    let content_right = content_rect.right();

    let mount_left = content_rect.left().min(content_right);
    let mount_right = (mount_left + columns.mount_width)
        .min(content_right)
        .max(mount_left);
    let percent_left = (mount_right + columns.gap_width).min(content_right);
    let percent_right = (percent_left + columns.percent_width)
        .min(content_right)
        .max(percent_left);
    let capacity_left = (percent_right + columns.gap_width).min(content_right);
    let capacity_right = (capacity_left + columns.capacity_width)
        .min(content_right)
        .max(capacity_left);

    DiskRowRects {
        mount: egui::Rect::from_min_max(
            egui::pos2(mount_left, content_rect.top()),
            egui::pos2(mount_right, content_rect.bottom()),
        ),
        percent: egui::Rect::from_min_max(
            egui::pos2(percent_left, content_rect.top()),
            egui::pos2(percent_right, content_rect.bottom()),
        ),
        capacity: egui::Rect::from_min_max(
            egui::pos2(capacity_left, content_rect.top()),
            egui::pos2(capacity_right, content_rect.bottom()),
        ),
    }
}

fn disk_mount_label(text: &str, color: egui::Color32) -> egui::Label {
    egui::Label::new(
        egui::RichText::new(text)
            .size(SIDEBAR_BODY_SIZE)
            .color(color),
    )
    .wrap()
    .halign(egui::Align::Min)
}

fn disk_row_height(
    ui: &egui::Ui,
    row_width: f32,
    columns: DiskRowColumns,
    mount_text: &str,
    percent_text: &str,
    capacity_text: &str,
) -> f32 {
    let probe = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(row_width, 0.0));
    let rects = disk_row_rects(probe, columns);
    let font = egui::FontId::proportional(SIDEBAR_BODY_SIZE);
    let text_height = [
        (mount_text, rects.mount.width()),
        (percent_text, rects.percent.width()),
        (capacity_text, rects.capacity.width()),
    ]
    .into_iter()
    .map(|(text, width)| {
        ui.painter()
            .layout(
                text.to_owned(),
                font.clone(),
                egui::Color32::WHITE,
                width.max(1.0),
            )
            .size()
            .y
    })
    .fold(0.0_f32, f32::max);
    (text_height + 6.0).max(DISK_ROW_MIN_HEIGHT)
}

fn render_disk_row_content(
    ui: &mut egui::Ui,
    row_rect: egui::Rect,
    columns: DiskRowColumns,
    mount_text: &str,
    percent_text: &str,
    capacity_text: &str,
    colors: DiskRowColors,
) {
    let rects = disk_row_rects(row_rect, columns);
    let mut row_ui = ui.new_child(
        egui::UiBuilder::new()
            .id_salt((
                "disk_row_content",
                row_rect.min.x.to_bits(),
                row_rect.min.y.to_bits(),
            ))
            .max_rect(row_rect)
            .layout(egui::Layout::top_down(egui::Align::Min)),
    );
    row_ui.set_clip_rect(row_ui.clip_rect().intersect(row_rect));

    if rects.mount.width() > 0.0 {
        let mut mount_ui = row_ui.new_child(
            egui::UiBuilder::new()
                .id_salt("disk_mount")
                .max_rect(rects.mount)
                .layout(egui::Layout::top_down_justified(egui::Align::Min)),
        );
        mount_ui.set_clip_rect(row_ui.clip_rect().intersect(rects.mount));
        mount_ui.add(disk_mount_label(mount_text, colors.mount));
    }
    if rects.percent.width() > 0.0 {
        row_ui.put(
            rects.percent,
            egui::Label::new(
                egui::RichText::new(percent_text)
                    .size(SIDEBAR_BODY_SIZE)
                    .color(colors.percent),
            )
            .wrap()
            .halign(egui::Align::Max),
        );
    }
    if rects.capacity.width() > 0.0 {
        row_ui.put(
            rects.capacity,
            egui::Label::new(
                egui::RichText::new(capacity_text)
                    .size(SIDEBAR_BODY_SIZE)
                    .color(colors.capacity),
            )
            .wrap()
            .halign(egui::Align::Max),
        );
    }
}

fn render_process_row_content(
    ui: &mut egui::Ui,
    row_rect: egui::Rect,
    columns: ProcessRowColumns,
    memory_text: &str,
    cpu: f32,
    command_text: &str,
    memory_color: egui::Color32,
) {
    let rects = process_row_rects(row_rect, columns);
    let cpu_height = 14.0_f32.min(rects.cpu.height().max(0.0));
    let cpu_top = rects.cpu.center().y - cpu_height / 2.0;
    let cpu_rect = egui::Rect::from_min_max(
        egui::pos2(rects.cpu.left(), cpu_top),
        egui::pos2(rects.cpu.right(), cpu_top + cpu_height),
    );

    // Paint row contents without child widgets. Labels registered above the row response would
    // otherwise win hit-testing and make only the painted CPU badge clickable.
    if columns.memory_width > 0.0 {
        let galley = ui.painter().layout(
            memory_text.to_owned(),
            egui::FontId::proportional(SIDEBAR_BODY_SIZE),
            memory_color,
            rects.memory.width(),
        );
        let position = egui::pos2(
            rects.memory.left(),
            rects.memory.center().y - galley.size().y / 2.0,
        );
        ui.painter()
            .with_clip_rect(rects.memory)
            .galley(position, galley, memory_color);
    }

    if columns.cpu_width > 0.0 && cpu_rect.height() > 0.0 {
        let cpu_bg = egui::Color32::from_rgb(0x21, 0x26, 0x2d);
        let cpu_fill = if cpu > 80.0 {
            egui::Color32::from_rgba_unmultiplied(0xf8, 0x51, 0x49, 0x66)
        } else if cpu > 50.0 {
            egui::Color32::from_rgba_unmultiplied(0xd2, 0x99, 0x22, 0x66)
        } else {
            egui::Color32::from_rgba_unmultiplied(0x3f, 0xb9, 0x50, 0x59)
        };
        let cpu_painter = ui.painter().with_clip_rect(cpu_rect);
        cpu_painter.rect_filled(cpu_rect, 3.0, cpu_bg);
        let fill_width = (cpu_rect.width() * (cpu / 100.0).min(1.0)).max(0.0);
        cpu_painter.rect_filled(
            egui::Rect::from_min_size(cpu_rect.min, egui::vec2(fill_width, cpu_rect.height())),
            3.0,
            cpu_fill,
        );
        cpu_painter.text(
            cpu_rect.center(),
            egui::Align2::CENTER_CENTER,
            format!("{:.1}%", cpu),
            egui::FontId::proportional(SIDEBAR_BODY_SIZE),
            egui::Color32::from_rgb(0xe6, 0xed, 0xf3),
        );
    }

    if columns.command_width > 0.0 {
        let command_color = egui::Color32::from_rgb(0xc9, 0xd1, 0xd9);
        let galley = ui.painter().layout(
            command_text.to_owned(),
            egui::FontId::proportional(SIDEBAR_BODY_SIZE),
            command_color,
            rects.command.width(),
        );
        let position = egui::pos2(
            rects.command.left(),
            rects.command.center().y - galley.size().y / 2.0,
        );
        ui.painter()
            .with_clip_rect(rects.command)
            .galley(position, galley, command_color);
    }
}

fn render_process_table_header(
    ui: &mut egui::Ui,
    row_rect: egui::Rect,
    columns: ProcessRowColumns,
    process_tab: &mut u8,
) {
    let rects = process_row_rects(row_rect, columns);
    for (rect, label, index, alignment) in [
        (rects.memory, "内存", 0_u8, egui::Align2::LEFT_CENTER),
        (rects.cpu, "CPU", 1_u8, egui::Align2::CENTER_CENTER),
        (rects.command, "命令", 2_u8, egui::Align2::LEFT_CENTER),
    ] {
        if rect.width() <= 0.0 {
            continue;
        }
        let response = ui.interact(
            rect,
            ui.id().with(("process_sort", index)),
            egui::Sense::click(),
        );
        let active = *process_tab == index;
        if active || response.hovered() {
            let fill = if active {
                egui::Color32::from_rgba_unmultiplied(0x00, 0xd4, 0xff, 0x26)
            } else {
                egui::Color32::from_rgba_unmultiplied(255, 255, 255, 8)
            };
            ui.painter()
                .rect_filled(rect.shrink2(egui::vec2(0.0, 2.0)), 3.0, fill);
        }
        let text_color = if active {
            egui::Color32::from_rgb(0x00, 0xd4, 0xff)
        } else {
            egui::Color32::from_rgb(0x48, 0x4f, 0x58)
        };
        let position = if index == 1 {
            rect.center()
        } else {
            rect.left_center()
        };
        ui.painter().with_clip_rect(rect).text(
            position,
            alignment,
            label,
            egui::FontId::proportional(SIDEBAR_META_SIZE),
            text_color,
        );
        if response.clicked() {
            *process_tab = index;
        }
    }
}

fn render_narrow_process_row(
    ui: &mut egui::Ui,
    memory_text: &str,
    cpu: f32,
    command_text: &str,
    memory_color: egui::Color32,
    row_fill: egui::Color32,
) -> egui::Response {
    let background = ui.painter().add(egui::Shape::Noop);
    let frame_response = egui::Frame::new()
        .inner_margin(egui::Margin::symmetric(8, 4))
        .show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.label(
                    egui::RichText::new(format!("内存 {memory_text}"))
                        .size(SIDEBAR_META_SIZE)
                        .color(memory_color),
                );
                ui.label(
                    egui::RichText::new(format!("CPU {cpu:.1}%"))
                        .size(SIDEBAR_META_SIZE)
                        .color(egui::Color32::from_rgb(0xe6, 0xed, 0xf3)),
                );
            });
            ui.add(
                egui::Label::new(
                    egui::RichText::new(command_text)
                        .size(SIDEBAR_BODY_SIZE)
                        .color(egui::Color32::from_rgb(0xc9, 0xd1, 0xd9)),
                )
                .wrap(),
            );
        })
        .response;
    let response = ui.interact(
        frame_response.rect,
        frame_response.id.with("click"),
        egui::Sense::click(),
    );
    let fill = if response.hovered() {
        egui::Color32::from_rgba_unmultiplied(0x00, 0xd4, 0xff, 0x0f)
    } else {
        row_fill
    };
    ui.painter().set(
        background,
        egui::Shape::rect_filled(frame_response.rect, 0.0, fill),
    );
    response
}

fn render_narrow_disk_row(
    ui: &mut egui::Ui,
    mount_text: &str,
    percent_text: &str,
    capacity_text: &str,
    colors: DiskRowColors,
    row_fill: egui::Color32,
) {
    egui::Frame::new()
        .fill(row_fill)
        .inner_margin(egui::Margin::symmetric(8, 4))
        .show(ui, |ui| {
            ui.add(
                egui::Label::new(
                    egui::RichText::new(mount_text)
                        .size(SIDEBAR_BODY_SIZE)
                        .color(colors.mount),
                )
                .wrap(),
            );
            ui.horizontal_wrapped(|ui| {
                ui.label(
                    egui::RichText::new(percent_text)
                        .size(SIDEBAR_BODY_SIZE)
                        .color(colors.percent),
                );
                ui.label(
                    egui::RichText::new(capacity_text)
                        .size(SIDEBAR_BODY_SIZE)
                        .color(colors.capacity),
                );
            });
        });
}

fn network_line_points(data: &[f64], rect: egui::Rect, max_value: f64) -> Vec<egui::Pos2> {
    if data.len() < 2 {
        return Vec::new();
    }

    let width = rect.width();
    let height = rect.height();
    if !rect.left().is_finite()
        || !rect.right().is_finite()
        || !rect.top().is_finite()
        || !rect.bottom().is_finite()
        || !width.is_finite()
        || !height.is_finite()
        || width <= 0.0
        || height <= 0.0
        || !max_value.is_finite()
        || data.iter().any(|value| !value.is_finite())
    {
        return Vec::new();
    }

    let max_value = max_value.max(1.0);
    let drawable_height = (height - 4.0).max(0.0);
    data.iter()
        .enumerate()
        .map(|(index, value)| {
            let x = rect.left() + index as f32 / (data.len() - 1) as f32 * width;
            let value = value.clamp(0.0, max_value);
            let y = rect.bottom() - (value / max_value) as f32 * drawable_height;
            egui::pos2(x, y)
        })
        .collect()
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
pub struct NewConnForm {
    pub label: String,
    pub host: String,
    pub port: String,
    pub user: String,
    pub auth_idx: usize, // 0=密钥, 1=密码
    pub key_path: String,
    pub password: String,
    pub group: String,
    pub new_group: String,
    pub status: String,
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
    width_commit_requested: bool,
    pub connections: Vec<SshConnection>,
    pub selected: Option<usize>,
    pub on_connect: Option<SshConnection>,
    pub connections_visible: bool,
    // 连接右键菜单
    pub conn_context_menu: Option<(usize, egui::Pos2)>,
    collapsed_groups: std::collections::HashSet<String>,
    // Dialog visibility
    pub show_new_connection: bool,
    pub show_key_manager: bool,
    // New connection form
    pub new_conn: NewConnForm,
    // SSH key manager
    ssh_keys: Vec<SshKeyInfo>,
    ssh_keys_loaded: bool,
    keygen_type: String,
    keygen_comment: String,
    keygen_status: String,
    // 每个监控身份独立保留网卡选择、进程排序和折线历史
    monitor_views: HashMap<crate::monitor::MonitorKey, MonitorViewState>,
    open_process_manager: Option<OpenProcessManagerAction>,
    open_network_detail: Option<OpenNetworkDetailAction>,
    // SSH 密码重试弹窗
    pub password_prompt: Option<SshConnection>,
    pub password_input: String,
    pub password_error: String,
    pub password_connect: Option<SshConnection>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OpenProcessManagerAction {
    pub key: crate::monitor::MonitorKey,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OpenNetworkDetailAction {
    pub key: crate::monitor::MonitorKey,
    pub initial_iface: Option<String>,
}

fn process_manager_open_action(
    key: &crate::monitor::MonitorKey,
) -> Option<OpenProcessManagerAction> {
    Some(OpenProcessManagerAction { key: key.clone() })
}

fn format_speed(bytes: u64) -> String {
    if bytes >= 1_073_741_824 {
        format!("{:.1}G", bytes as f64 / 1_073_741_824.0)
    } else if bytes >= 1_048_576 {
        format!("{:.1}M", bytes as f64 / 1_048_576.0)
    } else if bytes >= 1024 {
        format!("{:.0}K", bytes as f64 / 1024.0)
    } else {
        format!("{}B", bytes)
    }
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
            width: crate::settings::DEFAULT_SIDEBAR_WIDTH as f32,
            width_commit_requested: false,
            connections,
            selected: None,
            on_connect: None,
            connections_visible: true,
            conn_context_menu: None,
            collapsed_groups: std::collections::HashSet::new(),
            show_new_connection: false,
            show_key_manager: false,
            new_conn: NewConnForm::default(),
            ssh_keys: Vec::new(),
            ssh_keys_loaded: false,
            keygen_type: "ed25519".to_string(),
            keygen_comment: String::new(),
            keygen_status: String::new(),
            monitor_views: HashMap::new(),
            open_process_manager: None,
            open_network_detail: None,
            password_prompt: None,
            password_input: String::new(),
            password_error: String::new(),
            password_connect: None,
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
                    group: group.label.clone(),
                    group_color: color,
                    password: String::new(),
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

    pub fn take_width_commit(&mut self) -> Option<u32> {
        self.width_commit_requested
            .then(|| self.width.round() as u32)
            .inspect(|_| self.width_commit_requested = false)
    }

    pub fn take_open_process_manager(&mut self) -> Option<OpenProcessManagerAction> {
        self.open_process_manager.take()
    }

    pub fn take_open_network_detail(&mut self) -> Option<OpenNetworkDetailAction> {
        self.open_network_detail.take()
    }

    /// 监控数据更新时采样网速（每 2s 一次），不要在每帧 render 里 push。
    pub fn on_monitor_update(
        &mut self,
        key: &crate::monitor::MonitorKey,
        mon: &crate::monitor::MonitorData,
    ) {
        if mon.net_interfaces.is_empty() {
            return;
        }
        let view = self.monitor_views.entry(key.clone()).or_default();
        let selected_exists = view.selected_iface.as_ref().is_some_and(|selected| {
            mon.net_interfaces
                .iter()
                .any(|interface| interface.name == *selected)
        });
        if !selected_exists {
            view.interface_selection_manual = false;
        }
        let sel = if view.interface_selection_manual && selected_exists {
            view.selected_iface.clone()
        } else {
            automatic_network_interface(mon)
        }
        .unwrap_or_else(|| mon.net_interfaces[0].name.clone());
        if view.selected_iface.as_ref() != Some(&sel) {
            view.selected_iface = Some(sel.clone());
        }
        if view.last_chart_iface.as_ref() != Some(&sel) {
            view.net_rx_history.clear();
            view.net_tx_history.clear();
            view.last_chart_iface = Some(sel.clone());
        }
        let (tx_rate, rx_rate) = mon
            .net_interfaces
            .iter()
            .find(|n| n.name == sel)
            .map(|d| (d.tx_rate, d.rx_rate))
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

    pub fn retain_monitor_views(
        &mut self,
        referenced: &std::collections::HashSet<crate::monitor::MonitorKey>,
    ) {
        self.monitor_views.retain(|key, _| {
            matches!(key, crate::monitor::MonitorKey::Local) || referenced.contains(key)
        });
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

    #[cfg(test)]
    pub(crate) fn has_monitor_view_for_test(&self, key: &crate::monitor::MonitorKey) -> bool {
        self.monitor_views.contains_key(key)
    }
}

// ── SSH key helpers (from guishell ssh_keys.rs) ──

mod dialogs;
mod file_dialogs;
mod ui;

#[cfg(test)]
#[path = "sidebar/tests.rs"]
mod ui_tests;
