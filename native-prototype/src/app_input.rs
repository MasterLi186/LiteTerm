use super::*;
use winit::event::MouseScrollDelta;

pub(super) type SelectionPoint = (usize, i32);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ClickState {
    None,
    Single,
    Double,
    Triple,
}

pub(super) fn click_state_after_press(
    click_state: ClickState,
    elapsed_ms: u128,
    same_pos: bool,
) -> ClickState {
    if elapsed_ms < 400 && same_pos {
        match click_state {
            ClickState::Single => ClickState::Double,
            ClickState::Double => ClickState::Triple,
            _ => ClickState::Single,
        }
    } else {
        ClickState::Single
    }
}

pub(super) fn reset_click_sequence_state(
    click_state: &mut ClickState,
    last_click_time: &mut Instant,
    last_click_pos: &mut (usize, usize),
    now: Instant,
) {
    *click_state = ClickState::None;
    *last_click_time = now - std::time::Duration::from_millis(401);
    *last_click_pos = (usize::MAX, usize::MAX);
}

pub(super) fn window_drag_threshold_reached(
    origin: (f64, f64),
    current: (f64, f64),
    threshold: f64,
) -> bool {
    if !origin.0.is_finite()
        || !origin.1.is_finite()
        || !current.0.is_finite()
        || !current.1.is_finite()
        || !threshold.is_finite()
        || threshold < 0.0
    {
        return false;
    }
    let dx = current.0 - origin.0;
    let dy = current.1 - origin.1;
    dx * dx + dy * dy >= threshold * threshold
}

pub(super) fn point_in_terminal_bounds(
    x: f32,
    y: f32,
    left: f32,
    top: f32,
    right: f32,
    bottom: f32,
) -> bool {
    x >= left && x < right && y >= top && y < bottom
}

pub(super) fn terminal_grid_bounds(
    left: f32,
    top: f32,
    layout_bottom: f32,
    cell_width: f32,
    cell_height: f32,
    cols: u16,
    rows: u16,
) -> (f32, f32) {
    let right = left + cell_width * f32::from(cols);
    // Renderer::calculate_grid_size reserves one logical row as a glyph safety margin.
    // The live cursor can still occupy it, so pointer hit-testing includes that row.
    let interactive_rows = rows.saturating_add(1);
    let grid_bottom = top + cell_height * f32::from(interactive_rows);
    (right, grid_bottom.min(layout_bottom))
}

