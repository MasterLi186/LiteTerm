use super::{
    favorite_display_text, favorites_popup_size, history_popup_layer_ids, history_popup_position,
    history_popup_row_id, history_popup_size, CommandBar, CommandBarStorage, QuickCommand,
    FAVORITES_POPUP_WIDTH, HISTORY_POPUP_ACTIONS_MIN_ROW_WIDTH, HISTORY_POPUP_ACTIONS_WIDTH,
    HISTORY_POPUP_CHROME_HEIGHT, HISTORY_POPUP_MARGIN, HISTORY_POPUP_ROW_HEIGHT,
    HISTORY_POPUP_WIDTH,
};
use egui::{pos2, vec2, Rect};

const EPSILON: f32 = 0.01;

fn assert_finite_non_negative(size: egui::Vec2) {
    assert!(size.x.is_finite());
    assert!(size.y.is_finite());
    assert!(size.x >= 0.0);
    assert!(size.y >= 0.0);
}

fn assert_inside_screen(position: egui::Pos2, size: egui::Vec2, screen: Rect) {
    assert!(position.x.is_finite());
    assert!(position.y.is_finite());
    assert!(position.x + EPSILON >= screen.left());
    assert!(position.y + EPSILON >= screen.top());
    assert!(position.x + size.x <= screen.right() + EPSILON);
    assert!(position.y + size.y <= screen.bottom() + EPSILON);
}

fn test_command_bar(history: &[&str]) -> CommandBar {
    CommandBar {
        storage: CommandBarStorage::disabled(),
        commands: Vec::new(),
        input_text: String::new(),
        history: history
            .iter()
            .map(|command| (*command).to_owned())
            .collect(),
        favorites: Vec::new(),
        show_history: false,
        show_favorites: false,
        show_add: false,
        add_label: String::new(),
        add_command: String::new(),
        add_error: String::new(),
        edit_index: None,
        last_history_button_rect: None,
        last_favorites_button_rect: None,
    }
}

#[test]
fn ui_fixture_disables_persistence() {
    let command_bar = test_command_bar(&["echo isolated"]);

    assert!(command_bar.storage.is_disabled());
}

#[test]
fn favorite_display_text_does_not_repeat_identical_label_and_command() {
    assert_eq!(favorite_display_text("df -h", "df -h"), "df -h");
    assert_eq!(favorite_display_text("", "ls -al"), "ls -al");
    assert_eq!(
        favorite_display_text("查看磁盘", "df -h"),
        "查看磁盘  ·  df -h"
    );
}

#[test]
fn favorites_popup_recomputes_bounded_size_for_each_window_size() {
    let desktop = Rect::from_min_size(pos2(0.0, 0.0), vec2(1280.0, 720.0));
    let narrow = Rect::from_min_size(pos2(0.0, 0.0), vec2(240.0, 180.0));

    let desktop_size = favorites_popup_size(20, desktop);
    let narrow_size = favorites_popup_size(20, narrow);

    assert_eq!(desktop_size.x, FAVORITES_POPUP_WIDTH);
    assert!(desktop_size.y <= 280.0);
    assert!(narrow_size.x <= narrow.width() - HISTORY_POPUP_MARGIN * 2.0 + EPSILON);
    assert!(narrow_size.y <= narrow.height() - HISTORY_POPUP_MARGIN * 2.0 + EPSILON);
}

