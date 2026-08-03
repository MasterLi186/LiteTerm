use std::cmp::Ordering;
use std::collections::BTreeMap;
use std::fmt;
use std::io::Read;
#[cfg(target_os = "linux")]
use std::process::{Command, Stdio};

use crate::monitor::{MonitorKey, NetIfaceInfo};

pub(crate) const MAX_NETWORK_DETAIL_BYTES: usize = 512 * 1024;
pub(crate) const NETWORK_DETAIL_COMMAND: &str = "LC_ALL=C; export LC_ALL; printf '%s\\n' '===IP==='; ip -4 -o addr show 2>/dev/null | awk '{print $2, $4}'; printf '%s\\n' '===SS==='; ss -Htnp4 2>/dev/null";

const BACKGROUND: egui::Color32 = egui::Color32::from_rgb(0x0d, 0x11, 0x17);
const PANEL: egui::Color32 = egui::Color32::from_rgb(0x16, 0x1b, 0x22);
const PANEL_ALT: egui::Color32 = egui::Color32::from_rgb(0x21, 0x26, 0x2d);
const BORDER: egui::Color32 = egui::Color32::from_rgb(0x30, 0x36, 0x3d);
const TEXT: egui::Color32 = egui::Color32::from_rgb(0xe6, 0xed, 0xf3);
const MUTED: egui::Color32 = egui::Color32::from_rgb(0x8b, 0x94, 0x9e);
const DIM: egui::Color32 = egui::Color32::from_rgb(0x48, 0x4f, 0x58);
const CYAN: egui::Color32 = egui::Color32::from_rgb(0x00, 0xd4, 0xff);
const GREEN: egui::Color32 = egui::Color32::from_rgb(0x3f, 0xb9, 0x50);
const RED: egui::Color32 = egui::Color32::from_rgb(0xf8, 0x51, 0x49);
const YELLOW: egui::Color32 = egui::Color32::from_rgb(0xd2, 0x99, 0x22);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NetConnection {
    pub state: String,
    pub local_address: String,
    pub remote_address: String,
    pub pid: Option<u32>,
    pub process: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NetworkDetailSnapshot {
    pub interface_addresses: BTreeMap<String, Vec<String>>,
    pub connections: Vec<NetConnection>,
}

impl NetworkDetailSnapshot {
    pub fn interfaces(&self) -> impl Iterator<Item = &str> {
        self.interface_addresses.keys().map(String::as_str)
    }

    pub fn primary_address(&self, interface: &str) -> Option<&str> {
        self.interface_addresses
            .get(interface)
            .and_then(|addresses| addresses.first())
            .map(String::as_str)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NetworkSortKey {
    State,
    LocalAddress,
    RemoteAddress,
    Pid,
    Process,
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
pub enum NetworkDetailAction {
    Refresh { request_id: u64 },
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct NetworkInterfaceRate {
    pub rx_rate: u64,
    pub tx_rate: u64,
}

pub struct NetworkDetailState {
    target: MonitorKey,
    selected_interface: Option<String>,
    interface_rates: BTreeMap<String, NetworkInterfaceRate>,
    sort_key: NetworkSortKey,
    sort_direction: SortDirection,
    next_request_id: u64,
    pending_request_id: Option<u64>,
    has_requested: bool,
    snapshot: Option<NetworkDetailSnapshot>,
    error: Option<String>,
}

impl fmt::Debug for NetworkDetailState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NetworkDetailState")
            .field("target", &self.target)
            .field("selected_interface", &self.selected_interface)
            .field("interface_rate_count", &self.interface_rates.len())
            .field("sort_key", &self.sort_key)
            .field("sort_direction", &self.sort_direction)
            .field("next_request_id", &self.next_request_id)
            .field("pending_request_id", &self.pending_request_id)
            .field(
                "snapshot",
                &self
                    .snapshot
                    .as_ref()
                    .map(|snapshot| snapshot.connections.len()),
            )
            .field("error", &self.error.as_ref().map(|_| "Present"))
            .finish()
    }
}

impl NetworkDetailState {
    pub fn new(target: MonitorKey, initial_interface: Option<String>) -> Self {
        Self {
            target,
            selected_interface: normalize_selected_interface(initial_interface),
            interface_rates: BTreeMap::new(),
            sort_key: NetworkSortKey::Process,
            sort_direction: SortDirection::Ascending,
            next_request_id: 1,
            pending_request_id: None,
            has_requested: false,
            snapshot: None,
            error: None,
        }
    }

    pub fn target(&self) -> &MonitorKey {
        &self.target
    }

    pub fn selected_interface(&self) -> Option<&str> {
        self.selected_interface.as_deref()
    }

    pub fn select_interface(&mut self, interface: Option<String>) {
        self.selected_interface = normalize_selected_interface(interface);
    }

    pub fn update_rates(&mut self, interfaces: &[NetIfaceInfo]) {
        self.interface_rates.clear();
        self.interface_rates
            .extend(interfaces.iter().map(|interface| {
                (
                    normalize_interface_name(&interface.name).to_string(),
                    NetworkInterfaceRate {
                        rx_rate: interface.rx_rate,
                        tx_rate: interface.tx_rate,
                    },
                )
            }));
    }

    pub fn selected_rates(&self) -> Option<NetworkInterfaceRate> {
        self.selected_interface
            .as_deref()
            .and_then(|interface| self.interface_rates.get(interface))
            .copied()
    }

    pub fn sort_key(&self) -> NetworkSortKey {
        self.sort_key
    }

    pub fn sort_direction(&self) -> SortDirection {
        self.sort_direction
    }

    pub fn select_sort(&mut self, key: NetworkSortKey) {
        if self.sort_key == key {
            self.sort_direction = self.sort_direction.toggle();
        } else {
            self.sort_key = key;
            self.sort_direction = SortDirection::Ascending;
        }
    }

    pub fn pending_request_id(&self) -> Option<u64> {
        self.pending_request_id
    }

    pub fn snapshot(&self) -> Option<&NetworkDetailSnapshot> {
        self.snapshot.as_ref()
    }

    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    pub fn request_refresh(&mut self) -> Option<NetworkDetailAction> {
        if self.pending_request_id.is_some() {
            return None;
        }
        let request_id = self.next_request_id;
        self.next_request_id = self.next_request_id.wrapping_add(1).max(1);
        self.pending_request_id = Some(request_id);
        self.has_requested = true;
        self.error = None;
        Some(NetworkDetailAction::Refresh { request_id })
    }

    pub fn is_current_request(&self, request_id: u64) -> bool {
        self.pending_request_id == Some(request_id)
    }

    pub fn cancel_pending_refresh(&mut self) -> bool {
        self.pending_request_id.take().is_some()
    }

    pub fn apply_snapshot(
        &mut self,
        request_id: u64,
        result: Result<NetworkDetailSnapshot, String>,
    ) -> bool {
        if !self.is_current_request(request_id) {
            return false;
        }
        self.pending_request_id = None;
        match result {
            Ok(snapshot) => {
                if self.selected_interface.is_none() {
                    self.selected_interface = snapshot.interfaces().next().map(str::to_string);
                }
                self.snapshot = Some(snapshot);
                self.error = None;
            }
            Err(error) => {
                self.error = Some(error);
            }
        }
        true
    }

    pub fn reset_target(&mut self, target: MonitorKey, initial_interface: Option<String>) {
        if self.target == target {
            return;
        }
        *self = Self::new(target, initial_interface);
    }

    pub fn visible_connections(&self) -> Vec<&NetConnection> {
        let Some(snapshot) = self.snapshot.as_ref() else {
            return Vec::new();
        };
        sorted_connections(
            filtered_connections(snapshot, self.selected_interface.as_deref()),
            self.sort_key,
            self.sort_direction,
        )
    }
}

pub(crate) fn read_network_detail_bounded(
    reader: impl Read,
) -> Result<NetworkDetailSnapshot, String> {
    read_network_detail_bounded_for(reader, "远端")
}

fn read_local_network_detail_bounded(reader: impl Read) -> Result<NetworkDetailSnapshot, String> {
    read_network_detail_bounded_for(reader, "本地")
}

fn read_network_detail_bounded_for(
    mut reader: impl Read,
    source: &str,
) -> Result<NetworkDetailSnapshot, String> {
    let mut bytes = Vec::with_capacity(8 * 1024);
    let mut buffer = [0_u8; 8 * 1024];
    loop {
        let remaining = MAX_NETWORK_DETAIL_BYTES + 1 - bytes.len();
        let read_len = buffer.len().min(remaining);
        let count = reader
            .read(&mut buffer[..read_len])
            .map_err(|error| format!("读取{source}网络详情失败: {error}"))?;
        if count == 0 {
            break;
        }
        bytes.extend_from_slice(&buffer[..count]);
        if bytes.len() > MAX_NETWORK_DETAIL_BYTES {
            return Err(format!("{source}网络详情超过 512KiB 限制"));
        }
    }
    let output = String::from_utf8(bytes).map_err(|_| format!("{source}网络详情不是有效 UTF-8"))?;
    parse_network_detail(&output).map_err(|error| format!("解析{source}网络详情失败: {error}"))
}

#[cfg(target_os = "linux")]
pub(crate) fn collect_local() -> Result<NetworkDetailSnapshot, String> {
    let mut child = Command::new("sh")
        .arg("-c")
        .arg(NETWORK_DETAIL_COMMAND)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| format!("启动本地网络详情命令失败: {error}"))?;
    let Some(stdout) = child.stdout.take() else {
        let _ = child.kill();
        let _ = child.wait();
        return Err("读取本地网络详情失败: 无法获取命令输出".to_string());
    };

    let result = read_local_network_detail_bounded(stdout);
    if result.is_err() {
        let _ = child.kill();
    }
    let status = child.wait().map_err(|error| {
        if result.is_ok() {
            format!("等待本地网络详情命令失败: {error}")
        } else {
            result
                .as_ref()
                .err()
                .cloned()
                .unwrap_or_else(|| format!("等待本地网络详情命令失败: {error}"))
        }
    })?;
    match result {
        Err(error) => Err(error),
        Ok(_) if !status.success() => Err(format!(
            "本地网络详情命令执行失败（退出状态：{}）",
            status
                .code()
                .map(|code| code.to_string())
                .unwrap_or_else(|| "被信号终止".to_string())
        )),
        Ok(snapshot) => Ok(snapshot),
    }
}

#[cfg(not(target_os = "linux"))]
pub(crate) fn collect_local() -> Result<NetworkDetailSnapshot, String> {
    Err("当前平台暂不支持本机网络连接详情；远端 Linux 主机仍可查看".to_string())
}

pub fn parse_network_detail(output: &str) -> Result<NetworkDetailSnapshot, String> {
    let Some((prefix, after_ip)) = output.split_once("===IP===") else {
        return Err("网络详情数据缺少 ===IP=== 标记".to_string());
    };
    if !prefix.trim().is_empty() {
        return Err("网络详情数据在 ===IP=== 前包含意外内容".to_string());
    }
    let Some((ip_section, ss_section)) = after_ip.split_once("===SS===") else {
        return Err("网络详情数据缺少 ===SS=== 标记".to_string());
    };

    let mut interface_addresses = BTreeMap::<String, Vec<String>>::new();
    for line in ip_section
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
    {
        let mut fields = line.split_whitespace();
        let (Some(interface), Some(cidr)) = (fields.next(), fields.next()) else {
            continue;
        };
        let interface = normalize_interface_name(interface);
        let address = cidr.split('/').next().unwrap_or("").trim();
        if interface.is_empty() || address.parse::<std::net::Ipv4Addr>().is_err() {
            continue;
        }
        let addresses = interface_addresses
            .entry(interface.to_string())
            .or_default();
        if !addresses.iter().any(|existing| existing == address) {
            addresses.push(address.to_string());
        }
    }

    let connections = ss_section.lines().filter_map(parse_ss_connection).collect();
    Ok(NetworkDetailSnapshot {
        interface_addresses,
        connections,
    })
}

fn normalize_interface_name(interface: &str) -> &str {
    interface
        .split_once('@')
        .map(|(name, _)| name)
        .unwrap_or(interface)
}

fn normalize_selected_interface(interface: Option<String>) -> Option<String> {
    interface
        .filter(|name| !name.is_empty())
        .map(|name| normalize_interface_name(&name).to_string())
}

fn parse_ss_connection(line: &str) -> Option<NetConnection> {
    let fields = line.split_whitespace().collect::<Vec<_>>();
    if fields.len() < 5 {
        return None;
    }
    let process_info = fields
        .get(5..)
        .map(|fields| fields.join(" "))
        .unwrap_or_default();
    let (process, pid) = parse_process_info(&process_info);
    Some(NetConnection {
        state: fields[0].to_string(),
        local_address: fields[3].to_string(),
        remote_address: fields[4].to_string(),
        pid,
        process,
    })
}

fn parse_process_info(value: &str) -> (String, Option<u32>) {
    let Some(users) = value.find("users:((\"") else {
        return (String::new(), None);
    };
    let name_start = users + "users:((\"".len();
    let Some(name_end_offset) = value[name_start..].find('"') else {
        return (String::new(), None);
    };
    let name_end = name_start + name_end_offset;
    let process = value[name_start..name_end].to_string();
    let pid = value[name_end..]
        .find("pid=")
        .and_then(|offset| {
            value[name_end + offset + "pid=".len()..]
                .split(|character: char| !character.is_ascii_digit())
                .next()
        })
        .and_then(|pid| pid.parse::<u32>().ok());
    (process, pid)
}

fn filtered_connections<'a>(
    snapshot: &'a NetworkDetailSnapshot,
    interface: Option<&str>,
) -> Vec<&'a NetConnection> {
    let Some(interface) = interface.filter(|interface| !interface.is_empty()) else {
        return snapshot.connections.iter().collect();
    };
    let Some(addresses) = snapshot.interface_addresses.get(interface) else {
        return Vec::new();
    };
    snapshot
        .connections
        .iter()
        .filter(|connection| {
            endpoint_host(&connection.local_address)
                .map(|host| addresses.iter().any(|address| address == host))
                .unwrap_or(false)
        })
        .collect()
}

