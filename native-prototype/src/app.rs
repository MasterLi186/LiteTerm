use super::*;

pub(super) mod api_ops;
mod events;
mod layout;
mod pointer_motion;
mod render;
mod render_actions;
mod render_overlays;
mod runtime;
mod session;
mod tabs;
mod terminal_interaction;
mod ui_state;
mod user_events;
mod window_events;

pub(crate) use api_ops::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SerialTerminalMenuState {
    Connecting,
    Connected,
    Disconnected,
}

pub(super) struct App {
    window: Option<Arc<Window>>,
    gpu: Option<GpuState>,
    renderer: Option<Renderer>,
    tab_manager: TabManager,
    sftp_workers: HashMap<String, sftp::SftpHandle>,
    file_browsers: HashMap<String, file_browser::FileBrowserState>,
    process_managers: HashMap<String, process_manager::ProcessManagerState>,
    network_details: HashMap<String, network_detail::NetworkDetailState>,
    remote_monitors: HashMap<monitor::MonitorKey, remote_monitor::RemoteMonitorHandle>,
    remote_monitor_params: HashMap<monitor::MonitorKey, ssh::ConnectionParams>,
    remote_monitor_generations: HashMap<monitor::MonitorKey, u64>,
    monitor_slots: HashMap<monitor::MonitorKey, monitor::MonitorSlot>,
    local_monitor_refresh: Option<mpsc::SyncSender<()>>,
    next_remote_monitor_generation: u64,
    completion_request_id: u64,
    completion_popup_snapshot: Option<completion_popup::CompletionPopupSnapshot>,
    completion_popup_epoch: u64,
    zmodem_controls: HashMap<String, ZmodemControlSlot>,
    zmodem_views: HashMap<String, zmodem::ui::PaneZmodemView>,
    zmodem_settings_source: zmodem::runtime::RuntimeSettingsSource,
    next_zmodem_transfer_id: Option<u64>,
    adb_history_writer: adb_history::AdbHistoryWriter,
    cursor_visible: bool,
    last_render_time: Instant,
    cursor_timer: Instant,
    startup_time: Instant,
    proxy: EventLoopProxy<UserEvent>,
    modifiers: Modifiers,
    // egui
    egui_ctx: egui::Context,
    egui_state: Option<egui_winit::State>,
    egui_renderer: Option<egui_wgpu::Renderer>,
    sidebar: Sidebar,
    sidebar_width: f32,
    tab_bar_height: f32,
    command_bar: command_bar::CommandBar,
    command_bar_height: f32,
    // Mouse state
    left_mouse_gesture: Option<LeftMouseGesture>,
    left_mouse_pane_id: Option<PaneId>,
    dragged_split: Option<DraggedSplit>,
    drag_upload: drag_upload::DragUploadState,
    drag_upload_transfer_ids: HashSet<String>,
    pane_layout: LayoutSnapshot,
    /// Last winit cursor position in physical surface pixels.
    mouse_position: (f64, f64),
    terminal_wheel_accumulator: TerminalWheelAccumulator,
    terminal_wheel_pane_id: Option<PaneId>,
    selection_auto_scroll_lines: i32,
    selection_auto_scroll_at: Instant,
    pending_terminal_link: Option<(String, terminal_links::TerminalLink)>,
    clipboard: Option<arboard::Clipboard>,
    // Click detection
    last_click_time: Instant,
    last_click_pos: (usize, usize),
    click_state: ClickState,
    // 终端右键菜单
    show_terminal_menu: bool,
    terminal_menu_pos: egui::Pos2,
    terminal_menu_ignore_pointer_press_once: bool,
    // 窗口焦点
    window_focused: bool,
    // 设置
    settings: settings::Settings,
    settings_panel: settings_panel::SettingsPanel,
    new_tab_selector: new_tab_selector::NewTabSelector,
    tab_rename_dialog: tab_bar::TabRenameDialog,
    tab_drag_state: tab_bar::TabDragState,
    pending_window_drag_origin: Option<(f64, f64)>,
    pending_workspace_session: Option<workspace_session::WorkspaceSession>,
    workspace_session_saved: bool,
    batch_dialog: batch_command::BatchCommandDialog,
    tunnel_registry: tunnel::TunnelRegistry,
    tunnel_manager: tunnel_manager::TunnelManager,
    recordings: recording::RecordingRegistry,
    terminal_logs: terminal_log::TerminalLogRegistry,
    recording_dialog: recording::RecordingDialog,
    recording_playbacks: HashMap<String, recording::PlaybackState>,
    settings_load_warning: Option<String>,
    /// Request focus on the search query TextEdit next frame.
    search_request_focus: bool,
    /// Defense-in-depth: search TextEdit owns keyboard (suppresses PTY keys).
    search_field_owns_focus: bool,
    /// Native IME composition / duplicate-suppression state (pure machine in `ime`).
    ime: ime::ImeState,
    /// Terminal pane that owned the currently active preedit.
    ime_terminal_owner: Option<TerminalImeIdentity>,
    terminal_notice: Option<String>,
    api_outputs: api::OutputRegistry,
    api_streams: HashMap<String, (String, u64)>,
    api_ephemeral_tabs: HashSet<String>,
    api_server: Option<HttpApiServer>,
    /// Custom title-bar close button requests a normal event-loop shutdown.
    exit_requested: bool,
}

