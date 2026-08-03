use std::cmp::Ordering;
use std::fmt;

use crate::monitor::{
    normalize_process_start_time, MonitorKey, ProcessDetail, ProcessInfo, ProcessStats,
};

const BACKGROUND: egui::Color32 = egui::Color32::from_rgb(0x0d, 0x11, 0x17);
const PANEL: egui::Color32 = egui::Color32::from_rgb(0x16, 0x1b, 0x22);
const PANEL_ALT: egui::Color32 = egui::Color32::from_rgb(0x21, 0x26, 0x2d);
const BORDER: egui::Color32 = egui::Color32::from_rgb(0x30, 0x36, 0x3d);
const TEXT: egui::Color32 = egui::Color32::from_rgb(0xe6, 0xed, 0xf3);
const MUTED: egui::Color32 = egui::Color32::from_rgb(0x8b, 0x94, 0x9e);
const DIM: egui::Color32 = egui::Color32::from_rgb(0x48, 0x4f, 0x58);
const CYAN: egui::Color32 = egui::Color32::from_rgb(0x00, 0xd4, 0xff);
const GREEN: egui::Color32 = egui::Color32::from_rgb(0x3f, 0xb9, 0x50);
const YELLOW: egui::Color32 = egui::Color32::from_rgb(0xd2, 0x99, 0x22);
const RED: egui::Color32 = egui::Color32::from_rgb(0xf8, 0x51, 0x49);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProcessSortKey {
    Pid,
    User,
    ApplicationMemory,
    ResidentMemory,
    Cpu,
    Command,
    StartTime,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SortDirection {
    Ascending,
    Descending,
}

impl SortDirection {
    fn toggle(self) -> Self {
        match self {
            Self::Ascending => Self::Descending,
            Self::Descending => Self::Ascending,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProcessManagerAction {
    Refresh,
    Select { pid: u32, request_id: u64 },
    CloseDetail,
    CopyText(String),
}

pub struct ProcessManagerState {
    target: MonitorKey,
    sort_key: ProcessSortKey,
    sort_direction: SortDirection,
    selected_pid: Option<u32>,
    selected_start_time: Option<String>,
    next_request_id: u64,
    pending_request_id: Option<u64>,
    detail: Option<ProcessDetail>,
    detail_error: Option<String>,
    search_query: String,
    search_request_focus: bool,
}

impl fmt::Debug for ProcessManagerState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ProcessManagerState")
            .field("target", &self.target)
            .field("sort_key", &self.sort_key)
            .field("sort_direction", &self.sort_direction)
            .field("selected_pid", &self.selected_pid)
            .field("next_request_id", &self.next_request_id)
            .field("pending_request_id", &self.pending_request_id)
            .field("detail", &self.detail.as_ref().map(|_| "Present"))
            .field(
                "detail_error",
                &self.detail_error.as_ref().map(|_| "Present"),
            )
            .finish()
    }
}

impl ProcessManagerState {
    pub fn new(target: MonitorKey) -> Self {
        Self {
            target,
            sort_key: ProcessSortKey::Cpu,
            sort_direction: SortDirection::Descending,
            selected_pid: None,
            selected_start_time: None,
            next_request_id: 1,
            pending_request_id: None,
            detail: None,
            detail_error: None,
            search_query: String::new(),
            search_request_focus: false,
        }
    }

    pub fn target(&self) -> &MonitorKey {
        &self.target
    }

    pub fn sort_key(&self) -> ProcessSortKey {
        self.sort_key
    }

    pub fn sort_direction(&self) -> SortDirection {
        self.sort_direction
    }

    pub fn selected_pid(&self) -> Option<u32> {
        self.selected_pid
    }

    pub fn pending_request_id(&self) -> Option<u64> {
        self.pending_request_id
    }

    pub fn detail(&self) -> Option<&ProcessDetail> {
        self.detail.as_ref()
    }

    pub fn detail_error(&self) -> Option<&str> {
        self.detail_error.as_deref()
    }

    pub fn search_query(&self) -> &str {
        &self.search_query
    }

    pub fn set_search_query(&mut self, query: impl Into<String>) {
        self.search_query = query.into();
    }

    pub fn select_sort(&mut self, key: ProcessSortKey) {
        if self.sort_key == key {
            self.sort_direction = self.sort_direction.toggle();
        } else {
            self.sort_key = key;
            self.sort_direction = SortDirection::Descending;
        }
    }

    pub fn select_process(&mut self, pid: u32, start_time: &str) -> ProcessManagerAction {
        let request_id = self.next_request_id;
        self.next_request_id = self.next_request_id.wrapping_add(1).max(1);
        self.selected_pid = Some(pid);
        self.selected_start_time = Some(normalize_process_start_time(start_time));
        self.pending_request_id = Some(request_id);
        self.detail = None;
        self.detail_error = None;
        ProcessManagerAction::Select { pid, request_id }
    }

    pub fn is_current_request(&self, request_id: u64) -> bool {
        self.pending_request_id == Some(request_id)
    }

    pub fn apply_detail(&mut self, request_id: u64, result: Result<ProcessDetail, String>) -> bool {
        if !self.is_current_request(request_id) {
            return false;
        }
        match result {
            Ok(detail) if self.detail_matches_selection(&detail) => {
                self.pending_request_id = None;
                self.detail = Some(detail);
                self.detail_error = None;
            }
            Ok(_) => {
                self.pending_request_id = None;
                self.detail = None;
                self.detail_error = Some("进程已退出或 PID 已被复用".into());
            }
            Err(error) => {
                self.pending_request_id = None;
                self.detail = None;
                self.detail_error = Some(error);
            }
        }
        true
    }

    fn detail_matches_selection(&self, detail: &ProcessDetail) -> bool {
        self.selected_pid == Some(detail.identity.pid)
            && self.selected_start_time.as_deref()
                == Some(normalize_process_start_time(&detail.start_time).as_str())
    }

    pub fn clear_detail(&mut self) {
        self.selected_pid = None;
        self.selected_start_time = None;
        self.pending_request_id = None;
        self.detail = None;
        self.detail_error = None;
    }

    pub fn reset_target(&mut self, target: MonitorKey) {
        if self.target == target {
            return;
        }
        self.target = target;
        self.clear_detail();
    }

    pub fn reconcile_processes(&mut self, processes: &[ProcessInfo]) -> bool {
        let Some(pid) = self.selected_pid else {
            return false;
        };
        let Some(process) = processes.iter().find(|process| process.pid == pid) else {
            // The snapshot is intentionally capped at 100 rows, so absence does not
            // prove that the process exited.
            return false;
        };
        if self.selected_start_time.as_deref()
            == Some(normalize_process_start_time(&process.start_time).as_str())
        {
            return false;
        }

        self.clear_detail();
        self.detail_error = Some("PID 已被复用，旧进程详情已清理".into());
        true
    }
}

pub fn render(
    ctx: &egui::Context,
    state: &mut ProcessManagerState,
    processes: Option<&[ProcessInfo]>,
    stats: Option<&ProcessStats>,
    monitor_error: Option<&str>,
) -> Vec<ProcessManagerAction> {
    let mut actions = Vec::new();
    if let Some(processes) = processes {
        state.reconcile_processes(processes);
    }
    if ctx.input_mut(|input| input.consume_key(egui::Modifiers::CTRL, egui::Key::F)) {
        state.search_request_focus = true;
    }
    egui::CentralPanel::default()
        .frame(
            egui::Frame::new()
                .fill(BACKGROUND)
                .inner_margin(egui::Margin::same(0)),
        )
        .show(ctx, |ui| {
            render_toolbar(ui, state, processes, stats, &mut actions);
            render_monitor_state(ui, processes, monitor_error);
            if has_detail_area(state) {
                let maximum_height = detail_panel_max_height(ui.available_height());
                egui::TopBottomPanel::bottom("process_manager_detail_panel")
                    .resizable(true)
                    .default_height(300.0)
                    .min_height(120.0)
                    .max_height(maximum_height)
                    .frame(
                        egui::Frame::new()
                            .fill(BACKGROUND)
                            .inner_margin(egui::Margin::same(0)),
                    )
                    .show_inside(ui, |ui| {
                        ui.set_min_height(ui.available_height());
                        render_detail(ui, state, &mut actions);
                    });
            }
            render_processes(ui, state, processes, &mut actions);
        });
    actions
}

fn has_detail_area(state: &ProcessManagerState) -> bool {
    state.detail.is_some() || state.detail_error.is_some() || state.pending_request_id.is_some()
}

fn detail_panel_max_height(available_height: f32) -> f32 {
    let available_height = if available_height.is_finite() {
        available_height.max(0.0)
    } else {
        0.0
    };
    (available_height * 0.55).max(120.0)
}

fn render_toolbar(
    ui: &mut egui::Ui,
    state: &mut ProcessManagerState,
    processes: Option<&[ProcessInfo]>,
    stats: Option<&ProcessStats>,
    actions: &mut Vec<ProcessManagerAction>,
) {
    egui::Frame::new()
        .fill(PANEL)
        .stroke(egui::Stroke::new(1.0, BORDER))
        .inner_margin(egui::Margin::symmetric(10, 6))
        .show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                let count = processes.map(<[ProcessInfo]>::len);
                let filtered_count =
                    processes.map(|items| filtered_processes(items, &state.search_query).len());
                let title = match count {
                    Some(count) if !state.search_query.trim().is_empty() => format!(
                        "进程列表 - {}（筛选 {}/{count}）",
                        state.target.status_text(),
                        filtered_count.unwrap_or(0),
                    ),
                    Some(count) => format!(
                        "进程列表 - {}（当前显示 {count}）",
                        state.target.status_text()
                    ),
                    None => format!("进程列表 - {}", state.target.status_text()),
                };
                ui.label(egui::RichText::new(title).strong().color(TEXT));
                if let Some(stats) = stats {
                    render_stats(ui, stats);
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui
                        .add(
                            egui::Button::new(egui::RichText::new("刷新").size(11.0).color(MUTED))
                                .fill(PANEL_ALT)
                                .stroke(egui::Stroke::new(1.0, BORDER)),
                        )
                        .on_hover_text("立即刷新当前目标的进程快照")
                        .clicked()
                    {
                        actions.push(ProcessManagerAction::Refresh);
                    }
                    let search = ui.add_sized(
                        [220.0_f32.min(ui.available_width().max(100.0)), 26.0],
                        egui::TextEdit::singleline(&mut state.search_query)
                            .hint_text("搜索进程、命令、PID 或用户…"),
                    );
                    if state.search_request_focus {
                        search.request_focus();
                        state.search_request_focus = false;
                    }
                    if search.has_focus()
                        && ui.input_mut(|input| {
                            input.consume_key(egui::Modifiers::NONE, egui::Key::Escape)
                        })
                    {
                        if state.search_query.is_empty() {
                            ui.memory_mut(|memory| memory.surrender_focus(search.id));
                        } else {
                            state.search_query.clear();
                        }
                    }
                });
            });
            if let Some(stats) = stats {
                if stats.total > 10_000 {
                    ui.colored_label(
                        RED,
                        format!("⚠ 进程数异常（{}），可能存在进程泄漏", stats.total),
                    );
                }
                if stats.zombie > 100 {
                    ui.colored_label(
                        RED,
                        format!("⚠ 僵尸进程过多（{}），建议排查父进程", stats.zombie),
                    );
                }
            }
        });
}