fn endpoint_host(endpoint: &str) -> Option<&str> {
    if let Some(rest) = endpoint.strip_prefix('[') {
        return rest.split_once("]:").map(|(host, _)| host);
    }
    endpoint.rsplit_once(':').map(|(host, _)| host)
}

fn sorted_connections(
    mut connections: Vec<&NetConnection>,
    key: NetworkSortKey,
    direction: SortDirection,
) -> Vec<&NetConnection> {
    connections.sort_by(|left, right| {
        let ordering = compare_connections(left, right, key)
            .then_with(|| left.local_address.cmp(&right.local_address))
            .then_with(|| left.remote_address.cmp(&right.remote_address));
        match direction {
            SortDirection::Ascending => ordering,
            SortDirection::Descending => ordering.reverse(),
        }
    });
    connections
}

fn compare_connections(
    left: &NetConnection,
    right: &NetConnection,
    key: NetworkSortKey,
) -> Ordering {
    match key {
        NetworkSortKey::State => left.state.cmp(&right.state),
        NetworkSortKey::LocalAddress => left.local_address.cmp(&right.local_address),
        NetworkSortKey::RemoteAddress => left.remote_address.cmp(&right.remote_address),
        NetworkSortKey::Pid => left.pid.cmp(&right.pid),
        NetworkSortKey::Process => left.process.cmp(&right.process),
    }
}

