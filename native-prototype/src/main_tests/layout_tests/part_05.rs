    #[test]
    fn divider_drag_cannot_cross_tabs_even_when_split_ids_match() {
        let dragged = super::DraggedSplit {
            tab_id: "tab-a".into(),
            split_id: 1,
        };
        assert_eq!(
            super::active_dragged_split(Some(&dragged), Some("tab-a")),
            Some(1),
        );
        assert_eq!(
            super::active_dragged_split(Some(&dragged), Some("tab-b")),
            None,
        );
        assert_eq!(super::active_dragged_split(Some(&dragged), None), None);
    }

    #[test]
    fn logical_to_physical_ime_cursor_area_clamps_invalid_scale_to_one() {
        for bad_ppp in [0.0_f32, -2.0, f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            let area: super::PhysicalImeCursorArea =
                super::logical_to_physical_ime_cursor_area(10.0, 20.0, 8.0, 16.0, bad_ppp);
            assert_eq!(
                area.x, 10.0,
                "invalid ppp={bad_ppp} must clamp scale to 1.0"
            );
            assert_eq!(
                area.y, 20.0,
                "invalid ppp={bad_ppp} must clamp scale to 1.0"
            );
            assert_eq!(
                area.width, 8.0,
                "invalid ppp={bad_ppp} must clamp scale to 1.0"
            );
            assert_eq!(
                area.height, 16.0,
                "invalid ppp={bad_ppp} must clamp scale to 1.0"
            );
        }
    }

    #[test]
    fn logical_to_physical_ime_cursor_area_never_nonpositive_physical_size() {
        let zero: super::PhysicalImeCursorArea =
            super::logical_to_physical_ime_cursor_area(5.0, 6.0, 0.0, 0.0, 2.0);
        assert!(
            zero.width > 0.0,
            "physical width must be > 0 even for zero logical width"
        );
        assert!(
            zero.height > 0.0,
            "physical height must be > 0 even for zero logical height"
        );

        let negative: super::PhysicalImeCursorArea =
            super::logical_to_physical_ime_cursor_area(0.0, 0.0, -3.0, -4.0, 1.5);
        assert!(
            negative.width > 0.0,
            "physical width must be > 0 even for negative logical width"
        );
        assert!(
            negative.height > 0.0,
            "physical height must be > 0 even for negative logical height"
        );
    }

    #[test]
    fn should_reassert_terminal_ime_only_when_terminal_owner_and_focused() {
        use super::ime::InputOwner;

        assert!(
            super::should_reassert_terminal_ime(InputOwner::Terminal, true),
            "Terminal owner + focused window → allow terminal IME reassert"
        );
        assert!(
            !super::should_reassert_terminal_ime(InputOwner::Terminal, false),
            "Terminal owner + unfocused → must not reassert terminal IME"
        );
        assert!(
            !super::should_reassert_terminal_ime(InputOwner::Egui, true),
            "Egui owner + focused → must not override egui IME state"
        );
        assert!(
            !super::should_reassert_terminal_ime(InputOwner::Egui, false),
            "Egui owner + unfocused → must not reassert terminal IME"
        );
    }

    #[test]
    fn open_new_tab_selector_is_a_terminal_input_blocker() {
        assert!(
            super::blocking_dialog_visible(false, true),
            "an open new-tab selector must be treated as a blocking dialog"
        );
        assert!(
            super::terminal_input_blocked(false, super::blocking_dialog_visible(false, true),),
            "selector-only modal state must block terminal input"
        );
        assert!(
            !super::blocking_dialog_visible(false, false),
            "a closed selector must not steal terminal input"
        );
    }

    #[test]
    fn blocking_dialog_prevents_terminal_pointer_motion_until_closed() {
        use super::TerminalPointerMotionAction;

        assert_eq!(
            super::terminal_pointer_motion_action(
                false,
                super::blocking_dialog_visible(false, true),
            ),
            TerminalPointerMotionAction::BlockAndCancelGesture,
            "selector-open CursorMoved must neither report terminal mouse motion nor mutate selection",
        );
        assert_eq!(
            super::terminal_pointer_motion_action(false, false),
            TerminalPointerMotionAction::Process,
            "normal terminal CursorMoved must continue reporting or extending selection",
        );
    }

    #[test]
    fn open_selector_pressed_keyboard_input_requests_redraw() {
        assert_eq!(
            super::selector_keyboard_input_scheduling(
                true,
                super::KeyboardInputRoute::App,
                &Key::Named(NamedKey::Escape),
            ),
            Some(super::InputFrameScheduling::RequestRedraw),
            "Escape must schedule selector.show even when egui_winit reports not consumed",
        );
        assert_eq!(
            super::selector_keyboard_input_scheduling(
                true,
                super::KeyboardInputRoute::App,
                &Key::Character("x".into()),
            ),
            Some(super::InputFrameScheduling::RequestRedraw),
            "all real pressed selector keyboard input should schedule a frame",
        );
        assert_eq!(
            super::selector_keyboard_input_scheduling(
                false,
                super::KeyboardInputRoute::App,
                &Key::Named(NamedKey::Escape),
            ),
            None,
            "a closed selector must not add redraw ownership",
        );
    }

    #[test]
    fn open_rename_pressed_keyboard_input_requests_redraw() {
        for key in [
            Key::Named(NamedKey::Enter),
            Key::Named(NamedKey::Escape),
            Key::Character("x".into()),
        ] {
            assert_eq!(
                super::rename_keyboard_input_scheduling(true, super::KeyboardInputRoute::App, &key,),
                Some(super::InputFrameScheduling::RequestRedraw),
            );
        }
        assert_eq!(
            super::rename_keyboard_input_scheduling(
                false,
                super::KeyboardInputRoute::App,
                &Key::Named(NamedKey::Enter),
            ),
            None,
        );
        assert_eq!(
            super::rename_keyboard_input_scheduling(
                true,
                super::KeyboardInputRoute::EguiOnly,
                &Key::Named(NamedKey::Escape),
            ),
            None,
        );
    }

    #[test]
    fn ime_events_route_by_owner_without_leaking_egui_text_to_terminal_state() {
        use super::ime::{ImeAction, ImeState, InputOwner};
        use super::RoutedImeEvent;

        let mut ime = ImeState::default();
        assert_eq!(
            super::apply_routed_ime_event(
                &mut ime,
                InputOwner::Egui,
                RoutedImeEvent::Preedit("dialog".into(), Some((0, 6))),
            ),
            ImeAction::None,
        );
        assert_eq!(ime.preedit_text(), "");
        assert_eq!(
            super::apply_routed_ime_event(
                &mut ime,
                InputOwner::Egui,
                RoutedImeEvent::Commit("对话框".into()),
            ),
            ImeAction::Redraw,
        );

        assert_eq!(
            super::apply_routed_ime_event(&mut ime, InputOwner::Terminal, RoutedImeEvent::Enabled,),
            ImeAction::None,
        );
        assert!(ime.is_enabled(), "Enabled must call ImeState::on_enabled");
        assert_eq!(
            super::apply_routed_ime_event(
                &mut ime,
                InputOwner::Terminal,
                RoutedImeEvent::Preedit("zhong".into(), Some((0, 5))),
            ),
            ImeAction::Redraw,
        );
        assert_eq!(ime.preedit_text(), "zhong");
        assert_eq!(
            super::apply_routed_ime_event(
                &mut ime,
                InputOwner::Terminal,
                RoutedImeEvent::Commit("中".into()),
            ),
            ImeAction::Commit("中".into()),
        );
    }