fn render_stats(ui: &mut egui::Ui, stats: &ProcessStats) {
    ui.label(egui::RichText::new("共").size(10.0).color(DIM));
    ui.label(
        egui::RichText::new(stats.total.to_string())
            .size(10.0)
            .color(if stats.total > 10_000 { RED } else { TEXT }),
    );
    ui.label(egui::RichText::new("运行").size(10.0).color(DIM));
    ui.label(
        egui::RichText::new(stats.running.to_string())
            .size(10.0)
            .color(GREEN),
    );
    ui.label(egui::RichText::new("休眠").size(10.0).color(DIM));
    ui.label(
        egui::RichText::new(stats.sleeping.to_string())
            .size(10.0)
            .color(MUTED),
    );
    ui.label(egui::RichText::new("僵尸").size(10.0).color(DIM));
    ui.label(
        egui::RichText::new(stats.zombie.to_string())
            .size(10.0)
            .color(if stats.zombie > 0 { RED } else { DIM }),
    );
    if stats.stopped > 0 {
        ui.label(egui::RichText::new("停止").size(10.0).color(DIM));
        ui.label(
            egui::RichText::new(stats.stopped.to_string())
                .size(10.0)
                .color(YELLOW),
        );
    }
}

fn render_monitor_state(
    ui: &mut egui::Ui,
    processes: Option<&[ProcessInfo]>,
    monitor_error: Option<&str>,
) {
    match (processes, monitor_error) {
        (None, None) => {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(egui::RichText::new("正在加载进程…").color(MUTED));
            });
        }
        (None, Some(error)) => {
            state_banner(ui, RED, format!("加载进程列表失败：{error}"));
        }
        (Some(_), Some(error)) => {
            state_banner(
                ui,
                YELLOW,
                format!("⚠ 当前显示的是上次成功快照，数据可能已过期：{error}"),
            );
        }
        (Some(_), None) => {}
    }
}