pub fn render(ctx: &egui::Context, state: &mut NetworkDetailState) -> Vec<NetworkDetailAction> {
    let mut actions = Vec::new();
    if !state.has_requested {
        if let Some(action) = state.request_refresh() {
            actions.push(action);
        }
    }
    egui::CentralPanel::default()
        .frame(
            egui::Frame::new()
                .fill(BACKGROUND)
                .inner_margin(egui::Margin::same(0)),
        )
        .show(ctx, |ui| {
            render_toolbar(ui, state, &mut actions);
            render_request_state(ui, state);
            render_table(ui, state);
        });
    actions
}

fn render_toolbar(
    ui: &mut egui::Ui,
    state: &mut NetworkDetailState,
    actions: &mut Vec<NetworkDetailAction>,
) {
    egui::Frame::new()
        .fill(PANEL)
        .stroke(egui::Stroke::new(1.0, BORDER))
        .inner_margin(egui::Margin::symmetric(10, 6))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new(format!("网络连接 - {}", state.target.status_text()))
                        .strong()
                        .color(TEXT),
                );

                let interfaces = state
                    .snapshot
                    .as_ref()
                    .map(|snapshot| {
                        snapshot
                            .interfaces()
                            .map(str::to_string)
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default();
                if !interfaces.is_empty() {
                    ui.label(egui::RichText::new("网卡").size(10.0).color(MUTED));
                    let selected = state.selected_interface.as_deref().unwrap_or("全部");
                    egui::ComboBox::from_id_salt("network_detail_interface")
                        .selected_text(selected)
                        .width(130.0)
                        .show_ui(ui, |ui| {
                            ui.selectable_value(&mut state.selected_interface, None, "全部");
                            for interface in interfaces {
                                let label = state
                                    .snapshot
                                    .as_ref()
                                    .and_then(|snapshot| snapshot.primary_address(&interface))
                                    .map(|address| format!("{interface} ({address})"))
                                    .unwrap_or_else(|| interface.clone());
                                ui.selectable_value(
                                    &mut state.selected_interface,
                                    Some(interface),
                                    label,
                                );
                            }
                        });
                }

                match state.selected_rates() {
                    Some(rates) => {
                        ui.label(
                            egui::RichText::new(format!("↑ {}/s", format_rate(rates.tx_rate)))
                                .size(10.0)
                                .color(GREEN),
                        );
                        ui.label(
                            egui::RichText::new(format!("↓ {}/s", format_rate(rates.rx_rate)))
                                .size(10.0)
                                .color(CYAN),
                        );
                    }
                    None => {
                        ui.label(egui::RichText::new("↑ —/s").size(10.0).color(DIM));
                        ui.label(egui::RichText::new("↓ —/s").size(10.0).color(DIM));
                    }
                }

                let count = state.visible_connections().len();
                ui.label(
                    egui::RichText::new(format!("{count} 条连接"))
                        .size(10.0)
                        .color(DIM),
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let refresh = ui.add_enabled(
                        state.pending_request_id.is_none(),
                        egui::Button::new(egui::RichText::new("刷新").size(11.0).color(MUTED))
                            .fill(PANEL_ALT)
                            .stroke(egui::Stroke::new(1.0, BORDER)),
                    );
                    if refresh
                        .on_hover_text("通过当前监控连接立即刷新网络详情")
                        .clicked()
                    {
                        if let Some(action) = state.request_refresh() {
                            actions.push(action);
                        }
                    }
                });
            });
        });
}