#[test]
fn open_favorites_popup_reflows_inside_a_resized_window() {
    let ctx = egui::Context::default();
    let mut command_bar = test_command_bar(&[]);
    command_bar.favorites = vec![
        QuickCommand {
            label: "查看磁盘".into(),
            command: "df -h".into(),
            system: false,
        },
        QuickCommand {
            label: "ls -al".into(),
            command: "ls -al".into(),
            system: false,
        },
    ];
    command_bar.show_favorites = true;

    let desktop = Rect::from_min_size(pos2(0.0, 0.0), vec2(1280.0, 720.0));
    for _ in 0..3 {
        let _ = run_ui_frame_on_screen(&ctx, &mut command_bar, desktop, Vec::new());
    }

    let resized = Rect::from_min_size(pos2(0.0, 0.0), vec2(320.0, 240.0));
    for _ in 0..3 {
        let _ = run_ui_frame_on_screen(&ctx, &mut command_bar, resized, Vec::new());
    }

    let popup = ctx
        .memory(|memory| memory.area_rect(egui::Id::new("command_favorites_popup")))
        .expect("收藏弹窗应存在");
    assert!(popup.is_finite());
    assert!(
        resized.contains_rect(popup),
        "popup={popup:?}, screen={resized:?}"
    );
    assert!(
        popup.bottom()
            <= command_bar
                .last_favorites_button_rect
                .expect("应记录收藏按钮位置")
                .top()
                + EPSILON,
        "popup={popup:?}"
    );
}

#[test]
fn quick_command_editor_blocks_terminal_input_only_while_open() {
    let mut command_bar = test_command_bar(&[]);
    assert!(!command_bar.has_blocking_dialog());

    command_bar.show_add = true;
    assert!(command_bar.has_blocking_dialog());

    command_bar.show_add = false;
    assert!(!command_bar.has_blocking_dialog());
}

fn run_ui_frame(
    ctx: &egui::Context,
    command_bar: &mut CommandBar,
    events: Vec<egui::Event>,
) -> Option<String> {
    run_ui_frame_on_screen(
        ctx,
        command_bar,
        Rect::from_min_size(pos2(0.0, 0.0), vec2(800.0, 600.0)),
        events,
    )
}

fn run_ui_frame_on_screen(
    ctx: &egui::Context,
    command_bar: &mut CommandBar,
    screen: Rect,
    events: Vec<egui::Event>,
) -> Option<String> {
    run_ui_frame_on_screen_with_probe(ctx, command_bar, screen, events).0
}

fn run_ui_frame_on_screen_with_probe(
    ctx: &egui::Context,
    command_bar: &mut CommandBar,
    screen: Rect,
    events: Vec<egui::Event>,
) -> (Option<String>, bool) {
    let input = egui::RawInput {
        screen_rect: Some(screen),
        events,
        ..Default::default()
    };
    let mut command = None;
    let mut probe_clicked = false;
    let _ = ctx.run(input, |ctx| {
        probe_clicked = egui::Area::new(egui::Id::new("history_underlying_probe"))
            .fixed_pos(pos2(8.0, 8.0))
            .show(ctx, |ui| {
                ui.add_sized(vec2(40.0, 24.0), egui::Button::new("底层"))
                    .clicked()
            })
            .inner;
        command = command_bar.ui(ctx, 0.0);
    });
    (command, probe_clicked)
}

fn render_open_history_popup(
    screen_size: egui::Vec2,
    history_len: usize,
) -> (Rect, Rect, Vec<Rect>) {
    let screen = Rect::from_min_size(pos2(0.0, 0.0), screen_size);
    let ctx = egui::Context::default();
    let commands: Vec<String> = (0..history_len)
        .map(|index| format!("echo history-{index}"))
        .collect();
    let command_refs: Vec<&str> = commands.iter().map(String::as_str).collect();
    let mut command_bar = test_command_bar(&command_refs);
    command_bar.show_history = true;

    for _ in 0..3 {
        let _ = run_ui_frame_on_screen(&ctx, &mut command_bar, screen, Vec::new());
    }

    let popup_rect = ctx
        .memory(|memory| memory.area_rect(history_popup_layer_ids().1.id))
        .expect("历史弹窗应存在");
    let button_rect = command_bar
        .last_history_button_rect
        .expect("历史按钮实际矩形应被记录");
    let row_rects = (0..history_len)
        .filter_map(|index| {
            ctx.read_response(history_popup_row_id(index))
                .map(|response| response.rect)
        })
        .collect();
    (popup_rect, button_rect, row_rects)
}

fn pointer_button_event(position: egui::Pos2, pressed: bool) -> egui::Event {
    egui::Event::PointerButton {
        pos: position,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::NONE,
    }
}