fn state_banner(ui: &mut egui::Ui, color: egui::Color32, text: String) {
    egui::Frame::new()
        .fill(color.gamma_multiply(0.12))
        .stroke(egui::Stroke::new(1.0, color.gamma_multiply(0.45)))
        .inner_margin(egui::Margin::symmetric(10, 5))
        .show(ui, |ui| {
            ui.label(egui::RichText::new(text).size(10.0).color(color));
        });
}

fn render_processes(
    ui: &mut egui::Ui,
    state: &mut ProcessManagerState,
    processes: Option<&[ProcessInfo]>,
    actions: &mut Vec<ProcessManagerAction>,
) {
    let table_height = ui.available_height().max(0.0);
    let rows_height = (table_height - 28.0).max(0.0);
    egui::ScrollArea::horizontal()
        .id_salt("process_manager_table")
        .auto_shrink([false, false])
        .max_height(table_height)
        .show(ui, |ui| {
            ui.set_min_width(980.0);
            render_column_headers(ui, state);

            let Some(processes) = processes else {
                return;
            };
            if processes.is_empty() {
                ui.vertical_centered(|ui| {
                    ui.add_space(32.0);
                    ui.label(egui::RichText::new("暂无进程数据").color(MUTED));
                    ui.label(
                        egui::RichText::new("进程采集已成功，但当前列表为空")
                            .size(10.0)
                            .color(DIM),
                    );
                });
                return;
            }

            let filtered = filtered_processes(processes, &state.search_query);
            if filtered.is_empty() && !state.search_query.trim().is_empty() {
                ui.vertical_centered(|ui| {
                    ui.add_space(24.0);
                    ui.label(egui::RichText::new("没有匹配的进程").color(MUTED));
                });
                return;
            }
            let sorted = sorted_process_refs(filtered, state.sort_key, state.sort_direction);
            egui::ScrollArea::vertical()
                .id_salt("process_manager_table_rows")
                .auto_shrink([false, false])
                .max_height(rows_height)
                .show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 0.0;
                    for process in sorted {
                        let selected = state.selected_pid == Some(process.pid);
                        if render_process_row(ui, process, selected).clicked() {
                            actions.push(state.select_process(process.pid, &process.start_time));
                        }
                    }
                });
        });
}