fn format_rate(bytes: u64) -> String {
    const KIB: u64 = 1_024;
    const MIB: u64 = 1_048_576;
    const GIB: u64 = 1_073_741_824;

    if bytes >= GIB {
        format!("{:.1}G", bytes as f64 / GIB as f64)
    } else if bytes >= MIB {
        format!("{:.1}M", bytes as f64 / MIB as f64)
    } else if bytes >= KIB {
        format!("{:.1}K", bytes as f64 / KIB as f64)
    } else {
        format!("{bytes}B")
    }
}

fn render_request_state(ui: &mut egui::Ui, state: &NetworkDetailState) {
    match (
        state.snapshot.as_ref(),
        state.pending_request_id,
        state.error.as_deref(),
    ) {
        (None, Some(_), _) => {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(egui::RichText::new("正在加载网络连接…").color(MUTED));
            });
        }
        (None, None, Some(error)) => {
            state_banner(ui, RED, format!("加载网络连接失败：{error}"));
        }
        (Some(_), Some(_), _) => {
            state_banner(ui, CYAN, "正在刷新网络连接…".to_string());
        }
        (Some(_), None, Some(error)) => {
            state_banner(
                ui,
                YELLOW,
                format!("⚠ 当前显示的是上次成功结果，刷新失败：{error}"),
            );
        }
        _ => {}
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