fn click(
    ctx: &egui::Context,
    command_bar: &mut CommandBar,
    position: egui::Pos2,
) -> Option<String> {
    let _ = run_ui_frame(
        ctx,
        command_bar,
        vec![
            egui::Event::PointerMoved(position),
            pointer_button_event(position, true),
        ],
    );
    run_ui_frame(
        ctx,
        command_bar,
        vec![pointer_button_event(position, false)],
    )
}

fn click_with_probe(
    ctx: &egui::Context,
    command_bar: &mut CommandBar,
    position: egui::Pos2,
) -> (Option<String>, bool) {
    let screen = Rect::from_min_size(pos2(0.0, 0.0), vec2(800.0, 600.0));
    let (_, pressed_probe) = run_ui_frame_on_screen_with_probe(
        ctx,
        command_bar,
        screen,
        vec![
            egui::Event::PointerMoved(position),
            pointer_button_event(position, true),
        ],
    );
    let (command, released_probe) = run_ui_frame_on_screen_with_probe(
        ctx,
        command_bar,
        screen,
        vec![pointer_button_event(position, false)],
    );
    (command, pressed_probe || released_probe)
}

fn open_history_with_actual_button(ctx: &egui::Context, command_bar: &mut CommandBar) {
    let _ = run_ui_frame(ctx, command_bar, Vec::new());
    let button_center = command_bar
        .last_history_button_rect
        .expect("应记录真实历史按钮矩形")
        .center();
    let _ = click(ctx, command_bar, button_center);
    assert!(command_bar.show_history, "真实历史按钮点击应打开弹窗");
    for _ in 0..3 {
        let _ = run_ui_frame(ctx, command_bar, Vec::new());
    }
}

#[test]
fn history_popup_uses_desktop_width_and_caps_height() {
    let screen = Rect::from_min_size(pos2(0.0, 0.0), vec2(1280.0, 720.0));

    let size = history_popup_size(50, screen);

    assert!((size.x - 384.0).abs() < EPSILON);
    assert!(size.y <= 288.0);
    assert!(size.y > 0.0);
}

#[test]
fn history_popup_shrinks_to_narrow_screen_with_four_point_margin() {
    let screen = Rect::from_min_size(pos2(20.0, 10.0), vec2(300.0, 220.0));

    let size = history_popup_size(20, screen);
    let position = history_popup_position(
        Rect::from_min_size(pos2(280.0, 200.0), vec2(20.0, 18.0)),
        size,
        screen,
    );

    assert!((size.x - 292.0).abs() < EPSILON);
    assert!(position.x + EPSILON >= screen.left() + 4.0);
    assert!(position.y + EPSILON >= screen.top() + 4.0);
    assert!(position.x + size.x <= screen.right() - 4.0 + EPSILON);
    assert!(position.y + size.y <= screen.bottom() - 4.0 + EPSILON);
}

#[test]
fn history_popup_right_aligns_above_button_when_space_allows() {
    let screen = Rect::from_min_size(pos2(0.0, 0.0), vec2(1280.0, 720.0));
    let button = Rect::from_min_size(pos2(900.0, 680.0), vec2(20.0, 18.0));
    let size = history_popup_size(8, screen);

    let position = history_popup_position(button, size, screen);

    assert!((position.x + size.x - button.right()).abs() < EPSILON);
    assert!(position.y + size.y <= button.top() + EPSILON);
}

#[test]
fn history_popup_clamps_near_left_top_and_on_narrow_screen() {
    let screens_and_buttons = [
        (
            Rect::from_min_size(pos2(0.0, 0.0), vec2(1280.0, 720.0)),
            Rect::from_min_size(pos2(1.0, 680.0), vec2(20.0, 18.0)),
        ),
        (
            Rect::from_min_size(pos2(0.0, 0.0), vec2(320.0, 180.0)),
            Rect::from_min_size(pos2(8.0, 2.0), vec2(20.0, 18.0)),
        ),
    ];

    for (screen, button) in screens_and_buttons {
        let size = history_popup_size(50, screen);
        let position = history_popup_position(button, size, screen);
        assert_inside_screen(position, size, screen);
    }
}