fn render_column_headers(ui: &mut egui::Ui, state: &mut ProcessManagerState) {
    egui::Frame::new()
        .fill(PANEL_ALT)
        .stroke(egui::Stroke::new(1.0, BORDER))
        .inner_margin(egui::Margin::symmetric(4, 2))
        .show(ui, |ui| {
            ui.set_min_width(980.0);
            ui.horizontal(|ui| {
                sort_header(ui, state, "PID", ProcessSortKey::Pid, 60.0);
                sort_header(ui, state, "用户", ProcessSortKey::User, 90.0);
                sort_header(
                    ui,
                    state,
                    "应用内存",
                    ProcessSortKey::ApplicationMemory,
                    75.0,
                );
                sort_header(ui, state, "驻留内存", ProcessSortKey::ResidentMemory, 75.0);
                sort_header(ui, state, "CPU", ProcessSortKey::Cpu, 60.0);
                sort_header(ui, state, "命令", ProcessSortKey::Command, 390.0);
                sort_header(ui, state, "启动时间", ProcessSortKey::StartTime, 180.0);
            });
        });
}

fn sort_header(
    ui: &mut egui::Ui,
    state: &mut ProcessManagerState,
    label: &str,
    key: ProcessSortKey,
    width: f32,
) {
    let arrow = if state.sort_key == key {
        match state.sort_direction {
            SortDirection::Ascending => " ↑",
            SortDirection::Descending => " ↓",
        }
    } else {
        ""
    };
    if ui
        .add_sized(
            egui::vec2(width, 20.0),
            egui::Button::new(
                egui::RichText::new(format!("{label}{arrow}"))
                    .size(10.0)
                    .color(if state.sort_key == key { CYAN } else { MUTED }),
            )
            .frame(false),
        )
        .clicked()
    {
        state.select_sort(key);
    }
}