fn render_table(ui: &mut egui::Ui, state: &mut NetworkDetailState) {
    egui::ScrollArea::horizontal()
        .id_salt("network_detail_table")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.set_min_width(850.0);
            render_headers(ui, state);
            let rows = state.visible_connections();
            if state.snapshot.is_some() && rows.is_empty() {
                ui.vertical_centered(|ui| {
                    ui.add_space(32.0);
                    let message = state
                        .selected_interface
                        .as_deref()
                        .map(|interface| format!("{interface} 上没有活跃 TCP 连接"))
                        .unwrap_or_else(|| "没有活跃 TCP 连接".to_string());
                    ui.label(egui::RichText::new(message).color(MUTED));
                });
                return;
            }
            egui::ScrollArea::vertical()
                .id_salt("network_detail_rows")
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 0.0;
                    for connection in rows {
                        render_row(ui, connection);
                    }
                });
        });
}

fn render_headers(ui: &mut egui::Ui, state: &mut NetworkDetailState) {
    egui::Frame::new()
        .fill(PANEL_ALT)
        .stroke(egui::Stroke::new(1.0, BORDER))
        .inner_margin(egui::Margin::symmetric(4, 2))
        .show(ui, |ui| {
            ui.set_min_width(850.0);
            ui.horizontal(|ui| {
                sort_header(ui, state, "状态", NetworkSortKey::State, 70.0);
                sort_header(ui, state, "本地地址", NetworkSortKey::LocalAddress, 230.0);
                sort_header(ui, state, "远端地址", NetworkSortKey::RemoteAddress, 230.0);
                sort_header(ui, state, "PID", NetworkSortKey::Pid, 70.0);
                sort_header(ui, state, "进程", NetworkSortKey::Process, 190.0);
            });
        });
}