impl App {
    pub(super) fn new(proxy: EventLoopProxy<UserEvent>) -> Self {
        let (settings, settings_load_warning) = match settings::Settings::load() {
            Ok(mut settings) => {
                let warnings = settings.sanitize_loaded();
                let warning = settings::format_settings_load_warnings(&warnings);
                let warning = if warning.is_empty() {
                    None
                } else {
                    log::warn!("[SETTINGS] {warning}");
                    Some(warning)
                };
                (settings, warning)
            }
            Err(e) => {
                let warning = format!("加载设置失败，已使用默认值：{e}");
                log::warn!("[SETTINGS] {warning}");
                (settings::Settings::default(), Some(warning))
            }
        };
        let initial_zmodem_settings = zmodem_runtime_settings(&settings).unwrap_or_else(|_| {
            zmodem::runtime::RuntimeSettings {
                enabled: false,
                auto_detect: settings.zmodem.auto_detect,
                receive_directory: std::env::temp_dir(),
                transfer_timeout: Some(Duration::from_secs(settings.zmodem.timeout_secs.into())),
            }
        });
        let zmodem_settings_source =
            zmodem::runtime::RuntimeSettingsSource::new(initial_zmodem_settings);
        let api_outputs = api::OutputRegistry::new();
        let (api_server, api_start_warning) = match HttpApiServer::start(
            api::ApiServerConfig::default(),
            proxy.clone(),
            api_outputs.clone(),
        ) {
            Ok(server) => (Some(server), None),
            Err(error) => {
                log::warn!("{error}");
                (
                    None,
                    Some("HTTP API 未能启动，图形终端仍可正常使用".to_string()),
                )
            }
        };
        let mut sidebar = Sidebar::new();
        sidebar.width = settings.appearance.sidebar_width as f32;
        sidebar.visible = settings.appearance.show_sidebar;
        let sidebar_width = if sidebar.visible { sidebar.width } else { 0.0 };
        let pending_workspace_session = match workspace_session::WorkspaceSession::load() {
            Ok(session) => Some(session),
            Err(error) => {
                log::warn!("加载上次标签页会话失败：{error}");
                None
            }
        };
        Self {
            window: None,
            gpu: None,
            renderer: None,
            tab_manager: TabManager::new(),
            sftp_workers: HashMap::new(),
            file_browsers: HashMap::new(),
            process_managers: HashMap::new(),
            network_details: HashMap::new(),
            remote_monitors: HashMap::new(),
            remote_monitor_params: HashMap::new(),
            remote_monitor_generations: HashMap::new(),
            monitor_slots: HashMap::new(),
            local_monitor_refresh: None,
            next_remote_monitor_generation: 0,
            completion_request_id: 0,
            completion_popup_snapshot: None,
            completion_popup_epoch: 0,
            zmodem_controls: HashMap::new(),
            zmodem_views: HashMap::new(),
            zmodem_settings_source,
            next_zmodem_transfer_id: Some(1),
            adb_history_writer: adb_history::AdbHistoryWriter::start(),
            cursor_visible: true,
            cursor_timer: Instant::now(),
            last_render_time: Instant::now(),
            startup_time: Instant::now(),
            proxy,
            modifiers: Modifiers::default(),
            egui_ctx: {
                let ctx = egui::Context::default();
                let mut fonts = egui::FontDefinitions::default();
                if let Ok(font_data) =
                    std::fs::read("/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc")
                {
                    fonts.font_data.insert(
                        "noto_cjk".to_owned(),
                        egui::FontData::from_owned(font_data).into(),
                    );
                    fonts
                        .families
                        .entry(egui::FontFamily::Proportional)
                        .or_default()
                        .push("noto_cjk".to_owned());
                    fonts
                        .families
                        .entry(egui::FontFamily::Monospace)
                        .or_default()
                        .push("noto_cjk".to_owned());
                }
                ctx.set_fonts(fonts);
                ctx
            },
            egui_state: None,
            egui_renderer: None,
            sidebar,
            sidebar_width,
            tab_bar_height: tab_bar::TAB_BAR_HEIGHT,
            command_bar: command_bar::CommandBar::new(),
            command_bar_height: command_bar::COMMAND_BAR_HEIGHT,
            left_mouse_gesture: None,
            left_mouse_pane_id: None,
            dragged_split: None,
            drag_upload: drag_upload::DragUploadState::default(),
            drag_upload_transfer_ids: HashSet::new(),
            pane_layout: LayoutSnapshot::default(),
            mouse_position: (0.0, 0.0),
            terminal_wheel_accumulator: TerminalWheelAccumulator::default(),
            terminal_wheel_pane_id: None,
            selection_auto_scroll_lines: 0,
            selection_auto_scroll_at: Instant::now(),
            pending_terminal_link: None,
            clipboard: arboard::Clipboard::new().ok(),
            last_click_time: Instant::now() - std::time::Duration::from_secs(10),
            last_click_pos: (0, 0),
            click_state: ClickState::None,
            show_terminal_menu: false,
            terminal_menu_pos: egui::Pos2::ZERO,
            terminal_menu_ignore_pointer_press_once: false,
            window_focused: true,
            settings,
            settings_panel: settings_panel::SettingsPanel::default(),
            new_tab_selector: new_tab_selector::NewTabSelector::new(),
            tab_rename_dialog: tab_bar::TabRenameDialog::default(),
            tab_drag_state: tab_bar::TabDragState::default(),
            pending_window_drag_origin: None,
            pending_workspace_session,
            workspace_session_saved: false,
            batch_dialog: batch_command::BatchCommandDialog::default(),
            tunnel_registry: tunnel::TunnelRegistry::new(),
            tunnel_manager: tunnel_manager::TunnelManager::new(),
            recordings: recording::RecordingRegistry::default(),
            terminal_logs: terminal_log::TerminalLogRegistry::default(),
            recording_dialog: recording::RecordingDialog::default(),
            recording_playbacks: HashMap::new(),
            settings_load_warning,
            search_request_focus: false,
            search_field_owns_focus: false,
            ime: ime::ImeState::default(),
            ime_terminal_owner: None,
            terminal_notice: api_start_warning,
            api_outputs,
            api_streams: HashMap::new(),
            api_ephemeral_tabs: HashSet::new(),
            api_server,
            exit_requested: false,
        }
    }
}
