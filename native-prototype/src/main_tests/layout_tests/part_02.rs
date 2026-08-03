    #[test]
    fn constrained_menu_rect_keeps_bottom_right_pointer_inside() {
        let ctx = egui::Context::default();
        let screen_rect = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 600.0));
        let unconstrained_requested_rect =
            egui::Rect::from_min_size(egui::pos2(790.0, 590.0), egui::vec2(160.0, 350.0));
        let mut actual_constrained_rect = None;
        let input = egui::RawInput {
            screen_rect: Some(screen_rect),
            ..Default::default()
        };
        let _ = ctx.run(input, |ctx| {
            actual_constrained_rect = egui::Window::new("constrained_terminal_menu_test")
                .title_bar(false)
                .fixed_pos(unconstrained_requested_rect.min)
                .fixed_size(unconstrained_requested_rect.size())
                .show(ctx, |ui| {
                    ui.allocate_space(egui::vec2(152.0, 342.0));
                })
                .map(|response| response.response.rect);
        });
        let actual_constrained_rect = actual_constrained_rect.unwrap();
        let pointer = actual_constrained_rect.center();

        assert!(screen_rect.contains_rect(actual_constrained_rect));
        assert!(!unconstrained_requested_rect.contains(pointer));
        assert!(!super::should_close_terminal_context_menu(
            false,
            true,
            Some(pointer),
            Some(actual_constrained_rect),
            false,
        ));
        assert!(super::should_close_terminal_context_menu(
            false,
            true,
            Some(pointer),
            Some(unconstrained_requested_rect),
            false,
        ));
    }

    #[test]
    fn initial_right_press_does_not_immediately_close_new_menu() {
        let menu_rect =
            egui::Rect::from_min_size(egui::pos2(100.0, 100.0), egui::vec2(160.0, 350.0));
        let original_press = egui::pos2(790.0, 590.0);

        assert!(!super::should_close_terminal_context_menu(
            false,
            true,
            Some(original_press),
            Some(menu_rect),
            true,
        ));
        assert!(super::should_close_terminal_context_menu(
            false,
            true,
            Some(original_press),
            Some(menu_rect),
            false,
        ));
    }

    #[test]
    fn open_menu_captures_left_and_middle_but_allows_right_click_repositioning() {
        for button in [
            winit::event::MouseButton::Left,
            winit::event::MouseButton::Middle,
        ] {
            for mouse_mode in [false, true] {
                assert_eq!(
                    super::open_terminal_menu_mouse_press_gate(
                        true,
                        ElementState::Pressed,
                        button,
                        mouse_mode,
                    ),
                    Some(super::InputFrameScheduling::RequestRedraw),
                );
            }
        }
        for mouse_mode in [false, true] {
            assert_eq!(
                super::open_terminal_menu_mouse_press_gate(
                    true,
                    ElementState::Pressed,
                    winit::event::MouseButton::Right,
                    mouse_mode,
                ),
                None,
            );
        }
        assert_eq!(
            super::open_terminal_menu_mouse_press_gate(
                true,
                ElementState::Released,
                winit::event::MouseButton::Left,
                true,
            ),
            None,
        );
        assert_eq!(
            super::open_terminal_menu_mouse_press_gate(
                false,
                ElementState::Pressed,
                winit::event::MouseButton::Left,
                true,
            ),
            None,
        );
    }

    #[test]
    fn same_frame_file_dialog_overlay_blocks_a_cached_completion_popup() {
        let ctx = egui::Context::default();
        let mut may_render = true;
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(800.0, 600.0),
            )),
            ..Default::default()
        };

        let _ = ctx.run(input, |ctx| {
            egui::Area::new(egui::Id::new("file_delete_backdrop"))
                .order(egui::Order::Foreground)
                .fixed_pos(egui::Pos2::ZERO)
                .show(ctx, |ui| {
                    ui.allocate_exact_size(egui::vec2(800.0, 600.0), egui::Sense::click());
                });
            may_render = completion_popup_may_render(ctx, true, false);
        });

        assert!(!may_render);
    }

    #[test]
    fn newly_opened_sidebar_dialog_blocks_a_cached_completion_popup() {
        let ctx = egui::Context::default();
        let mut may_render = true;

        let _ = ctx.run(Default::default(), |ctx| {
            may_render = completion_popup_may_render(ctx, true, true);
        });

        assert!(!may_render);
    }

    #[test]
    fn mouse_report_invalidates_authenticated_prompt_and_clears_candidates() {
        let session = CompletionSessionKey::new_for_test(1, "0123456789abcdef0123456789abcdef");
        let mut terminal =
            super::TerminalState::authenticated_prompt_with_input_for_test(session.clone(), "ls");
        let mut completion = super::smart_completion::CompletionState::new(session);
        completion.replace_history(vec!["ls -al".into()]);
        completion.refresh("ls");
        assert_eq!(terminal.current_bash_input().as_deref(), Some("ls"));
        assert!(completion.is_popup_visible());

        write_completion_invalidating_control_sequence(
            &mut completion,
            &mut terminal,
            "\x1b[<0;1;1M",
        );

        assert_eq!(terminal.current_bash_input(), None);
        assert!(completion.candidates().is_empty());
    }

    #[test]
    fn arrows_are_only_captured_for_a_visible_popup() {
        assert_eq!(
            completion_key_action(
                &Key::Named(NamedKey::ArrowDown),
                ModifiersState::empty(),
                true,
                false,
                false,
                false,
                false,
            ),
            CompletionKeyAction::Next
        );
        assert_eq!(
            completion_key_action(
                &Key::Named(NamedKey::ArrowUp),
                ModifiersState::empty(),
                true,
                false,
                false,
                false,
                false,
            ),
            CompletionKeyAction::Previous
        );
    }

    #[test]
    fn tab_accepts_visible_completion_while_enter_submits_current_input() {
        assert_eq!(
            completion_key_action(
                &Key::Named(NamedKey::Tab),
                ModifiersState::empty(),
                true,
                false,
                false,
                false,
                false,
            ),
            CompletionKeyAction::Accept
        );
        assert_eq!(
            completion_key_action(
                &Key::Named(NamedKey::Enter),
                ModifiersState::empty(),
                true,
                false,
                false,
                false,
                false,
            ),
            CompletionKeyAction::PassThrough
        );
        assert_eq!(
            completion_key_action(
                &Key::Named(NamedKey::Tab),
                ModifiersState::SHIFT,
                true,
                false,
                false,
                false,
                false,
            ),
            CompletionKeyAction::PassThrough
        );
        assert_eq!(
            completion_key_action(
                &Key::Named(NamedKey::Tab),
                ModifiersState::empty(),
                false,
                false,
                false,
                false,
                false,
            ),
            CompletionKeyAction::PassThrough
        );
    }

    #[test]
    fn completion_offscreen_logical_candidates_do_not_capture_navigation_or_enter() {
        let snapshot = super::completion_popup::CompletionPopupSnapshot::new(
            "tab-a".into(),
            CompletionSessionKey::new_for_test(1, "session"),
            false,
            egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 600.0)),
            Some(egui::Rect::from_min_size(
                egui::pos2(900.0, 10.0),
                egui::vec2(10.0, 20.0),
            )),
            vec!["git status".into()],
            0,
        );
        assert!(snapshot.is_none());

        for key in [
            Key::Named(NamedKey::ArrowUp),
            Key::Named(NamedKey::ArrowDown),
            Key::Named(NamedKey::Enter),
        ] {
            assert_eq!(
                completion_key_action(
                    &key,
                    ModifiersState::empty(),
                    snapshot.is_some(),
                    false,
                    false,
                    false,
                    false,
                ),
                CompletionKeyAction::PassThrough,
            );
        }
    }

    #[test]
    fn completion_rendered_snapshot_requires_active_tab_pane_and_session_identity() {
        let mut manager = super::TabManager::new();
        let (tab_id, _) = manager.new_local("bash", 80, 24);
        let session = manager.tabs[0].completion.session().clone();
        let snapshot = super::completion_popup::CompletionPopupSnapshot::new(
            tab_id,
            session.clone(),
            false,
            egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 600.0)),
            Some(egui::Rect::from_min_size(
                egui::pos2(10.0, 10.0),
                egui::vec2(10.0, 20.0),
            )),
            vec!["git status".into()],
            0,
        );
        assert!(super::current_completion_popup_snapshot(&manager, &snapshot).is_some());

        let mut stale_session = snapshot.clone().unwrap();
        stale_session.session =
            CompletionSessionKey::new_for_test(session.generation, "stale-session");
        assert!(super::current_completion_popup_snapshot(&manager, &Some(stale_session)).is_none());

        let mut stale_tab = snapshot.unwrap();
        stale_tab.tab_id = "other-tab".into();
        assert!(super::current_completion_popup_snapshot(&manager, &Some(stale_tab)).is_none());

        let stale_pane = super::completion_popup::CompletionPopupSnapshot::new_for_pane(
            manager.tabs[0].id.clone(),
            "other-pane".into(),
            session,
            false,
            egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 600.0)),
            Some(egui::Rect::from_min_size(
                egui::pos2(10.0, 10.0),
                egui::vec2(10.0, 20.0),
            )),
            vec!["git status".into()],
            0,
        );
        assert!(super::current_completion_popup_snapshot(&manager, &stale_pane).is_none());
    }

    #[test]
    fn completion_throttled_redraw_invalidates_cached_snapshot() {
        let session = CompletionSessionKey::new_for_test(1, "session");
        let mut snapshot = super::completion_popup::CompletionPopupSnapshot::new(
            "tab-a".into(),
            session,
            false,
            egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 600.0)),
            Some(egui::Rect::from_min_size(
                egui::pos2(10.0, 10.0),
                egui::vec2(10.0, 20.0),
            )),
            vec!["git status".into()],
            0,
        );
        let last_render = Instant::now();
        let mut epoch = 7;

        assert_eq!(
            super::prepare_completion_redraw(&mut snapshot, &mut epoch, last_render, last_render,),
            super::CompletionRedrawSchedule::RequestRedraw,
        );
        assert!(snapshot.is_none());
        assert_eq!(epoch, 8);
    }

    #[test]
    fn completion_redraw_after_throttle_window_renders_now() {
        let mut snapshot = None;
        let mut epoch = 2;
        let last_render = Instant::now();

        assert_eq!(
            super::prepare_completion_redraw(
                &mut snapshot,
                &mut epoch,
                last_render,
                last_render + Duration::from_millis(16),
            ),
            super::CompletionRedrawSchedule::RenderNow,
        );
        assert_eq!(epoch, 3);
    }

    #[test]
    fn completion_snapshot_is_published_only_after_present_and_matching_epoch() {
        let session = CompletionSessionKey::new_for_test(1, "session");
        let candidate = super::completion_popup::CompletionPopupSnapshot::new(
            "tab-a".into(),
            session,
            false,
            egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 600.0)),
            Some(egui::Rect::from_min_size(
                egui::pos2(10.0, 10.0),
                egui::vec2(10.0, 20.0),
            )),
            vec!["git status".into()],
            0,
        );
        let mut stored = candidate.clone();

        super::publish_completion_popup_snapshot(&mut stored, candidate.clone(), false, 4, 4);
        assert!(stored.is_none());

        let mut invalidated = candidate.clone();
        super::publish_completion_popup_snapshot(&mut invalidated, candidate.clone(), true, 4, 5);
        assert!(invalidated.is_none());

        super::publish_completion_popup_snapshot(&mut stored, candidate, true, 4, 4);
        assert!(stored.is_some());
    }

    #[test]
    fn completion_enter_selection_comes_from_persisted_snapshot_window() {
        let snapshot = super::completion_popup::CompletionPopupSnapshot::new(
            "tab-a".into(),
            CompletionSessionKey::new_for_test(1, "session"),
            false,
            egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(
                    400.0,
                    super::completion_popup::ROW_HEIGHT * 2.0
                        + super::completion_popup::POPUP_MARGIN * 2.0,
                ),
            ),
            Some(egui::Rect::from_min_size(
                egui::pos2(2.0, 2.0),
                egui::vec2(2.0, 2.0),
            )),
            vec!["one".into(), "two".into(), "three".into()],
            2,
        )
        .unwrap();

        assert_eq!(
            super::completion_snapshot_selection(&snapshot),
            Some(("tab-a".into(), "tab-a".into(), "three".into()))
        );
    }

    #[test]
    fn pending_fill_enter_still_submits_the_current_prefix() {
        assert_eq!(
            completion_key_action(
                &Key::Named(NamedKey::Enter),
                ModifiersState::empty(),
                false,
                true,
                false,
                false,
                false,
            ),
            CompletionKeyAction::PassThrough
        );
    }

    #[test]
    fn only_plain_tab_is_captured_when_completion_popup_is_visible() {
        assert_eq!(
            completion_key_action(
                &Key::Character(" ".into()),
                ModifiersState::empty(),
                true,
                false,
                false,
                false,
                false,
            ),
            CompletionKeyAction::PassThrough
        );
        assert_eq!(
            completion_key_action(
                &Key::Named(NamedKey::Tab),
                ModifiersState::empty(),
                true,
                false,
                false,
                false,
                false,
            ),
            CompletionKeyAction::Accept
        );
        assert_eq!(
            completion_key_action(
                &Key::Named(NamedKey::Tab),
                ModifiersState::CONTROL,
                true,
                false,
                false,
                false,
                false,
            ),
            CompletionKeyAction::PassThrough
        );
        assert_eq!(
            completion_key_action(
                &Key::Named(NamedKey::Enter),
                ModifiersState::empty(),
                true,
                false,
                true,
                false,
                false,
            ),
            CompletionKeyAction::PassThrough
        );
        assert_eq!(
            completion_key_action(
                &Key::Named(NamedKey::Escape),
                ModifiersState::empty(),
                true,
                false,
                false,
                true,
                false,
            ),
            CompletionKeyAction::PassThrough
        );
        for modifiers in [
            ModifiersState::CONTROL,
            ModifiersState::ALT,
            ModifiersState::SUPER,
        ] {
            assert_eq!(
                completion_key_action(
                    &Key::Named(NamedKey::Enter),
                    modifiers,
                    true,
                    false,
                    false,
                    false,
                    false,
                ),
                CompletionKeyAction::PassThrough
            );
        }
    }

    #[test]
    fn search_focus_prevents_completion_popup_from_capturing_navigation_keys() {
        assert_eq!(
            completion_key_action(
                &Key::Named(NamedKey::Enter),
                ModifiersState::empty(),
                true,
                false,
                false,
                false,
                true,
            ),
            CompletionKeyAction::PassThrough
        );
        assert_eq!(
            completion_key_action(
                &Key::Named(NamedKey::Escape),
                ModifiersState::empty(),
                true,
                false,
                false,
                false,
                true,
            ),
            CompletionKeyAction::PassThrough
        );
    }

    #[test]
    fn completion_exact_edits_are_printable_backspace_and_ctrl_u() {
        for input in ["ls -al", "你好", "\x7f", "\x08", "\x15"] {
            assert_eq!(
                completion_user_input_effect(input),
                CompletionUserInputEffect::ExactTrackedEdit,
                "{input:?}"
            );
        }
    }

    #[test]
    fn completion_known_readline_edits_are_recoverable_but_ambiguous() {
        for input in [
            "\t", "\x01", "\x02", "\x05", "\x06", "\x0b", "\x0e", "\x10", "\x14", "\x17", "\x19",
            "\x1b\x7f", "\x1b[A", "\x1b[B", "\x1b[C", "\x1b[D", "\x1b[H", "\x1b[F", "\x1b[3~",
        ] {
            assert_eq!(
                completion_user_input_effect(input),
                CompletionUserInputEffect::RecoverableReadlineEdit,
                "{input:?}"
            );
        }
    }

    #[test]
    fn completion_submission_and_unknown_controls_invalidate_prompt() {
        for input in [
            "\x03",
            "\x04",
            "\x0a",
            "\x0d",
            "\n",
            "echo one\n",
            "echo one\r",
            "\x1b",
            "\x1bOP",
            "\0",
        ] {
            assert_eq!(
                completion_user_input_effect(input),
                CompletionUserInputEffect::InvalidatePrompt,
                "{input:?}"
            );
        }
    }

    #[test]
    fn completion_recoverable_edit_preserves_prompt_but_invalidating_control_clears_it() {
        let session = CompletionSessionKey::new_for_test(4, "current");
        let mut completion = super::smart_completion::CompletionState::new(session.clone());
        completion.replace_history(vec!["git status".into()]);
        completion.track_user_input("git");
        completion.refresh("git");
        assert!(completion.is_popup_visible());
        let mut popup_snapshot = None;
        let mut terminal = super::terminal::TerminalState::authenticated_prompt_with_input_for_test(
            session, "git",
        );

        let recoverable = super::apply_completion_user_input_state(
            &mut completion,
            &mut popup_snapshot,
            "\x1b[D",
        );
        assert_eq!(
            recoverable,
            CompletionUserInputEffect::RecoverableReadlineEdit
        );
        assert_eq!(completion.tracked_input(), None);
        assert!(completion.candidates().is_empty());
        super::apply_completion_prompt_effect(&mut terminal, recoverable);
        assert!(terminal.has_authenticated_active_bash_prompt());

        let invalidating =
            super::apply_completion_user_input_state(&mut completion, &mut popup_snapshot, "\x03");
        assert_eq!(invalidating, CompletionUserInputEffect::InvalidatePrompt);
        super::apply_completion_prompt_effect(&mut terminal, invalidating);
        assert!(!terminal.has_authenticated_active_bash_prompt());
    }

    #[test]
    fn completion_ctrl_j_and_ctrl_m_are_submission_actions() {
        assert_eq!(
            super::ctrl_terminal_input_action(0x0a),
            super::CtrlTerminalInputAction::Submit
        );
        assert_eq!(
            super::ctrl_terminal_input_action(0x0d),
            super::CtrlTerminalInputAction::Submit
        );
        assert_eq!(
            super::ctrl_terminal_input_action(0x03),
            super::CtrlTerminalInputAction::Write('\x03')
        );
    }

    #[test]
    fn tracked_input_is_used_only_for_an_authenticated_active_prompt() {
        let session = CompletionSessionKey::new_for_test(4, "current");
        let mut completion = super::smart_completion::CompletionState::new(session.clone());
        completion.track_user_input("tracked");
        let mut terminal = super::terminal::TerminalState::authenticated_prompt_with_input_for_test(
            session, "grid",
        );

        assert_eq!(
            super::completion_input_for_render(&mut completion, &mut terminal, Instant::now()),
            Some("tracked".into())
        );
        terminal.take_bash_submission();
        assert_eq!(
            super::completion_input_for_render(&mut completion, &mut terminal, Instant::now()),
            None
        );
    }

    #[test]
    fn tracked_input_remains_available_in_adb_but_is_hidden_in_fish() {
        let session = CompletionSessionKey::new_for_test(4, "nested");
        let mut completion = super::smart_completion::CompletionState::new(session.clone());
        let mut terminal =
            super::terminal::TerminalState::authenticated_prompt_with_input_for_test(session, "");

        terminal.take_bash_submission();
        completion.observe_submission(Some("adb -s SERIAL shell"), true);
        completion.track_user_input("free");
        assert_eq!(
            super::completion_input_for_render(&mut completion, &mut terminal, Instant::now()),
            Some("free".into())
        );

        completion.complete_submission(None);
        completion.track_user_input("fish");
        let submission = completion.complete_submission(None);
        completion.observe_submission(submission.as_deref(), false);
        completion.track_user_input("git");
        assert_eq!(
            super::completion_input_for_render(&mut completion, &mut terminal, Instant::now()),
            None
        );
    }

    #[test]
    fn completion_event_matches_tab_generation_and_token() {
        let current = CompletionSessionKey::new_for_test(4, "current");
        assert!(super::completion_event_is_current(
            &current,
            &CompletionSessionKey::new_for_test(4, "current"),
        ));
        assert!(!super::completion_event_is_current(
            &current,
            &CompletionSessionKey::new_for_test(3, "current"),
        ));
        assert!(!super::completion_event_is_current(
            &current,
            &CompletionSessionKey::new_for_test(4, "old"),
        ));
    }

    #[test]
    fn completion_fill_gate_requires_success_and_exact_identity() {
        let current = CompletionSessionKey::new_for_test(4, "current");
        let mut state = super::smart_completion::CompletionState::new(current.clone());
        state.begin_fill(9, "git status");

        assert!(super::completion_fill_may_commit(
            &state,
            &current,
            &current,
            9,
            &Ok(()),
        ));
        assert!(!super::completion_fill_may_commit(
            &state,
            &current,
            &current,
            8,
            &Ok(()),
        ));
        assert!(!super::completion_fill_may_commit(
            &state,
            &current,
            &CompletionSessionKey::new_for_test(3, "current"),
            9,
            &Ok(()),
        ));
        assert!(!super::completion_fill_may_commit(
            &state,
            &current,
            &CompletionSessionKey::new_for_test(4, "stale"),
            9,
            &Ok(()),
        ));
        assert!(!super::completion_fill_may_commit(
            &state,
            &current,
            &current,
            9,
            &Err("write failed".into()),
        ));
    }