fn sort_header(
    ui: &mut egui::Ui,
    state: &mut NetworkDetailState,
    label: &str,
    key: NetworkSortKey,
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

fn render_row(ui: &mut egui::Ui, connection: &NetConnection) {
    let row_height = 22.0;
    let (rect, response) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), row_height),
        egui::Sense::hover(),
    );
    ui.painter().rect_filled(
        rect,
        0.0,
        if response.hovered() {
            PANEL_ALT
        } else {
            BACKGROUND
        },
    );
    let state_color = match connection.state.as_str() {
        "ESTAB" => GREEN,
        "LISTEN" => CYAN,
        _ => MUTED,
    };
    let columns = [
        (70.0, connection.state.clone(), state_color),
        (230.0, connection.local_address.clone(), TEXT),
        (230.0, connection.remote_address.clone(), TEXT),
        (
            70.0,
            connection
                .pid
                .map(|pid| pid.to_string())
                .unwrap_or_else(|| "—".to_string()),
            DIM,
        ),
        (
            190.0,
            if connection.process.is_empty() {
                "—".to_string()
            } else {
                connection.process.clone()
            },
            if connection.process.is_empty() {
                DIM
            } else {
                TEXT
            },
        ),
    ];
    let mut left = rect.left() + 4.0;
    for (width, text, color) in columns {
        let clip =
            egui::Rect::from_min_size(egui::pos2(left, rect.top()), egui::vec2(width, row_height));
        ui.painter().with_clip_rect(clip).text(
            egui::pos2(left + 2.0, rect.center().y),
            egui::Align2::LEFT_CENTER,
            text,
            egui::FontId::monospace(10.0),
            color,
        );
        left += width + ui.spacing().item_spacing.x;
    }
}

#[cfg(test)]
#[path = "network_detail/tests.rs"]
mod tests;
