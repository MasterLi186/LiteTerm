use super::{
    click_state_after_press, completion_key_action, completion_popup_may_render,
    completion_user_input_effect, is_modifier_only_key, keyboard_input_route, left_mouse_gesture,
    plan_settings_apply, point_in_terminal_bounds, prepare_for_active_tab_change_state,
    refresh_side_for_event, reset_click_sequence_state, selection_auto_scroll_lines, sftp,
    should_clear_selection_on_focus_loss, should_copy_left_selection,
    should_pass_consumed_mouse_input, should_pass_keyboard_to_terminal,
    should_pass_pointer_to_terminal, should_process_left_release, terminal_backspace_sequence,
    terminal_grid_bounds, terminal_input_blocked, terminal_report_release_cell,
    write_completion_invalidating_control_sequence, ClickState, CompletionKeyAction,
    CompletionSessionKey, CompletionUserInputEffect, FrameActionTiming, KeyboardInputRoute,
    LeftMouseGesture, SettingsApplyPlan, TerminalWheelAccumulator, UserEvent,
};
use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};
use winit::event::{ElementState, MouseScrollDelta};
use winit::keyboard::{Key, ModifiersState, NamedKey};

fn remote_params(user: &str, host: &str, port: u16) -> super::ssh::ConnectionParams {
    super::ssh::ConnectionParams {
        user: user.into(),
        host: host.into(),
        port,
        auth: "key".into(),
        key_path: "private-key-sentinel".into(),
        password: "password-sentinel".into(),
    }
}

fn history_request(session: CompletionSessionKey) -> super::smart_completion::HistoryLoadRequest {
    super::smart_completion::CompletionState::new(session).mark_history_loading()
}
include!("layout_tests/part_01.rs");
include!("layout_tests/part_02.rs");
include!("layout_tests/part_03.rs");
include!("layout_tests/part_04.rs");
include!("layout_tests/part_05.rs");
include!("layout_tests/part_06.rs");