#[test]
fn history_popup_geometry_is_safe_for_non_finite_and_tiny_screens() {
    let screens = [
        Rect::from_min_max(
            pos2(f32::NAN, f32::NEG_INFINITY),
            pos2(f32::INFINITY, f32::NAN),
        ),
        Rect::from_min_size(pos2(7.0, 11.0), vec2(3.0, 2.0)),
        Rect::from_min_size(pos2(9.0, 13.0), vec2(0.0, 0.0)),
    ];
    let invalid_button = Rect::from_min_max(
        pos2(f32::NAN, f32::INFINITY),
        pos2(f32::NAN, f32::NEG_INFINITY),
    );

    for screen in screens {
        let size = history_popup_size(usize::MAX, screen);
        let position = history_popup_position(invalid_button, size, screen);
        assert_finite_non_negative(size);
        assert!(position.x.is_finite());
        assert!(position.y.is_finite());
        if screen.is_finite() {
            assert_inside_screen(position, size, screen);
        }
    }
}

#[test]
fn rendered_history_popup_outer_rect_matches_geometry_and_screen_bounds() {
    let rendered: Vec<_> = [
        vec2(800.0, 600.0),
        vec2(100.0, 180.0),
        vec2(50.0, 100.0),
        vec2(8.0, 8.0),
    ]
    .into_iter()
    .map(|screen_size| {
        let screen = Rect::from_min_size(pos2(0.0, 0.0), screen_size);
        let expected_size = history_popup_size(8, screen);
        let (popup_rect, button_rect, _) = render_open_history_popup(screen_size, 8);
        (screen_size, expected_size, popup_rect, button_rect)
    })
    .collect();

    for (screen_size, expected_size, popup_rect, button_rect) in rendered {
        let screen = Rect::from_min_size(pos2(0.0, 0.0), screen_size);
        let margin_x = HISTORY_POPUP_MARGIN.min(screen_size.x / 2.0);
        let margin_y = HISTORY_POPUP_MARGIN.min(screen_size.y / 2.0);
        assert!(popup_rect.is_finite(), "{screen_size:?}: {popup_rect:?}");
        assert!(
            popup_rect.left() + EPSILON >= screen.left() + margin_x,
            "{screen_size:?}: {popup_rect:?}"
        );
        assert!(
            popup_rect.top() + EPSILON >= screen.top() + margin_y,
            "{screen_size:?}: {popup_rect:?}"
        );
        assert!(
            popup_rect.right() <= screen.right() - margin_x + EPSILON,
            "{screen_size:?}: {popup_rect:?}"
        );
        assert!(
            popup_rect.bottom() <= screen.bottom() - margin_y + EPSILON,
            "{screen_size:?}: {popup_rect:?}"
        );
        assert!(
            (popup_rect.width() - expected_size.x).abs() < EPSILON,
            "{screen_size:?}: actual={popup_rect:?}, expected={expected_size:?}"
        );
        assert!(
            (popup_rect.height() - expected_size.y).abs() < EPSILON,
            "{screen_size:?}: actual={popup_rect:?}, expected={expected_size:?}"
        );
        if screen_size.x > HISTORY_POPUP_WIDTH + HISTORY_POPUP_MARGIN * 2.0 {
            assert!(
                (popup_rect.right() - button_rect.right()).abs() < EPSILON,
                "actual={popup_rect:?}, button={button_rect:?}"
            );
        }
    }
}

#[test]
fn rendered_history_rows_advance_by_exactly_twenty_eight_points() {
    let (popup_rect, _, row_rects) = render_open_history_popup(vec2(800.0, 600.0), 8);

    assert_eq!(row_rects.len(), 8);
    for row in &row_rects {
        assert!((row.height() - HISTORY_POPUP_ROW_HEIGHT).abs() < EPSILON);
    }
    for rows in row_rects.windows(2) {
        assert!(
            (rows[1].top() - rows[0].top() - HISTORY_POPUP_ROW_HEIGHT).abs() < EPSILON,
            "相邻行实际步长={}，期望={HISTORY_POPUP_ROW_HEIGHT}",
            rows[1].top() - rows[0].top()
        );
    }
    assert!(
        (popup_rect.height() - (HISTORY_POPUP_CHROME_HEIGHT + 8.0 * HISTORY_POPUP_ROW_HEIGHT))
            .abs()
            < EPSILON
    );
}