fn render_process_row(ui: &mut egui::Ui, process: &ProcessInfo, selected: bool) -> egui::Response {
    let row_height = 22.0;
    let (rect, response) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), row_height),
        egui::Sense::click(),
    );
    let fill = if selected {
        CYAN.gamma_multiply(0.14)
    } else if response.hovered() {
        PANEL_ALT
    } else {
        BACKGROUND
    };
    ui.painter().rect_filled(rect, 0.0, fill);

    let columns = [
        (
            60.0,
            process.pid.to_string(),
            MUTED,
            egui::Align2::LEFT_CENTER,
        ),
        (90.0, process.user.clone(), TEXT, egui::Align2::LEFT_CENTER),
        (
            75.0,
            process.mem_mb.clone(),
            TEXT,
            egui::Align2::RIGHT_CENTER,
        ),
        (
            75.0,
            process.resident_mem_mb.clone(),
            MUTED,
            egui::Align2::RIGHT_CENTER,
        ),
        (
            60.0,
            format!("{:.1}", process.cpu),
            TEXT,
            egui::Align2::RIGHT_CENTER,
        ),
        (
            390.0,
            display_command(process).to_string(),
            TEXT,
            egui::Align2::LEFT_CENTER,
        ),
        (
            180.0,
            process.start_time.clone(),
            DIM,
            egui::Align2::LEFT_CENTER,
        ),
    ];
    let mut left = rect.left() + 4.0;
    for (width, text, color, align) in columns {
        let position = match align {
            egui::Align2::RIGHT_CENTER => egui::pos2(left + width - 4.0, rect.center().y),
            _ => egui::pos2(left + 2.0, rect.center().y),
        };
        let clip =
            egui::Rect::from_min_size(egui::pos2(left, rect.top()), egui::vec2(width, row_height));
        ui.painter().with_clip_rect(clip).text(
            position,
            align,
            text,
            egui::FontId::monospace(10.0),
            color,
        );
        left += width + ui.spacing().item_spacing.x;
    }
    response
}

fn sorted_processes(
    processes: &[ProcessInfo],
    key: ProcessSortKey,
    direction: SortDirection,
) -> Vec<&ProcessInfo> {
    let mut sorted = processes.iter().collect::<Vec<_>>();
    sorted.sort_by(|left, right| {
        let ordering = compare_processes(left, right, key).then_with(|| left.pid.cmp(&right.pid));
        match direction {
            SortDirection::Ascending => ordering,
            SortDirection::Descending => ordering.reverse(),
        }
    });
    sorted
}

fn sorted_process_refs(
    mut processes: Vec<&ProcessInfo>,
    key: ProcessSortKey,
    direction: SortDirection,
) -> Vec<&ProcessInfo> {
    processes.sort_by(|left, right| {
        let ordering = compare_processes(left, right, key).then_with(|| left.pid.cmp(&right.pid));
        match direction {
            SortDirection::Ascending => ordering,
            SortDirection::Descending => ordering.reverse(),
        }
    });
    processes
}

fn filtered_processes<'a>(processes: &'a [ProcessInfo], query: &str) -> Vec<&'a ProcessInfo> {
    let query = query.trim().to_lowercase();
    if query.is_empty() {
        return processes.iter().collect();
    }
    processes
        .iter()
        .filter(|process| {
            process.pid.to_string().contains(&query)
                || process.user.to_lowercase().contains(&query)
                || process.name.to_lowercase().contains(&query)
                || process.command.to_lowercase().contains(&query)
        })
        .collect()
}

fn compare_processes(left: &ProcessInfo, right: &ProcessInfo, key: ProcessSortKey) -> Ordering {
    match key {
        ProcessSortKey::Pid => left.pid.cmp(&right.pid),
        ProcessSortKey::User => left.user.cmp(&right.user),
        ProcessSortKey::ApplicationMemory => left.mem_bytes.cmp(&right.mem_bytes),
        ProcessSortKey::ResidentMemory => left.resident_mem_bytes.cmp(&right.resident_mem_bytes),
        ProcessSortKey::Cpu => left.cpu.total_cmp(&right.cpu),
        ProcessSortKey::Command => display_command(left).cmp(display_command(right)),
        ProcessSortKey::StartTime => left.start_time.cmp(&right.start_time),
    }
}

fn display_command(process: &ProcessInfo) -> &str {
    if process.command.is_empty() {
        &process.name
    } else {
        &process.command
    }
}