pub(super) fn drag_selection_range(
    anchor: Option<SelectionPoint>,
    current: SelectionPoint,
) -> Option<(SelectionPoint, SelectionPoint)> {
    anchor
        .filter(|start| *start != current)
        .map(|start| (start, current))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum LeftMouseGesture {
    LocalSelection,
    TerminalReport { last_cell: (usize, usize) },
}

pub(super) fn left_mouse_gesture(
    mouse_mode: bool,
    shift: bool,
    cell: (usize, usize),
) -> LeftMouseGesture {
    if mouse_mode && !shift {
        LeftMouseGesture::TerminalReport { last_cell: cell }
    } else {
        LeftMouseGesture::LocalSelection
    }
}

pub(super) fn terminal_report_release_cell(
    gesture: Option<LeftMouseGesture>,
    current_cell: Option<(usize, usize)>,
) -> Option<(usize, usize)> {
    match gesture {
        Some(LeftMouseGesture::TerminalReport { last_cell }) => current_cell.or(Some(last_cell)),
        _ => None,
    }
}

pub(super) fn should_copy_left_selection(
    gesture: Option<LeftMouseGesture>,
    selection_start: Option<SelectionPoint>,
    selection_end: Option<SelectionPoint>,
) -> bool {
    matches!(gesture, Some(LeftMouseGesture::LocalSelection))
        && selection_start.is_some()
        && selection_end.is_some()
}

pub(super) fn should_process_left_release(in_terminal: bool, has_active_gesture: bool) -> bool {
    in_terminal || has_active_gesture
}

pub(super) fn should_pass_consumed_mouse_input(
    _in_terminal: bool,
    has_active_left_release: bool,
) -> bool {
    has_active_left_release
}

pub(super) fn should_clear_selection_on_focus_loss(gesture: Option<LeftMouseGesture>) -> bool {
    matches!(gesture, Some(LeftMouseGesture::LocalSelection))
}

pub(super) fn clear_selection_state(
    selection_start: &mut Option<SelectionPoint>,
    selection_end: &mut Option<SelectionPoint>,
    selection_drag_anchor: &mut Option<SelectionPoint>,
) {
    *selection_start = None;
    *selection_end = None;
    *selection_drag_anchor = None;
}

pub(super) fn take_left_mouse_gesture_state(
    gesture: &mut Option<LeftMouseGesture>,
    selection_drag_anchor: &mut Option<SelectionPoint>,
) -> Option<(usize, usize)> {
    let gesture = gesture.take();
    *selection_drag_anchor = None;
    terminal_report_release_cell(gesture, None)
}

pub(super) fn prepare_for_active_tab_change_state(
    gesture: &mut Option<LeftMouseGesture>,
    selection_start: &mut Option<SelectionPoint>,
    selection_end: &mut Option<SelectionPoint>,
    selection_drag_anchor: &mut Option<SelectionPoint>,
    click_sequence: (&mut ClickState, &mut Instant, &mut (usize, usize)),
    now: Instant,
) -> Option<(usize, usize)> {
    let release_cell = take_left_mouse_gesture_state(gesture, selection_drag_anchor);
    clear_selection_state(selection_start, selection_end, selection_drag_anchor);
    let (click_state, last_click_time, last_click_pos) = click_sequence;
    reset_click_sequence_state(click_state, last_click_time, last_click_pos, now);
    release_cell
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum KeyboardInputRoute {
    Drop,
    EguiOnly,
    App,
}

pub(super) fn keyboard_input_route(state: ElementState, is_synthetic: bool) -> KeyboardInputRoute {
    match (state, is_synthetic) {
        (ElementState::Pressed, true) => KeyboardInputRoute::Drop,
        (ElementState::Pressed, false) => KeyboardInputRoute::App,
        (ElementState::Released, _) => KeyboardInputRoute::EguiOnly,
    }
}

pub(super) fn is_modifier_only_key(key: &Key) -> bool {
    matches!(
        key,
        Key::Named(
            NamedKey::Shift
                | NamedKey::Control
                | NamedKey::Alt
                | NamedKey::Super
                | NamedKey::AltGraph
        )
    )
}

pub(super) fn terminal_backspace_sequence(alt: bool) -> &'static str {
    if alt {
        "\x1b\x7f"
    } else {
        "\x7f"
    }
}

pub(super) fn is_primary_find_shortcut(
    key: &Key,
    modifiers: winit::keyboard::ModifiersState,
) -> bool {
    let command_modifier = modifiers.control_key() || modifiers.super_key();
    command_modifier
        && !modifiers.alt_key()
        && !modifiers.shift_key()
        && matches!(key, Key::Character(character) if character.eq_ignore_ascii_case("f"))
}

pub(super) fn terminal_input_blocked(
    settings_panel_visible: bool,
    has_sidebar_dialog: bool,
) -> bool {
    settings_panel_visible || has_sidebar_dialog
}

pub(super) fn blocking_dialog_visible(
    has_sidebar_dialog: bool,
    new_tab_selector_open: bool,
) -> bool {
    has_sidebar_dialog || new_tab_selector_open
}

pub(super) fn should_pass_keyboard_to_terminal(
    settings_panel_visible: bool,
    has_sidebar_dialog: bool,
    egui_wants_keyboard: bool,
    search_field_owns_focus: bool,
) -> bool {
    !terminal_input_blocked(settings_panel_visible, has_sidebar_dialog)
        && !egui_wants_keyboard
        && !search_field_owns_focus
}

pub(super) fn terminal_search_query_id(tab_id: &str) -> egui::Id {
    egui::Id::new(("terminal_search_query", tab_id))
}

pub(super) fn search_keyboard_fallback_action(
    key: &Key,
    shift: bool,
    search_owns_input: bool,
    has_dialog: bool,
) -> Option<terminal_search::SearchBarKeyAction> {
    if !search_owns_input || has_dialog {
        return None;
    }
    match key {
        Key::Named(NamedKey::Enter) => Some(terminal_search::search_bar_key_action(
            terminal_search::SearchBarKey::Enter,
            shift,
        )),
        Key::Named(NamedKey::Escape) => Some(terminal_search::search_bar_key_action(
            terminal_search::SearchBarKey::Escape,
            false,
        )),
        _ => None,
    }
}

/// Only forward pointer events while no blocking UI is open and the pointer is in the terminal.
pub(super) fn should_pass_pointer_to_terminal(
    settings_panel_visible: bool,
    has_sidebar_dialog: bool,
    in_terminal: bool,
) -> bool {
    !terminal_input_blocked(settings_panel_visible, has_sidebar_dialog) && in_terminal
}

const TERMINAL_WHEEL_LINES_PER_NOTCH: f32 = 7.0;
const TERMINAL_SCROLL_PIXELS_PER_LINE: f64 = 18.0;
pub(super) const UI_WHEEL_POINTS_PER_LINE: f32 = 80.0;
const MAX_DIRECT_UI_WHEEL_EVENT_POINTS: f32 = 7.0;

#[derive(Debug, Default)]
pub(super) struct TerminalWheelAccumulator {
    pixel_remainder: f64,
}

impl TerminalWheelAccumulator {
    pub(super) fn reset(&mut self) {
        self.pixel_remainder = 0.0;
    }

    pub(super) fn scroll_lines(&mut self, delta: &MouseScrollDelta) -> i32 {
        match delta {
            MouseScrollDelta::LineDelta(_, y) => {
                self.reset();
                (*y * TERMINAL_WHEEL_LINES_PER_NOTCH).round() as i32
            }
            MouseScrollDelta::PixelDelta(position) => {
                let contribution = position.y / TERMINAL_SCROLL_PIXELS_PER_LINE;
                if self.pixel_remainder != 0.0
                    && contribution != 0.0
                    && self.pixel_remainder.signum() != contribution.signum()
                {
                    // A direction reversal must feel immediate instead of first paying off
                    // a fractional remainder accumulated in the opposite direction.
                    self.reset();
                }
                let total = self.pixel_remainder + contribution;
                let lines = total.trunc() as i32;
                self.pixel_remainder = total - f64::from(lines);
                lines
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum TerminalPointerMotionAction {
    Process,
    BlockAndCancelGesture,
}

pub(super) fn terminal_pointer_motion_action(
    settings_panel_visible: bool,
    has_blocking_dialog: bool,
) -> TerminalPointerMotionAction {
    if terminal_input_blocked(settings_panel_visible, has_blocking_dialog) {
        TerminalPointerMotionAction::BlockAndCancelGesture
    } else {
        TerminalPointerMotionAction::Process
    }
}

pub(super) fn terminal_link_modifier_active(modifiers: winit::keyboard::ModifiersState) -> bool {
    if cfg!(target_os = "macos") {
        modifiers.super_key()
    } else {
        modifiers.control_key()
    }
}

/// Physical IME cursor rectangle for `Window::set_ime_cursor_area`.
/// Width/height are always > 0 after `logical_to_physical_ime_cursor_area`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct PhysicalImeCursorArea {
    pub(super) x: f64,
    pub(super) y: f64,
    pub(super) width: f64,
    pub(super) height: f64,
}

/// Resolve who owns text input / IME composition.
///
/// Any blocker → `Egui`. All false → `Terminal`.
/// Completion popup alone is intentionally **not** a parameter and must not
/// force Egui ownership.
pub(super) fn resolve_ime_input_owner(
    settings_panel_visible: bool,
    has_sidebar_dialog: bool,
    blocking_egui_overlay: bool,
    search_owns_keyboard: bool,
    egui_wants_keyboard: bool,
) -> ime::InputOwner {
    if settings_panel_visible
        || has_sidebar_dialog
        || blocking_egui_overlay
        || search_owns_keyboard
        || egui_wants_keyboard
    {
        ime::InputOwner::Egui
    } else {
        ime::InputOwner::Terminal
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum RoutedImeEvent {
    Enabled,
    Disabled,
    Preedit(String, Option<(usize, usize)>),
    Commit(String),
}

pub(super) fn apply_routed_ime_event(
    ime: &mut ime::ImeState,
    owner: ime::InputOwner,
    event: RoutedImeEvent,
) -> ime::ImeAction {
    match event {
        RoutedImeEvent::Enabled => ime.on_enabled(),
        RoutedImeEvent::Disabled => ime.on_disabled(),
        RoutedImeEvent::Preedit(text, cursor) => {
            if owner == ime::InputOwner::Terminal {
                ime.preedit(text, cursor)
            } else {
                ime::ImeAction::None
            }
        }
        RoutedImeEvent::Commit(text) => ime.commit(text, owner),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ImeOwnedInputKind {
    Keyboard,
    ModifiersChanged,
    Ime,
    Other,
}

pub(super) fn terminal_preedit_blocks_input(
    owner: ime::InputOwner,
    has_active_preedit: bool,
    kind: ImeOwnedInputKind,
) -> bool {
    owner == ime::InputOwner::Terminal && has_active_preedit && kind == ImeOwnedInputKind::Keyboard
}

/// Convert a logical cursor rect to a physical IME cursor area.
///
/// Invalid / non-positive `pixels_per_point` is treated as `1.0`.
/// Physical width/height are always forced to be > 0.
pub(super) fn logical_to_physical_ime_cursor_area(
    min_x: f32,
    min_y: f32,
    width: f32,
    height: f32,
    pixels_per_point: f32,
) -> PhysicalImeCursorArea {
    let scale = if pixels_per_point.is_finite() && pixels_per_point > 0.0 {
        f64::from(pixels_per_point)
    } else {
        1.0
    };
    let mut phys_w = f64::from(width) * scale;
    let mut phys_h = f64::from(height) * scale;
    if phys_w <= 0.0 {
        phys_w = 1.0;
    }
    if phys_h <= 0.0 {
        phys_h = 1.0;
    }
    PhysicalImeCursorArea {
        x: f64::from(min_x) * scale,
        y: f64::from(min_y) * scale,
        width: phys_w,
        height: phys_h,
    }
}

/// Whether terminal IME may reassert `set_ime_allowed` / cursor area after egui
/// platform output. Never override when Egui owns input or the window is unfocused.
pub(super) fn should_reassert_terminal_ime(owner: ime::InputOwner, window_focused: bool) -> bool {
    matches!(owner, ime::InputOwner::Terminal) && window_focused
}

pub(super) fn completion_blocking_egui_overlay_visible(ctx: &egui::Context) -> bool {
    const FOREGROUND_IDS: &[&str] = &[
        "command_history_backdrop",
        "file_rename_backdrop",
        "file_create_backdrop",
        "file_delete_backdrop",
        "tab_rename_backdrop",
    ];
    const MIDDLE_IDS: &[&str] = &["命令收藏", "添加快捷命令", "编辑快捷命令"];
    ctx.memory(|memory| {
        FOREGROUND_IDS.iter().any(|id| {
            memory.areas().is_visible(&egui::LayerId::new(
                egui::Order::Foreground,
                egui::Id::new(*id),
            ))
        }) || MIDDLE_IDS.iter().any(|id| {
            memory
                .areas()
                .is_visible(&egui::LayerId::new(egui::Order::Middle, egui::Id::new(*id)))
        })
    })
}

pub(super) fn completion_popup_may_render(
    ctx: &egui::Context,
    has_cached_snapshot: bool,
    sidebar_modal_visible: bool,
) -> bool {
    has_cached_snapshot && !sidebar_modal_visible && !completion_blocking_egui_overlay_visible(ctx)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum FrameActionTiming {
    NoAction,
    AfterPresent { request_redraw: bool },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum InputFrameScheduling {
    RenderNow,
    RequestRedraw,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum WindowGeometryEventKind {
    Resized,
    ScaleFactorChanged,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum WindowGeometryUpdatePlan {
    ResizeSurfaceAndSyncAndRender,
    SyncAndRender,
}

pub(super) fn plan_window_geometry_update(
    kind: WindowGeometryEventKind,
) -> WindowGeometryUpdatePlan {
    match kind {
        WindowGeometryEventKind::Resized => WindowGeometryUpdatePlan::ResizeSurfaceAndSyncAndRender,
        WindowGeometryEventKind::ScaleFactorChanged => WindowGeometryUpdatePlan::SyncAndRender,
    }
}

pub(super) fn terminal_split_menu_labels(
    split_actions_enabled: bool,
) -> (&'static str, &'static str) {
    if split_actions_enabled {
        ("水平分屏", "垂直分屏")
    } else {
        ("水平分屏（串口不支持）", "垂直分屏（串口不支持）")
    }
}

pub(super) fn selector_keyboard_input_scheduling(
    selector_open: bool,
    keyboard_route: KeyboardInputRoute,
    _key: &Key,
) -> Option<InputFrameScheduling> {
    (selector_open && keyboard_route == KeyboardInputRoute::App)
        .then_some(InputFrameScheduling::RequestRedraw)
}

pub(super) fn rename_keyboard_input_scheduling(
    rename_open: bool,
    keyboard_route: KeyboardInputRoute,
    _key: &Key,
) -> Option<InputFrameScheduling> {
    (rename_open && keyboard_route == KeyboardInputRoute::App)
        .then_some(InputFrameScheduling::RequestRedraw)
}

pub(super) fn validate_zmodem_drop_paths(paths: &[std::path::PathBuf]) -> Result<(), String> {
    if paths.is_empty() {
        return Err("没有可上传的文件".into());
    }
    for path in paths {
        let metadata = std::fs::symlink_metadata(path)
            .map_err(|error| format!("无法读取拖入路径 {}：{error}", path.display()))?;
        if metadata.file_type().is_symlink() {
            return Err(format!("ZMODEM 不支持符号链接：{}", path.display()));
        }
        if !metadata.is_file() {
            return Err(format!("ZMODEM 仅支持普通文件：{}", path.display()));
        }
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct TerminalContextMenuTransition {
    pub(super) visible: bool,
    pub(super) position: egui::Pos2,
    pub(super) ignore_pointer_press_once: bool,
    pub(super) frame_action: InputFrameScheduling,
}

pub(super) fn consumed_event_frame_scheduling(
    is_right_mouse_input: bool,
    is_mouse_wheel: bool,
    is_pointer_motion: bool,
) -> InputFrameScheduling {
    if is_right_mouse_input || is_mouse_wheel || is_pointer_motion {
        InputFrameScheduling::RequestRedraw
    } else {
        InputFrameScheduling::RenderNow
    }
}

pub(super) fn normalize_ui_wheel_events(events: &mut Vec<egui::Event>) {
    let mut normalized = Vec::with_capacity(events.len());
    for event in events.drain(..) {
        let egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Line,
            delta,
            modifiers,
        } = event
        else {
            normalized.push(event);
            continue;
        };
        let points = delta * UI_WHEEL_POINTS_PER_LINE;
        let parts = (points.abs().max_elem() / MAX_DIRECT_UI_WHEEL_EVENT_POINTS)
            .ceil()
            .max(1.0) as usize;
        let part = points / parts as f32;
        normalized.extend((0..parts).map(|_| egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            delta: part,
            modifiers,
        }));
    }
    *events = normalized;
}

pub(super) fn sanitize_pixels_per_point(pixels_per_point: f32) -> f32 {
    if pixels_per_point.is_finite() && pixels_per_point > 0.0 {
        pixels_per_point
    } else {
        1.0
    }
}

pub(super) fn physical_to_egui_position(
    physical_position: (f64, f64),
    pixels_per_point: f32,
) -> egui::Pos2 {
    let pixels_per_point = sanitize_pixels_per_point(pixels_per_point);
    egui::pos2(
        physical_position.0 as f32 / pixels_per_point,
        physical_position.1 as f32 / pixels_per_point,
    )
}

pub(super) fn logical_to_physical_pane_rect(
    logical: egui::Rect,
    pixels_per_point: f32,
) -> PaneRenderRect {
    let scale = sanitize_pixels_per_point(pixels_per_point);
    PaneRenderRect::new(
        logical.left() * scale,
        logical.top() * scale,
        logical.width() * scale,
        logical.height() * scale,
    )
}

pub(super) fn physical_to_logical_rect(physical: egui::Rect, pixels_per_point: f32) -> egui::Rect {
    let scale = sanitize_pixels_per_point(pixels_per_point);
    egui::Rect::from_min_max(physical.min / scale, physical.max / scale)
}

pub(super) fn logical_terminal_layout_rect(
    surface_width: u32,
    surface_height: u32,
    pixels_per_point: f32,
    sidebar_width: f32,
    tab_bar_height: f32,
    command_bar_height: f32,
    browser_height: f32,
) -> egui::Rect {
    let scale = sanitize_pixels_per_point(pixels_per_point);
    let logical_width = surface_width as f32 / scale;
    let logical_height = surface_height as f32 / scale;
    egui::Rect::from_min_size(
        egui::pos2(sidebar_width, tab_bar_height),
        egui::vec2(
            (logical_width - sidebar_width).max(1.0),
            (logical_height - tab_bar_height - command_bar_height - browser_height).max(1.0),
        ),
    )
}

pub(super) fn terminal_context_menu_press_transition(
    physical_position: (f64, f64),
    pixels_per_point: f32,
) -> TerminalContextMenuTransition {
    TerminalContextMenuTransition {
        visible: true,
        position: physical_to_egui_position(physical_position, pixels_per_point),
        ignore_pointer_press_once: true,
        frame_action: InputFrameScheduling::RequestRedraw,
    }
}

pub(super) fn apply_terminal_context_menu_transition(
    visible: &mut bool,
    position: &mut egui::Pos2,
    ignore_pointer_press_once: &mut bool,
    transition: TerminalContextMenuTransition,
    request_redraw: impl FnOnce(),
) {
    *visible = transition.visible;
    *position = transition.position;
    *ignore_pointer_press_once = transition.ignore_pointer_press_once;
    match transition.frame_action {
        InputFrameScheduling::RequestRedraw => request_redraw(),
        InputFrameScheduling::RenderNow => {
            panic!("右键菜单事件禁止同步渲染或 present")
        }
    }
}

pub(super) fn should_close_terminal_context_menu(
    action_requested_close: bool,
    pointer_pressed: bool,
    pointer_position: Option<egui::Pos2>,
    actual_menu_rect: Option<egui::Rect>,
    ignore_pointer_press_once: bool,
) -> bool {
    if action_requested_close {
        return true;
    }
    if ignore_pointer_press_once || !pointer_pressed {
        return false;
    }
    pointer_position
        .zip(actual_menu_rect)
        .is_some_and(|(position, menu_rect)| !menu_rect.contains(position))
}

pub(super) fn open_terminal_menu_mouse_press_gate(
    menu_visible: bool,
    state: ElementState,
    button: MouseButton,
    _mouse_mode: bool,
) -> Option<InputFrameScheduling> {
    (menu_visible && state == ElementState::Pressed && button != MouseButton::Right)
        .then_some(InputFrameScheduling::RequestRedraw)
}

pub(super) fn tab_action_frame_timing(action: &tab_bar::TabBarAction) -> FrameActionTiming {
    match action {
        tab_bar::TabBarAction::None => FrameActionTiming::NoAction,
        tab_bar::TabBarAction::SwitchTo(_)
        | tab_bar::TabBarAction::NewTab
        | tab_bar::TabBarAction::Rename(_) => FrameActionTiming::AfterPresent {
            request_redraw: true,
        },
        _ => FrameActionTiming::AfterPresent {
            request_redraw: false,
        },
    }
}

pub(super) fn refresh_side_for_event(event: &sftp::SftpEvent) -> Option<sftp::FileSide> {
    match event {
        sftp::SftpEvent::Ready { .. } => Some(sftp::FileSide::Remote),
        sftp::SftpEvent::TransferFinished {
            direction: sftp::TransferDirection::Upload,
            result: Ok(()),
            ..
        } => Some(sftp::FileSide::Remote),
        sftp::SftpEvent::TransferFinished {
            direction: sftp::TransferDirection::Download,
            result: Ok(()),
            ..
        } => Some(sftp::FileSide::Local),
        sftp::SftpEvent::MutationFinished {
            side,
            result: Ok(()),
            ..
        } => Some(*side),
        _ => None,
    }
}

pub(super) fn bounded_notice(prefix: &str, detail: &str) -> String {
    let detail = detail
        .chars()
        .filter(|character| !character.is_control())
        .take(200)
        .collect::<String>();
    format!("{prefix}{detail}")
}

pub(super) fn retire_drag_upload_transfer(
    transfer_ids: &mut HashSet<String>,
    event: &sftp::SftpEvent,
) -> bool {
    match event {
        sftp::SftpEvent::TransferFinished { transfer_id, .. } => transfer_ids.remove(transfer_id),
        _ => false,
    }
}

#[cfg(test)]
mod drag_upload_event_tests {
    use super::*;

    #[test]
    fn transfer_tracking_is_retired_before_worker_identity_filtering() {
        let mut tracked = HashSet::from(["drag-id".to_string()]);
        let stale_event = sftp::SftpEvent::TransferFinished {
            tab_id: "closed-tab".into(),
            transfer_id: "drag-id".into(),
            direction: sftp::TransferDirection::Upload,
            result: Err("worker stopped".into()),
        };

        assert!(retire_drag_upload_transfer(&mut tracked, &stale_event));
        assert!(tracked.is_empty());
        assert!(!retire_drag_upload_transfer(&mut tracked, &stale_event));
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct TerminalImeIdentity {
    pub(super) tab_id: String,
    pub(super) pane_id: PaneId,
}

pub(super) fn terminal_ime_commit_matches(
    composition_owner: Option<&TerminalImeIdentity>,
    current_owner: Option<&TerminalImeIdentity>,
) -> bool {
    match (composition_owner, current_owner) {
        // Some XIM implementations (notably Fcitx 4) can commit text without
        // first emitting a non-empty Preedit event. Accept that direct commit
        // when a terminal is still active.
        (None, Some(_)) => true,
        // A composed commit must stay bound to the tab and pane where its
        // preedit started, otherwise a delayed platform event could leak into
        // the newly selected terminal.
        (Some(composition), Some(current)) => composition == current,
        (_, None) => false,
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct DraggedSplit {
    pub(super) tab_id: String,
    pub(super) split_id: SplitId,
}

pub(super) fn active_dragged_split(
    dragged: Option<&DraggedSplit>,
    active_tab_id: Option<&str>,
) -> Option<SplitId> {
    dragged
        .filter(|dragged| Some(dragged.tab_id.as_str()) == active_tab_id)
        .map(|dragged| dragged.split_id)
}