#[test]
fn normal_width_history_row_exposes_all_actions_and_executes() {
    let ctx = egui::Context::default();
    let mut command_bar = test_command_bar(&["echo actions"]);
    assert!(command_bar.storage.is_disabled());
    open_history_with_actual_button(&ctx, &mut command_bar);

    let row_id = history_popup_row_id(0);
    let execute = ctx
        .read_response(row_id.with("execute"))
        .expect("正常宽度应显示立即执行");
    assert!(
        ctx.read_response(row_id.with("copy")).is_some(),
        "正常宽度应显示复制"
    );
    assert!(
        ctx.read_response(row_id.with("favorite")).is_some(),
        "正常宽度应显示收藏"
    );
    assert!(
        ctx.read_response(row_id.with("delete")).is_some(),
        "正常宽度应显示删除"
    );

    let executed = click(&ctx, &mut command_bar, execute.rect.center());

    assert_eq!(
        executed.as_deref(),
        Some("echo actions\n"),
        "点击后show_history={}，execute_rect={:?}，popup_rect={:?}",
        command_bar.show_history,
        execute.rect,
        ctx.memory(|memory| memory.area_rect(history_popup_layer_ids().1.id))
    );
    assert!(!command_bar.show_history);
}

#[test]
fn normal_width_history_actions_have_stable_non_overlapping_rects() {
    let ctx = egui::Context::default();
    let mut command_bar = test_command_bar(&["echo geometry"]);
    open_history_with_actual_button(&ctx, &mut command_bar);

    let row_id = history_popup_row_id(0);
    let row_rect = ctx.read_response(row_id).expect("历史行应存在").rect;
    let command_rect = ctx
        .read_response(row_id.with("command"))
        .expect("命令点击区应存在")
        .rect;
    let action_rects: Vec<Rect> = ["execute", "copy", "favorite", "delete"]
        .into_iter()
        .map(|action| {
            ctx.read_response(row_id.with(action))
                .unwrap_or_else(|| panic!("缺少稳定动作响应：{action}"))
                .rect
        })
        .collect();

    for rect in &action_rects {
        assert!((rect.width() - 20.0).abs() < EPSILON);
        assert!((rect.height() - 20.0).abs() < EPSILON);
        assert!(row_rect.contains_rect(*rect));
    }
    for pair in action_rects.windows(2) {
        assert!(pair[0].right() <= pair[1].left() + EPSILON);
        assert!((pair[1].center().x - pair[0].center().x - 20.0).abs() < EPSILON);
    }
    assert!(command_rect.right() <= action_rects[0].left() + EPSILON);
}

#[test]
fn delete_history_action_removes_only_target_and_keeps_popup_open() {
    let ctx = egui::Context::default();
    let mut command_bar = test_command_bar(&["echo first", "printf '删除我🌏'", "echo last"]);
    assert!(command_bar.storage.is_disabled());
    open_history_with_actual_button(&ctx, &mut command_bar);

    let delete = ctx
        .read_response(history_popup_row_id(1).with("delete"))
        .expect("正常宽度应显示删除操作");
    let emitted = click(&ctx, &mut command_bar, delete.rect.center());

    assert_eq!(emitted, None);
    assert_eq!(command_bar.history, ["echo first", "echo last"]);
    assert!(command_bar.show_history);
}

#[test]
fn delete_history_action_removes_all_duplicate_unicode_commands_in_order() {
    let ctx = egui::Context::default();
    let target = "printf '你好，世界🌏'";
    let mut command_bar = test_command_bar(&["保留 α", target, "保留 β", target, "保留 γ"]);
    assert!(command_bar.storage.is_disabled());
    open_history_with_actual_button(&ctx, &mut command_bar);

    let delete = ctx
        .read_response(history_popup_row_id(1).with("delete"))
        .expect("Unicode 历史行应显示删除操作");
    let _ = click(&ctx, &mut command_bar, delete.rect.center());

    assert_eq!(command_bar.history, ["保留 α", "保留 β", "保留 γ"]);
    assert!(command_bar.show_history);
}