fn render_detail(
    ui: &mut egui::Ui,
    state: &mut ProcessManagerState,
    actions: &mut Vec<ProcessManagerAction>,
) {
    if state.pending_request_id.is_some() {
        ui.separator();
        ui.horizontal(|ui| {
            ui.spinner();
            ui.label(egui::RichText::new("正在加载进程详情…").color(MUTED));
        });
    }
    if let Some(error) = state.detail_error.as_deref() {
        state_banner(ui, RED, format!("加载进程详情失败：{error}"));
        if ui.small_button("关闭").clicked() {
            state.clear_detail();
            actions.push(ProcessManagerAction::CloseDetail);
        }
        return;
    }

    let Some(detail) = state.detail.as_ref() else {
        return;
    };
    let mut close = false;
    let copy_all = format_process_detail(detail);
    ui.separator();
    egui::Frame::new()
        .fill(PANEL)
        .inner_margin(egui::Margin::symmetric(10, 6))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new(format!("PID {} — {}", detail.identity.pid, detail.name))
                        .strong()
                        .color(CYAN),
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui
                        .add(egui::Button::new(egui::RichText::new("×").size(16.0)).frame(false))
                        .on_hover_text("关闭详情并清理敏感数据")
                        .clicked()
                    {
                        close = true;
                    }
                    if ui
                        .add(
                            egui::Button::new(
                                egui::RichText::new("复制全部").size(10.5).color(TEXT),
                            )
                            .fill(PANEL_ALT)
                            .stroke(egui::Stroke::new(1.0, BORDER)),
                        )
                        .on_hover_text("复制当前进程展示的全部取证信息")
                        .clicked()
                    {
                        actions.push(ProcessManagerAction::CopyText(copy_all.clone()));
                    }
                });
            });

            egui::ScrollArea::vertical()
                .id_salt("process_manager_detail")
                .max_height((ui.available_height() - 8.0).max(80.0))
                .show(ui, |ui| {
                    render_basic_detail(ui, detail);
                    render_ancestors(ui, detail);
                    render_copyable_section(
                        ui,
                        "完整命令行",
                        value_or_dash(&detail.command),
                        GREEN,
                    );
                    if detail.environ.is_empty() {
                        ui.label(
                            egui::RichText::new("环境变量不可用或无读取权限")
                                .size(10.0)
                                .color(DIM),
                        );
                    } else {
                        ui.add_space(6.0);
                        ui.label(
                            egui::RichText::new(format!("环境变量（{}）", detail.environ.len()))
                                .strong()
                                .color(MUTED),
                        );
                        let environment = detail
                            .environ
                            .iter()
                            .map(|variable| format!("{}={}", variable.key, variable.value))
                            .collect::<Vec<_>>()
                            .join("\n");
                        render_copyable_text(ui, environment, TEXT);
                    }
                });
        });
    if close {
        state.clear_detail();
        actions.push(ProcessManagerAction::CloseDetail);
    }
}

fn render_basic_detail(ui: &mut egui::Ui, detail: &ProcessDetail) {
    ui.add_space(4.0);
    ui.label(egui::RichText::new("基本信息").strong().color(MUTED));
    egui::Grid::new("process_manager_basic_detail")
        .num_columns(2)
        .spacing(egui::vec2(14.0, 3.0))
        .show(ui, |ui| {
            detail_row(ui, "PID", detail.identity.pid.to_string(), TEXT);
            detail_row(ui, "用户", value_or_dash(&detail.user), TEXT);
            detail_row(ui, "驻留内存", detail.mem_mb.clone(), TEXT);
            if let Some(metric) = &detail.platform_memory {
                detail_row(ui, metric.label, metric.text.clone(), CYAN);
            }
            detail_row(ui, "启动时间", unavailable_reason(&detail.start_time), TEXT);
            detail_row(
                ui,
                &format!("/proc/{}/exe", detail.identity.pid),
                unavailable_reason(&detail.executable),
                GREEN,
            );
            detail_row(ui, "工作目录", value_or_dash(&detail.working_dir), TEXT);
        });
}

fn detail_row(ui: &mut egui::Ui, label: &str, value: impl Into<String>, color: egui::Color32) {
    ui.label(egui::RichText::new(label).size(10.0).color(DIM));
    let value = value.into();
    let response = ui.add(
        egui::Label::new(
            egui::RichText::new(&value)
                .monospace()
                .size(10.0)
                .color(color),
        )
        .selectable(true),
    );
    response.context_menu(|ui| {
        if ui.button("复制").clicked() {
            ui.ctx().copy_text(value.clone());
            ui.close_menu();
        }
    });
    ui.end_row();
}

