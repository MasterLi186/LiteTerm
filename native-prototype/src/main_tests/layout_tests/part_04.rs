    #[test]
    fn shift_overrides_mouse_mode_with_local_selection_gesture() {
        assert_eq!(
            left_mouse_gesture(true, true, (4, 2)),
            LeftMouseGesture::LocalSelection
        );
    }

    #[test]
    fn shift_is_a_modifier_only_key_and_must_not_prepare_terminal_input() {
        assert!(is_modifier_only_key(&Key::Named(NamedKey::Shift)));
        assert!(is_modifier_only_key(&Key::Named(NamedKey::Control)));
        assert!(!is_modifier_only_key(&Key::Named(NamedKey::Enter)));
        assert!(!is_modifier_only_key(&Key::Character("a".into())));
    }

    #[test]
    fn non_mouse_mode_starts_local_selection_gesture() {
        assert_eq!(
            left_mouse_gesture(false, false, (4, 2)),
            LeftMouseGesture::LocalSelection
        );
    }

    #[test]
    fn stored_terminal_route_survives_later_modifier_changes() {
        let gesture = left_mouse_gesture(true, false, (4, 2));
        let (_current_mouse_mode, _current_shift) = (false, true);

        assert_eq!(
            terminal_report_release_cell(Some(gesture), Some((7, 3))),
            Some((7, 3))
        );
        assert_eq!(
            terminal_report_release_cell(Some(gesture), None),
            Some((4, 2))
        );
    }

    #[test]
    fn release_attempts_copy_only_for_local_selection() {
        let terminal_report = Some(LeftMouseGesture::TerminalReport { last_cell: (4, 2) });
        let local_selection = Some(LeftMouseGesture::LocalSelection);

        assert!(!should_copy_left_selection(terminal_report));
        assert!(should_copy_left_selection(local_selection));
    }

    #[test]
    fn active_left_release_is_processed_outside_terminal() {
        assert!(should_process_left_release(false, true));
        assert!(!should_process_left_release(false, false));
        assert!(should_process_left_release(true, false));
    }

    #[test]
    fn consumed_mouse_input_passes_only_active_left_release() {
        assert!(!should_pass_consumed_mouse_input(true, false));
        assert!(should_pass_consumed_mouse_input(true, true));
        assert!(should_pass_consumed_mouse_input(false, true));
    }

    #[test]
    fn focus_loss_clears_unfinished_local_selection_only() {
        assert!(should_clear_selection_on_focus_loss(Some(
            LeftMouseGesture::LocalSelection
        )));
        assert!(!should_clear_selection_on_focus_loss(Some(
            LeftMouseGesture::TerminalReport { last_cell: (4, 2) }
        )));
        assert!(!should_clear_selection_on_focus_loss(None));
    }

    #[test]
    fn active_tab_change_preparation_releases_report_and_resets_click_sequence() {
        let mut gesture = Some(LeftMouseGesture::TerminalReport { last_cell: (4, 2) });
        let mut click_state = ClickState::Single;
        let now = Instant::now();
        let mut last_click_time = now;
        let mut last_click_pos = (4, 2);

        let release_cell = prepare_for_active_tab_change_state(
            &mut gesture,
            (&mut click_state, &mut last_click_time, &mut last_click_pos),
            now,
        );

        assert_eq!(release_cell, Some((4, 2)));
        assert_eq!(gesture, None);
        assert_eq!(click_state, ClickState::None);

        click_state = ClickState::Double;
        prepare_for_active_tab_change_state(
            &mut gesture,
            (&mut click_state, &mut last_click_time, &mut last_click_pos),
            now,
        );
        assert_eq!(click_state, ClickState::None);
    }

    #[test]
    fn reset_click_sequence_forces_next_click_to_single() {
        let cell = (4, 2);
        let now = Instant::now();
        let mut click_state = ClickState::Double;
        let mut last_click_time = now;
        let mut last_click_pos = cell;

        reset_click_sequence_state(
            &mut click_state,
            &mut last_click_time,
            &mut last_click_pos,
            now,
        );

        assert_eq!(click_state, ClickState::None);
        assert!(now.duration_since(last_click_time) > Duration::from_millis(400));
        assert_ne!(last_click_pos, cell);
        assert_eq!(
            click_state_after_press(click_state, 1, true),
            ClickState::Single
        );
    }

    #[test]
    fn file_browser_area_is_not_terminal_input() {
        assert!(point_in_terminal_bounds(
            300.0, 200.0, 220.0, 36.0, 700.0, 500.0
        ));
        assert!(!point_in_terminal_bounds(
            300.0, 650.0, 220.0, 36.0, 700.0, 500.0
        ));
        assert!(!point_in_terminal_bounds(
            100.0, 200.0, 220.0, 36.0, 700.0, 500.0
        ));
    }

    #[test]
    fn terminal_grid_rejects_right_padding() {
        let (right, bottom) = terminal_grid_bounds(220.0, 36.0, 600.0, 10.0, 20.0, 48, 20);

        assert!(point_in_terminal_bounds(
            699.0, 200.0, 220.0, 36.0, right, bottom
        ));
        assert!(!point_in_terminal_bounds(
            700.0, 200.0, 220.0, 36.0, right, bottom
        ));
    }

    #[test]
    fn terminal_grid_accepts_rendered_bottom_safety_row_but_rejects_layout_below_it() {
        let (right, bottom) = terminal_grid_bounds(220.0, 36.0, 600.0, 10.0, 20.0, 48, 20);

        assert_eq!(bottom, 456.0);
        assert!(point_in_terminal_bounds(
            300.0, 455.0, 220.0, 36.0, right, bottom
        ));
        assert!(!point_in_terminal_bounds(
            300.0, 500.0, 220.0, 36.0, right, bottom
        ));
    }

    #[test]
    fn terminal_grid_bottom_row_is_clipped_before_the_command_bar() {
        let (right, bottom) = terminal_grid_bounds(220.0, 36.0, 445.0, 10.0, 20.0, 48, 20);

        assert_eq!(bottom, 445.0);
        assert!(point_in_terminal_bounds(
            300.0, 444.9, 220.0, 36.0, right, bottom
        ));
        assert!(!point_in_terminal_bounds(
            300.0, 445.0, 220.0, 36.0, right, bottom
        ));
    }

    #[test]
    fn synthetic_pressed_is_dropped() {
        assert_eq!(
            keyboard_input_route(ElementState::Pressed, true),
            KeyboardInputRoute::Drop
        );
    }

    #[test]
    fn alt_backspace_uses_meta_delete_while_plain_backspace_stays_del() {
        assert_eq!(terminal_backspace_sequence(false), "\x7f");
        assert_eq!(terminal_backspace_sequence(true), "\x1b\x7f");
    }

    #[test]
    fn synthetic_released_reaches_egui_only() {
        assert_eq!(
            keyboard_input_route(ElementState::Released, true),
            KeyboardInputRoute::EguiOnly
        );
    }

    #[test]
    fn real_pressed_routes_to_app_including_repeat_presses() {
        // winit 将真实重复按键继续报告为 Pressed，因此走同一个 App 路由。
        assert_eq!(
            keyboard_input_route(ElementState::Pressed, false),
            KeyboardInputRoute::App
        );
    }

    #[test]
    fn real_released_reaches_egui_only() {
        assert_eq!(
            keyboard_input_route(ElementState::Released, false),
            KeyboardInputRoute::EguiOnly
        );
    }

    #[test]
    fn successful_mutation_refreshes_its_own_side_only() {
        let success = sftp::SftpEvent::MutationFinished {
            tab_id: "tab".into(),
            side: sftp::FileSide::Remote,
            operation: sftp::FileOperation::Rename,
            result: Ok(()),
        };
        assert_eq!(
            refresh_side_for_event(&success),
            Some(sftp::FileSide::Remote)
        );

        let failure = sftp::SftpEvent::MutationFinished {
            tab_id: "tab".into(),
            side: sftp::FileSide::Local,
            operation: sftp::FileOperation::Delete,
            result: Err("denied".into()),
        };
        assert_eq!(refresh_side_for_event(&failure), None);
    }

    // --- P0 Task 3 RED-E: settings panel main-flow input isolation + apply plan ---

    /// 设置面板可见时必须阻断终端键入，即使 egui 未声明 wants_keyboard_input。
    #[test]
    fn settings_panel_visible_blocks_terminal_keyboard_without_egui_want() {
        assert!(
            terminal_input_blocked(true, false),
            "settings_panel.visible 必须进入 terminal_input_blocked"
        );
        assert!(
            !should_pass_keyboard_to_terminal(true, false, false, false),
            "设置面板可见且 egui 不抢键时仍不得把 KeyboardInput 传给终端"
        );
    }

    /// 已有 sidebar 弹窗 / egui wants 语义不得回归。
    #[test]
    fn sidebar_dialog_and_egui_wants_still_block_terminal_keyboard() {
        assert!(
            terminal_input_blocked(false, true),
            "侧栏弹窗必须继续阻断终端输入"
        );
        assert!(
            !should_pass_keyboard_to_terminal(false, true, false, false),
            "侧栏弹窗打开时不得穿透到终端"
        );
        assert!(
            !should_pass_keyboard_to_terminal(false, false, true, false),
            "egui wants_keyboard_input 时不得穿透到终端"
        );
        assert!(
            !terminal_input_blocked(false, false),
            "无设置面板且无侧栏弹窗时 terminal_input_blocked 应为 false"
        );
        assert!(
            should_pass_keyboard_to_terminal(false, false, false, false),
            "无遮罩且 egui 不抢键时应允许终端接收键盘"
        );
    }

    /// 主题与字体族/字号变化可独立识别，便于 Apply 时只做必要 GPU 更新。
    #[test]
    fn plan_settings_apply_identifies_theme_and_font_changes_independently() {
        let base = super::settings::Settings::default();

        let mut theme_only = base.clone();
        theme_only.terminal.color_scheme = "3024 Day".into();
        assert_eq!(
            plan_settings_apply(&base, &theme_only),
            SettingsApplyPlan {
                theme_changed: true,
                font_family_changed: false,
                font_size_changed: false,
                zmodem_changed: false,
            }
        );

        let mut family_only = base.clone();
        family_only.terminal.font = "Noto Sans Mono".into();
        assert_eq!(
            plan_settings_apply(&base, &family_only),
            SettingsApplyPlan {
                theme_changed: false,
                font_family_changed: true,
                font_size_changed: false,
                zmodem_changed: false,
            }
        );

        let mut size_only = base.clone();
        size_only.terminal.font_size = 18.0;
        assert_eq!(
            plan_settings_apply(&base, &size_only),
            SettingsApplyPlan {
                theme_changed: false,
                font_family_changed: false,
                font_size_changed: true,
                zmodem_changed: false,
            }
        );

        assert_eq!(
            plan_settings_apply(&base, &base),
            SettingsApplyPlan {
                theme_changed: false,
                font_family_changed: false,
                font_size_changed: false,
                zmodem_changed: false,
            }
        );

        let mut zmodem_only = base.clone();
        zmodem_only.zmodem.enabled = false;
        zmodem_only.zmodem.auto_detect = false;
        zmodem_only.zmodem.download_dir = "/var/tmp/zmodem-new".into();
        zmodem_only.zmodem.timeout_secs = 120;
        assert_eq!(
            plan_settings_apply(&base, &zmodem_only),
            SettingsApplyPlan {
                theme_changed: false,
                font_family_changed: false,
                font_size_changed: false,
                zmodem_changed: true,
            }
        );
    }

    // --- P0 Task 3 审查修复 RED: 指针事件 (MouseWheel/MouseInput) 穿透门控 ---

    /// 仅当设置面板与侧栏弹窗均关闭，且指针位于终端区域内时，才允许把指针事件传给终端。
    #[test]
    fn should_pass_pointer_to_terminal_only_when_unblocked_and_in_terminal() {
        assert!(
            should_pass_pointer_to_terminal(false, false, true),
            "无 modal 且 in_terminal 时应允许指针穿透（滚轮/鼠标）"
        );
        assert!(
            !should_pass_pointer_to_terminal(false, false, false),
            "指针不在终端内时不得穿透"
        );
        assert!(
            !should_pass_pointer_to_terminal(true, false, true),
            "设置面板可见时必须阻断指针穿透"
        );
        assert!(
            !should_pass_pointer_to_terminal(false, true, true),
            "侧栏弹窗打开时必须阻断指针穿透"
        );
        assert!(
            !should_pass_pointer_to_terminal(true, true, true),
            "双 modal 同时打开时必须阻断"
        );
        assert!(
            !should_pass_pointer_to_terminal(true, false, false),
            "设置面板 + 指针在终端外：仍阻断"
        );
        assert!(
            !should_pass_pointer_to_terminal(false, true, false),
            "侧栏弹窗 + 指针在终端外：仍阻断"
        );
        assert!(
            !should_pass_pointer_to_terminal(true, true, false),
            "双 modal + 指针在终端外：仍阻断"
        );
    }

    #[test]
    fn terminal_wheel_scrolls_seven_lines_per_physical_notch() {
        let mut wheel = TerminalWheelAccumulator::default();
        assert_eq!(
            wheel.scroll_lines(&MouseScrollDelta::LineDelta(0.0, 1.0)),
            7
        );
        assert_eq!(
            wheel.scroll_lines(&MouseScrollDelta::LineDelta(0.0, -2.0)),
            -14
        );
    }

    #[test]
    fn terminal_touchpad_keeps_pixel_proportional_scrolling() {
        let mut wheel = TerminalWheelAccumulator::default();
        assert_eq!(
            wheel.scroll_lines(&MouseScrollDelta::PixelDelta(
                winit::dpi::PhysicalPosition::new(0.0, 36.0)
            )),
            2
        );
        assert_eq!(
            wheel.scroll_lines(&MouseScrollDelta::PixelDelta(
                winit::dpi::PhysicalPosition::new(0.0, -17.0)
            )),
            0
        );
    }

    #[test]
    fn terminal_touchpad_accumulates_small_pixel_deltas_without_dropping_them() {
        let mut wheel = TerminalWheelAccumulator::default();
        let delta = MouseScrollDelta::PixelDelta(winit::dpi::PhysicalPosition::new(0.0, 6.0));

        assert_eq!(wheel.scroll_lines(&delta), 0);
        assert_eq!(wheel.scroll_lines(&delta), 0);
        assert_eq!(wheel.scroll_lines(&delta), 1);
    }

    // --- P0 Task 4 RED-D: search field focus suppresses terminal PTY input ---

    /// Search TextEdit owning focus must block terminal keyboard even when
    /// settings/sidebar are closed and egui has not yet reported wants_keyboard.
    /// GREEN extends `should_pass_keyboard_to_terminal` with a
    /// `search_field_owns_focus: bool` gate (defense-in-depth, same pattern as
    /// settings_panel_visible).
    #[test]
    fn search_field_owns_focus_suppresses_terminal_keyboard() {
        assert!(
            !should_pass_keyboard_to_terminal(false, false, false, true),
            "search field focus must suppress PTY keyboard input"
        );
        assert!(
            should_pass_keyboard_to_terminal(false, false, false, false),
            "no search focus and no other blockers → terminal may receive keys"
        );
        // Existing blockers still win regardless of search focus flag.
        assert!(
            !should_pass_keyboard_to_terminal(true, false, false, false),
            "settings panel still blocks"
        );
        assert!(
            !should_pass_keyboard_to_terminal(false, true, false, false),
            "sidebar dialog still blocks"
        );
        assert!(
            !should_pass_keyboard_to_terminal(false, false, true, false),
            "egui wants_keyboard still blocks"
        );
        assert!(
            !should_pass_keyboard_to_terminal(false, false, true, true),
            "search focus + egui wants still blocks"
        );
    }

    #[test]
    fn search_query_widget_id_is_stable_and_isolated_per_tab() {
        assert_eq!(
            super::terminal_search_query_id("tab-a"),
            super::terminal_search_query_id("tab-a")
        );
        assert_ne!(
            super::terminal_search_query_id("tab-a"),
            super::terminal_search_query_id("tab-b")
        );
    }

    #[test]
    fn search_keyboard_fallback_handles_navigation_only_when_search_owns_input() {
        assert_eq!(
            super::search_keyboard_fallback_action(
                &Key::Named(NamedKey::Enter),
                false,
                true,
                false,
            ),
            Some(super::terminal_search::SearchBarKeyAction::Next)
        );
        assert_eq!(
            super::search_keyboard_fallback_action(&Key::Named(NamedKey::Enter), true, true, false,),
            Some(super::terminal_search::SearchBarKeyAction::Previous)
        );
        assert_eq!(
            super::search_keyboard_fallback_action(
                &Key::Named(NamedKey::Escape),
                false,
                true,
                false,
            ),
            Some(super::terminal_search::SearchBarKeyAction::Close)
        );
        assert_eq!(
            super::search_keyboard_fallback_action(
                &Key::Named(NamedKey::Enter),
                false,
                false,
                false,
            ),
            None
        );
        assert_eq!(
            super::search_keyboard_fallback_action(
                &Key::Named(NamedKey::Escape),
                false,
                true,
                true,
            ),
            None
        );
    }

    /// Active-tab open path: open_search on the active tab's TerminalSearchState
    /// makes search visible; navigation target is available for reveal without
    /// any PTY write. Pure TabManager + terminal_search — no GPU/window.
    #[test]
    fn open_search_on_active_tab_is_visible_with_reveal_target_no_pty() {
        use super::terminal_search::{
            open_search, SearchBarEffect, SearchCell, SearchLine, SearchMatch,
        };

        let mut manager = super::TabManager::new();
        manager.new_ssh_placeholder(&placeholder_connection("inactive.example"));
        manager.new_ssh_placeholder(&placeholder_connection("active.example"));
        manager.switch_to(1);
        assert_eq!(manager.active_idx, 1);

        // Seed active tab query; inactive tab must stay hidden/empty.
        let lines = vec![SearchLine::new(
            -2,
            vec![SearchCell::primary(0, 'h'), SearchCell::primary(1, 'i')],
        )];
        manager.tabs[1].search.query = "hi".into();
        let effect = open_search(&mut manager.tabs[1].search, &lines);
        assert!(manager.tabs[1].search.visible);
        assert_eq!(manager.tabs[1].search.status_text(), "1/1");
        assert_eq!(manager.tabs[1].search.current, Some(0));
        let target = match effect {
            SearchBarEffect::Reveal(m) => m,
            SearchBarEffect::FocusQuery => manager.tabs[1]
                .search
                .current
                .and_then(|i| manager.tabs[1].search.matches.get(i).copied())
                .expect("FocusQuery open still exposes current match"),
            other => panic!("unexpected open effect: {other:?}"),
        };
        assert_eq!(
            target,
            SearchMatch {
                line: -2,
                start_col: 0,
                end_col: 2,
            }
        );
        // Reveal target is absolute line only — coordinator calls
        // terminal.reveal_search_line(target.line); never write_to_pty.
        let _reveal_line: i32 = target.line;

        assert!(
            !manager.tabs[0].search.visible,
            "opening search on active tab must not make inactive tab visible"
        );
        assert!(manager.tabs[0].search.query.is_empty());
        assert_eq!(
            manager.active().map(|t| t.search.visible),
            Some(true),
            "active tab search must be visible after open_search"
        );
    }

    // --- P0 Task 5 RED-B: IME owner routing, physical cursor area, reassert ---
    //
    // Pure helpers only (no Window/GPU). GREEN must add production free functions
    // + `PhysicalImeCursorArea` in main.rs. Completion popup is intentionally
    // NOT an Egui owner and is not a parameter of `resolve_ime_input_owner`.
    //
    // Contract:
    //   resolve_ime_input_owner(
    //       settings_panel_visible, has_sidebar_dialog, blocking_egui_overlay,
    //       search_owns_keyboard, egui_wants_keyboard,
    //   ) -> ime::InputOwner
    //     any true → Egui; all false → Terminal
    //
    //   logical_to_physical_ime_cursor_area(min_x, min_y, width, height, ppp)
    //     -> PhysicalImeCursorArea { x, y, width, height }  // physical f64
    //     scale min + size by ppp; invalid/nonpositive ppp → 1.0;
    //     physical width/height always > 0
    //
    //   should_reassert_terminal_ime(owner, window_focused) -> bool
    //     Terminal + focused → true; Egui or unfocused → false

    #[test]
    fn resolve_ime_input_owner_returns_egui_for_each_blocker() {
        use super::ime::InputOwner;

        assert_eq!(
            super::resolve_ime_input_owner(true, false, false, false, false),
            InputOwner::Egui,
            "settings modal must own IME as Egui"
        );
        assert_eq!(
            super::resolve_ime_input_owner(false, true, false, false, false),
            InputOwner::Egui,
            "sidebar modal must own IME as Egui"
        );
        assert_eq!(
            super::resolve_ime_input_owner(false, false, true, false, false),
            InputOwner::Egui,
            "blocking egui overlay must own IME as Egui"
        );
        assert_eq!(
            super::resolve_ime_input_owner(false, false, false, true, false),
            InputOwner::Egui,
            "search keyboard ownership must own IME as Egui"
        );
        assert_eq!(
            super::resolve_ime_input_owner(false, false, false, false, true),
            InputOwner::Egui,
            "egui wants_keyboard must own IME as Egui"
        );
        // Combined blockers still Egui.
        assert_eq!(
            super::resolve_ime_input_owner(true, true, true, true, true),
            InputOwner::Egui,
        );
    }

    #[test]
    fn resolve_ime_input_owner_returns_terminal_only_when_all_blockers_false() {
        use super::ime::InputOwner;

        assert_eq!(
            super::resolve_ime_input_owner(false, false, false, false, false),
            InputOwner::Terminal,
            "no settings/sidebar/overlay/search/egui-keyboard → Terminal owns IME"
        );
    }

    #[test]
    fn logical_to_physical_ime_cursor_area_scales_min_and_size() {
        let area: super::PhysicalImeCursorArea =
            super::logical_to_physical_ime_cursor_area(10.0, 20.0, 8.0, 16.0, 2.0);
        assert_eq!(area.x, 20.0);
        assert_eq!(area.y, 40.0);
        assert_eq!(area.width, 16.0);
        assert_eq!(area.height, 32.0);
    }

    #[test]
    fn hidpi_layout_hit_test_and_render_rect_use_explicit_spaces() {
        let bounds = super::logical_terminal_layout_rect(2000, 1200, 2.0, 100.0, 30.0, 20.0, 0.0);
        assert_eq!(
            bounds,
            egui::Rect::from_min_max(egui::pos2(100.0, 30.0), egui::pos2(1000.0, 580.0),)
        );

        let tree = super::split::PaneTree::new("only-pane".into());
        let layout = tree.layout(bounds);
        let physical_pointer = (800.0, 400.0);
        let logical_pointer = super::physical_to_egui_position(physical_pointer, 2.0);
        assert_eq!(
            layout
                .pane_at(logical_pointer)
                .map(|pane| pane.pane_id.as_str()),
            Some("only-pane"),
        );

        let physical = super::logical_to_physical_pane_rect(layout.panes[0].rect, 2.0);
        assert_eq!(
            physical,
            super::PaneRenderRect::new(200.0, 60.0, 1800.0, 1100.0),
        );
        assert_eq!(
            super::physical_to_logical_rect(
                egui::Rect::from_min_size(
                    egui::pos2(physical.x, physical.y),
                    egui::vec2(physical.width, physical.height),
                ),
                2.0,
            ),
            layout.panes[0].rect,
        );
    }

    #[test]
    fn scale_factor_change_plans_terminal_sync_and_render_without_resized() {
        assert_eq!(
            super::plan_window_geometry_update(super::WindowGeometryEventKind::ScaleFactorChanged),
            super::WindowGeometryUpdatePlan::SyncAndRender,
        );
        assert_eq!(
            super::plan_window_geometry_update(super::WindowGeometryEventKind::Resized),
            super::WindowGeometryUpdatePlan::ResizeSurfaceAndSyncAndRender,
        );
    }

    #[test]
    fn serial_split_menu_labels_make_the_unsupported_state_visible() {
        assert_eq!(
            super::terminal_split_menu_labels(false),
            ("水平分屏（串口不支持）", "垂直分屏（串口不支持）",),
        );
        assert_eq!(
            super::terminal_split_menu_labels(true),
            ("水平分屏", "垂直分屏"),
        );
    }

    #[test]
    fn terminal_ime_commit_accepts_direct_xim_commit_and_rejects_stale_composition() {
        let owner = super::TerminalImeIdentity {
            tab_id: "tab-a".into(),
            pane_id: "pane-a".into(),
        };
        let same = owner.clone();
        let other_pane = super::TerminalImeIdentity {
            tab_id: "tab-a".into(),
            pane_id: "pane-b".into(),
        };
        let other_tab = super::TerminalImeIdentity {
            tab_id: "tab-b".into(),
            pane_id: "pane-a".into(),
        };

        assert!(super::terminal_ime_commit_matches(
            Some(&owner),
            Some(&same),
        ));
        assert!(!super::terminal_ime_commit_matches(
            Some(&owner),
            Some(&other_pane),
        ));
        assert!(!super::terminal_ime_commit_matches(
            Some(&owner),
            Some(&other_tab),
        ));
        assert!(super::terminal_ime_commit_matches(None, Some(&same)));
        assert!(!super::terminal_ime_commit_matches(None, None));
        assert!(!super::terminal_ime_commit_matches(Some(&owner), None));
    }
