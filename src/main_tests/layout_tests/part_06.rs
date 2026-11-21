    #[test]
    fn terminal_preedit_blocks_only_keyboard_input_events() {
        use super::ime::InputOwner;
        use super::ImeOwnedInputKind;

        assert!(super::terminal_preedit_blocks_input(
            InputOwner::Terminal,
            true,
            ImeOwnedInputKind::Keyboard,
        ));
        assert!(!super::terminal_preedit_blocks_input(
            InputOwner::Terminal,
            true,
            ImeOwnedInputKind::ModifiersChanged,
        ));
        assert!(!super::terminal_preedit_blocks_input(
            InputOwner::Terminal,
            true,
            ImeOwnedInputKind::Ime,
        ));
        assert!(!super::terminal_preedit_blocks_input(
            InputOwner::Egui,
            true,
            ImeOwnedInputKind::Keyboard,
        ));
        assert!(!super::terminal_preedit_blocks_input(
            InputOwner::Terminal,
            false,
            ImeOwnedInputKind::Keyboard,
        ));
    }