fn render_ancestors(ui: &mut egui::Ui, detail: &ProcessDetail) {
    if detail.ancestors.is_empty() {
        return;
    }
    ui.add_space(6.0);
    ui.label(egui::RichText::new("祖先链").strong().color(MUTED));
    for (depth, ancestor) in detail.ancestors.iter().rev().enumerate() {
        ui.horizontal(|ui| {
            ui.add_space(depth as f32 * 14.0);
            if depth > 0 {
                ui.label(egui::RichText::new("└─").monospace().color(DIM));
            }
            ui.label(
                egui::RichText::new(ancestor.pid.to_string())
                    .monospace()
                    .color(CYAN),
            );
            ui.label(egui::RichText::new(&ancestor.name).monospace().color(MUTED));
            if !ancestor.command.is_empty() && ancestor.command != ancestor.name {
                ui.label(
                    egui::RichText::new(&ancestor.command)
                        .monospace()
                        .size(10.0)
                        .color(DIM),
                );
            }
        });
    }
}

fn render_copyable_section(
    ui: &mut egui::Ui,
    title: &str,
    value: impl Into<String>,
    color: egui::Color32,
) {
    ui.add_space(6.0);
    ui.label(egui::RichText::new(title).strong().color(MUTED));
    egui::Frame::new()
        .fill(BACKGROUND)
        .inner_margin(egui::Margin::symmetric(7, 5))
        .show(ui, |ui| {
            render_copyable_text(ui, value.into(), color);
        });
}

fn render_copyable_text(ui: &mut egui::Ui, mut value: String, color: egui::Color32) {
    let output = egui::TextEdit::multiline(&mut value)
        .font(egui::TextStyle::Monospace)
        .text_color(color)
        .desired_width(f32::INFINITY)
        .frame(false)
        .show(ui);
    let selection = output.cursor_range.and_then(|range| {
        let range = range.as_sorted_char_range();
        (range.start < range.end).then(|| {
            value
                .chars()
                .skip(range.start)
                .take(range.end - range.start)
                .collect::<String>()
        })
    });
    output.response.context_menu(|ui| {
        if ui.button("复制").clicked() {
            ui.ctx()
                .copy_text(selection.clone().unwrap_or_else(|| value.clone()));
            ui.close_menu();
        }
    });
}

fn unavailable_reason(value: &str) -> String {
    if value.trim().is_empty() {
        "不可读取（权限不足或进程已退出）".into()
    } else {
        value.to_string()
    }
}

fn format_process_detail(detail: &ProcessDetail) -> String {
    let mut lines = vec![
        format!("PID: {}", detail.identity.pid),
        format!("进程名: {}", value_or_dash(&detail.name)),
        format!("用户: {}", value_or_dash(&detail.user)),
        format!("状态: {}", value_or_dash(&detail.state)),
        format!("CPU: {:.1}%", detail.cpu),
        format!("驻留内存: {}", detail.mem_mb),
        format!("启动时间: {}", unavailable_reason(&detail.start_time)),
        format!(
            "/proc/{}/exe: {}",
            detail.identity.pid,
            unavailable_reason(&detail.executable)
        ),
        format!("工作目录: {}", unavailable_reason(&detail.working_dir)),
        format!("完整命令行: {}", unavailable_reason(&detail.command)),
    ];
    if let Some(metric) = &detail.platform_memory {
        lines.push(format!("{}: {}", metric.label, metric.text));
    }
    if !detail.ancestors.is_empty() {
        lines.push(String::new());
        lines.push("祖先链:".into());
        lines.extend(
            detail.ancestors.iter().rev().map(|ancestor| {
                format!("  {} {} {}", ancestor.pid, ancestor.name, ancestor.command)
            }),
        );
    }
    if !detail.environ.is_empty() {
        lines.push(String::new());
        lines.push("环境变量:".into());
        lines.extend(
            detail
                .environ
                .iter()
                .map(|variable| format!("{}={}", variable.key, variable.value)),
        );
    }
    lines.join("\n")
}

fn value_or_dash(value: &str) -> String {
    if value.is_empty() {
        "—".to_string()
    } else {
        value.to_string()
    }
}

#[cfg(test)]
#[path = "process_manager/tests.rs"]
mod tests;