#[test]
fn history_actions_appear_at_derived_four_button_width_threshold() {
    assert!((HISTORY_POPUP_ACTIONS_WIDTH - 84.0).abs() < EPSILON);
    assert!((HISTORY_POPUP_ACTIONS_MIN_ROW_WIDTH - 140.0).abs() < EPSILON);

    for (screen_width, expected_row_width, expect_actions) in
        [(149.0, 139.0, false), (150.0, 140.0, true)]
    {
        let ctx = egui::Context::default();
        let screen = Rect::from_min_size(pos2(0.0, 0.0), vec2(screen_width, 600.0));
        let mut command_bar = test_command_bar(&["echo threshold"]);
        command_bar.show_history = true;
        for _ in 0..3 {
            let _ = run_ui_frame_on_screen(&ctx, &mut command_bar, screen, Vec::new());
        }

        let row_id = history_popup_row_id(0);
        let row_rect = ctx.read_response(row_id).expect("临界宽历史行应存在").rect;
        assert!(
            (row_rect.width() - expected_row_width).abs() < EPSILON,
            "screen={screen_width}, row={row_rect:?}"
        );
        for action in ["execute", "copy", "favorite", "delete"] {
            assert_eq!(
                ctx.read_response(row_id.with(action)).is_some(),
                expect_actions,
                "screen={screen_width}, action={action}, row={row_rect:?}"
            );
        }
    }
}

#[test]
fn clear_history_uses_disabled_fixture_storage() {
    let ctx = egui::Context::default();
    let mut command_bar = test_command_bar(&["echo first", "echo second"]);
    assert!(command_bar.storage.is_disabled());
    open_history_with_actual_button(&ctx, &mut command_bar);

    let clear = ctx
        .read_response(egui::Id::new("command_history_clear"))
        .expect("历史弹窗应显示清空操作");
    let _ = click(&ctx, &mut command_bar, clear.rect.center());

    assert!(command_bar.history.is_empty());
}

#[test]
fn favorite_toggle_uses_disabled_fixture_storage() {
    let ctx = egui::Context::default();
    let mut command_bar = test_command_bar(&["echo favorite"]);
    assert!(command_bar.storage.is_disabled());
    open_history_with_actual_button(&ctx, &mut command_bar);

    let favorite_id = history_popup_row_id(0).with("favorite");
    let add_favorite = ctx
        .read_response(favorite_id)
        .expect("历史行应显示收藏操作");
    let _ = click(&ctx, &mut command_bar, add_favorite.rect.center());
    assert_eq!(command_bar.favorites.len(), 1);
    assert_eq!(command_bar.favorites[0].command, "echo favorite");

    for _ in 0..2 {
        let _ = run_ui_frame(&ctx, &mut command_bar, Vec::new());
    }
    let remove_favorite = ctx
        .read_response(favorite_id)
        .expect("收藏后仍应显示取消收藏操作");
    let _ = click(&ctx, &mut command_bar, remove_favorite.rect.center());

    assert!(command_bar.favorites.is_empty());
}

#[test]
fn history_popup_remains_interactive_after_outside_close_and_reopen() {
    let ctx = egui::Context::default();
    let mut command_bar = test_command_bar(&["echo 跨帧层级"]);

    open_history_with_actual_button(&ctx, &mut command_bar);

    for cycle in 0..3 {
        let (_, leaked_to_underlying) = click_with_probe(&ctx, &mut command_bar, pos2(20.0, 20.0));
        assert!(
            !command_bar.show_history,
            "第 {cycle} 轮外部点击应关闭历史弹窗，底层clicked={leaked_to_underlying}"
        );
        assert!(
            !leaked_to_underlying,
            "第 {cycle} 轮遮罩不得把点击泄漏给底层按钮"
        );

        let _ = run_ui_frame(&ctx, &mut command_bar, Vec::new());
        command_bar.input_text.clear();
        open_history_with_actual_button(&ctx, &mut command_bar);

        let popup_rect = ctx
            .memory(|memory| memory.area_rect(history_popup_layer_ids().1.id))
            .expect("重新打开后应存在历史弹窗");
        let command_text_position = popup_rect.min + vec2(24.0, 46.0);
        let executed = click(&ctx, &mut command_bar, command_text_position);

        assert!(
            command_bar.show_history,
            "第 {cycle} 轮点击弹窗内部不得被遮罩误判为外部点击"
        );
        assert_eq!(command_bar.input_text, "echo 跨帧层级");
        assert_eq!(executed, None);
    }
}

