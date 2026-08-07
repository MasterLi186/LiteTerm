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

pub(super) fn selection_kind_for_press(
    click_state: ClickState,
    block_modifier: bool,
) -> terminal::TerminalSelectionKind {
    if block_modifier {
        return terminal::TerminalSelectionKind::Block;
    }
    match click_state {
        ClickState::Double => terminal::TerminalSelectionKind::Semantic,
        ClickState::Triple => terminal::TerminalSelectionKind::Lines,
        _ => terminal::TerminalSelectionKind::Simple,
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

const WINDOW_RESIZE_BORDER_LOGICAL: f64 = 8.0;
const MIN_WINDOW_WIDTH_LOGICAL: f64 = 640.0;
const MIN_WINDOW_HEIGHT_LOGICAL: f64 = 400.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum WindowResizeInteraction {
    /// The platform owns the pointer grab after `Window::drag_resize_window`.
    System,
    /// Fallback used on platforms where winit cannot start a native resize drag.
    Manual(WindowResizeDrag),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct WindowResizeDrag {
    pub(super) direction: winit::window::ResizeDirection,
    pub(super) start_cursor: (f64, f64),
    pub(super) start_outer_position: (i32, i32),
    pub(super) start_inner_size: (u32, u32),
    pub(super) min_inner_size: (u32, u32),
}

impl WindowResizeDrag {
    pub(super) fn geometry_for_cursor(self, cursor: (f64, f64)) -> ((i32, i32), (u32, u32)) {
        let dx = cursor.0 - self.start_cursor.0;
        let dy = cursor.1 - self.start_cursor.1;
        let resize_west = matches!(
            self.direction,
            winit::window::ResizeDirection::West
                | winit::window::ResizeDirection::NorthWest
                | winit::window::ResizeDirection::SouthWest
        );
        let resize_north = matches!(
            self.direction,
            winit::window::ResizeDirection::North
                | winit::window::ResizeDirection::NorthWest
                | winit::window::ResizeDirection::NorthEast
        );
        let resize_east = matches!(
            self.direction,
            winit::window::ResizeDirection::East
                | winit::window::ResizeDirection::NorthEast
                | winit::window::ResizeDirection::SouthEast
        );
        let resize_south = matches!(
            self.direction,
            winit::window::ResizeDirection::South
                | winit::window::ResizeDirection::SouthWest
                | winit::window::ResizeDirection::SouthEast
        );

        let mut left = f64::from(self.start_outer_position.0);
        let mut top = f64::from(self.start_outer_position.1);
        let mut width = f64::from(self.start_inner_size.0);
        let mut height = f64::from(self.start_inner_size.1);

        if resize_west {
            width = (f64::from(self.start_inner_size.0) - dx).max(f64::from(self.min_inner_size.0));
            left =
                f64::from(self.start_outer_position.0) + f64::from(self.start_inner_size.0) - width;
        } else if resize_east {
            width = (f64::from(self.start_inner_size.0) + dx).max(f64::from(self.min_inner_size.0));
        }

        if resize_north {
            height =
                (f64::from(self.start_inner_size.1) - dy).max(f64::from(self.min_inner_size.1));
            top = f64::from(self.start_outer_position.1) + f64::from(self.start_inner_size.1)
                - height;
        } else if resize_south {
            height =
                (f64::from(self.start_inner_size.1) + dy).max(f64::from(self.min_inner_size.1));
        }

        (
            (
                clamp_f64_to_i32(left.round()),
                clamp_f64_to_i32(top.round()),
            ),
            (
                clamp_f64_to_u32(width.round()),
                clamp_f64_to_u32(height.round()),
            ),
        )
    }
}

fn clamp_f64_to_i32(value: f64) -> i32 {
    value.clamp(f64::from(i32::MIN), f64::from(i32::MAX)) as i32
}

fn clamp_f64_to_u32(value: f64) -> u32 {
    value.clamp(1.0, f64::from(u32::MAX)) as u32
}

pub(super) fn window_resize_direction(
    position: (f64, f64),
    window_size: (u32, u32),
    border: f64,
) -> Option<winit::window::ResizeDirection> {
    let (x, y) = position;
    let (width, height) = (f64::from(window_size.0), f64::from(window_size.1));
    if !x.is_finite()
        || !y.is_finite()
        || !border.is_finite()
        || border <= 0.0
        || width <= 0.0
        || height <= 0.0
        || x < 0.0
        || y < 0.0
        || x >= width
        || y >= height
    {
        return None;
    }

    let border_x = border.min(width / 2.0);
    let border_y = border.min(height / 2.0);
    let horizontal = if x < border_x {
        Some(winit::window::ResizeDirection::West)
    } else if x >= width - border_x {
        Some(winit::window::ResizeDirection::East)
    } else {
        None
    };
    let vertical = if y < border_y {
        Some(winit::window::ResizeDirection::North)
    } else if y >= height - border_y {
        Some(winit::window::ResizeDirection::South)
    } else {
        None
    };

    match (horizontal, vertical) {
        (
            Some(winit::window::ResizeDirection::West),
            Some(winit::window::ResizeDirection::North),
        ) => Some(winit::window::ResizeDirection::NorthWest),
        (
            Some(winit::window::ResizeDirection::West),
            Some(winit::window::ResizeDirection::South),
        ) => Some(winit::window::ResizeDirection::SouthWest),
        (
            Some(winit::window::ResizeDirection::East),
            Some(winit::window::ResizeDirection::North),
        ) => Some(winit::window::ResizeDirection::NorthEast),
        (
            Some(winit::window::ResizeDirection::East),
            Some(winit::window::ResizeDirection::South),
        ) => Some(winit::window::ResizeDirection::SouthEast),
        (Some(direction), None) | (None, Some(direction)) => Some(direction),
        (None, None) => None,
        (Some(_), Some(_)) => None,
    }
}

pub(super) fn window_resize_border_physical(scale_factor: f64) -> f64 {
    if scale_factor.is_finite() && scale_factor > 0.0 {
        WINDOW_RESIZE_BORDER_LOGICAL * scale_factor
    } else {
        WINDOW_RESIZE_BORDER_LOGICAL
    }
}

pub(super) fn min_window_inner_size_physical(scale_factor: f64) -> (u32, u32) {
    let scale_factor = if scale_factor.is_finite() && scale_factor > 0.0 {
        scale_factor
    } else {
        1.0
    };
    (
        clamp_f64_to_u32((MIN_WINDOW_WIDTH_LOGICAL * scale_factor).ceil()),
        clamp_f64_to_u32((MIN_WINDOW_HEIGHT_LOGICAL * scale_factor).ceil()),
    )
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

pub(super) fn selection_auto_scroll_lines(
    pointer_y: f32,
    pane_top: f32,
    pane_bottom: f32,
    cell_height: f32,
) -> i32 {
    if !pointer_y.is_finite()
        || !pane_top.is_finite()
        || !pane_bottom.is_finite()
        || !cell_height.is_finite()
        || pane_bottom <= pane_top
        || cell_height <= 0.0
    {
        return 0;
    }

    let distance = if pointer_y < pane_top {
        pane_top - pointer_y
    } else if pointer_y >= pane_bottom {
        pointer_y - pane_bottom + 1.0
    } else {
        return 0;
    };
    let magnitude = (distance / cell_height).ceil().clamp(1.0, 8.0) as i32;
    if pointer_y < pane_top {
        magnitude
    } else {
        -magnitude
    }
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

pub(super) fn should_copy_left_selection(gesture: Option<LeftMouseGesture>) -> bool {
    matches!(gesture, Some(LeftMouseGesture::LocalSelection))
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

pub(super) fn take_left_mouse_gesture_state(
    gesture: &mut Option<LeftMouseGesture>,
) -> Option<(usize, usize)> {
    let gesture = gesture.take();
    terminal_report_release_cell(gesture, None)
}

pub(super) fn prepare_for_active_tab_change_state(
    gesture: &mut Option<LeftMouseGesture>,
    click_sequence: (&mut ClickState, &mut Instant, &mut (usize, usize)),
    now: Instant,
) -> Option<(usize, usize)> {
    let release_cell = take_left_mouse_gesture_state(gesture);
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

#[cfg(test)]
mod window_resize_tests {
    use super::*;

    #[test]
    fn resize_hit_testing_distinguishes_edges_and_corners() {
        let size = (1_280, 800);
        assert_eq!(
            window_resize_direction((0.0, 400.0), size, 8.0),
            Some(winit::window::ResizeDirection::West)
        );
        assert_eq!(
            window_resize_direction((1_279.0, 400.0), size, 8.0),
            Some(winit::window::ResizeDirection::East)
        );
        assert_eq!(
            window_resize_direction((4.0, 4.0), size, 8.0),
            Some(winit::window::ResizeDirection::NorthWest)
        );
        assert_eq!(
            window_resize_direction((1_276.0, 796.0), size, 8.0),
            Some(winit::window::ResizeDirection::SouthEast)
        );
        assert_eq!(window_resize_direction((640.0, 400.0), size, 8.0), None);
        assert_eq!(window_resize_direction((-1.0, 400.0), size, 8.0), None);
    }

    #[test]
    fn manual_resize_keeps_the_opposite_corner_fixed_at_minimum_size() {
        let drag = WindowResizeDrag {
            direction: winit::window::ResizeDirection::NorthWest,
            start_cursor: (100.0, 100.0),
            start_outer_position: (20, 30),
            start_inner_size: (1_000, 700),
            min_inner_size: (640, 400),
        };

        let (outer, size) = drag.geometry_for_cursor((600.0, 500.0));
        assert_eq!(size, (640, 400));
        assert_eq!(outer, (380, 330));
    }

    #[test]
    fn resize_border_and_minimum_size_scale_with_dpi() {
        assert_eq!(window_resize_border_physical(2.0), 16.0);
        assert_eq!(min_window_inner_size_physical(1.5), (960, 600));
        assert_eq!(window_resize_border_physical(f64::NAN), 8.0);
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