#[test]
fn escape_closes_only_the_open_history_popup() {
    let ctx = egui::Context::default();
    let mut command_bar = test_command_bar(&["echo escape"]);
    open_history_with_actual_button(&ctx, &mut command_bar);

    let _ = run_ui_frame(
        &ctx,
        &mut command_bar,
        vec![egui::Event::Key {
            key: egui::Key::Escape,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        }],
    );

    assert!(!command_bar.show_history);
    assert!(!command_bar.show_favorites);
}

#[test]
fn quick_command_dialog_enter_saves_valid_command() {
    let ctx = egui::Context::default();
    let mut command_bar = test_command_bar(&[]);
    command_bar.show_add = true;
    command_bar.add_label = "检查磁盘".into();
    command_bar.add_command = "df -h".into();

    let _ = run_ui_frame(
        &ctx,
        &mut command_bar,
        vec![egui::Event::Key {
            key: egui::Key::Enter,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        }],
    );

    assert!(!command_bar.show_add);
    assert_eq!(command_bar.commands.len(), 1);
    assert_eq!(command_bar.commands[0].label, "检查磁盘");
    assert_eq!(command_bar.commands[0].command, "df -h");
}

#[test]
fn quick_command_dialog_escape_cancels_without_saving() {
    let ctx = egui::Context::default();
    let mut command_bar = test_command_bar(&[]);
    command_bar.show_add = true;
    command_bar.add_label = "不会保存".into();
    command_bar.add_command = "echo discarded".into();

    let _ = run_ui_frame(
        &ctx,
        &mut command_bar,
        vec![egui::Event::Key {
            key: egui::Key::Escape,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        }],
    );

    assert!(!command_bar.show_add);
    assert!(command_bar.commands.is_empty());
}

#[test]
fn edit_quick_command_dialog_enter_saves_changes() {
    let ctx = egui::Context::default();
    let mut command_bar = test_command_bar(&[]);
    command_bar.commands.push(QuickCommand {
        label: "旧标签".into(),
        command: "echo old".into(),
        system: false,
    });
    command_bar.show_add = true;
    command_bar.edit_index = Some(0);
    command_bar.add_label = "新标签".into();
    command_bar.add_command = "echo new".into();

    let _ = run_ui_frame(
        &ctx,
        &mut command_bar,
        vec![egui::Event::Key {
            key: egui::Key::Enter,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        }],
    );

    assert!(!command_bar.show_add);
    assert_eq!(command_bar.commands[0].label, "新标签");
    assert_eq!(command_bar.commands[0].command, "echo new");
}

#[test]
fn edit_quick_command_dialog_escape_discards_changes() {
    let ctx = egui::Context::default();
    let mut command_bar = test_command_bar(&[]);
    command_bar.commands.push(QuickCommand {
        label: "原标签".into(),
        command: "echo original".into(),
        system: false,
    });
    command_bar.show_add = true;
    command_bar.edit_index = Some(0);
    command_bar.add_label = "不应保存".into();
    command_bar.add_command = "echo discarded".into();

    let _ = run_ui_frame(
        &ctx,
        &mut command_bar,
        vec![egui::Event::Key {
            key: egui::Key::Escape,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        }],
    );

    assert!(!command_bar.show_add);
    assert_eq!(command_bar.commands[0].label, "原标签");
    assert_eq!(command_bar.commands[0].command, "echo original");
}
