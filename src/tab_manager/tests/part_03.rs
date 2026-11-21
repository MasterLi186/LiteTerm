    #[test]
    fn serial_open_plan_and_failure_require_matching_generation_and_pane() {
        let mut manager = TabManager::new();
        let plan = manager.new_serial_placeholder(crate::serial::SerialSpec {
            device: "/dev/ttyTEST".into(),
            display_name: "测试串口".into(),
            serial_number: None,
            baud_rate: crate::serial::DEFAULT_BAUD_RATE,
        });

        assert_eq!(plan.tab_id, plan.pane_id);
        assert_eq!(plan.generation, 1);
        assert!(!manager.serial_failed(
            &plan.tab_id,
            &plan.pane_id,
            plan.generation + 1,
            "stale failure"
        ));
        assert!(!manager.serial_failed(
            &plan.tab_id,
            "missing-pane",
            plan.generation,
            "wrong pane"
        ));
        assert_eq!(manager.tabs[0].status, PaneStatus::Connecting);

        assert!(manager.serial_failed(&plan.tab_id, &plan.pane_id, plan.generation, "open failed"));
        assert_eq!(
            manager.tabs[0].status,
            PaneStatus::Failed("open failed".into())
        );
        assert_eq!(manager.tabs[0].label, "ttyTEST (打开失败)");
    }

    #[test]
    fn serial_reconnect_reuses_tab_rotates_generation_and_preserves_identity() {
        let mut manager = TabManager::new();
        let initial = manager.new_serial_placeholder(crate::serial::SerialSpec {
            device: "/dev/ttyUSB1".into(),
            display_name: "FT232R USB UART".into(),
            serial_number: Some("A10LCL3D".into()),
            baud_rate: crate::serial::DEFAULT_BAUD_RATE,
        });
        let old_session = manager.tabs[0].completion.session().clone();

        let reconnect = manager.reset_serial_for_reconnect(0).unwrap();

        assert_eq!(reconnect.open.tab_id, initial.tab_id);
        assert_eq!(reconnect.open.pane_id, initial.pane_id);
        assert_eq!(reconnect.open.generation, initial.generation + 1);
        assert_eq!(reconnect.open.spec.serial_number.as_deref(), Some("A10LCL3D"));
        assert_eq!(manager.tabs[0].serial_generation, initial.generation + 1);
        assert_eq!(manager.tabs[0].status, PaneStatus::Connecting);
        assert_eq!(manager.tabs[0].label, "ttyUSB1 · A10LCL3D (打开中...)");
        assert_ne!(manager.tabs[0].completion.session(), &old_session);
    }

    #[test]
    fn serial_disconnect_rotates_generation_and_leaves_reconnectable_placeholder() {
        let mut manager = TabManager::new();
        let initial = manager.new_serial_placeholder(crate::serial::SerialSpec {
            device: "/dev/ttyUSB2".into(),
            display_name: "FT232R USB UART".into(),
            serial_number: Some("A10LCQIM".into()),
            baud_rate: crate::serial::DEFAULT_BAUD_RATE,
        });
        manager.tabs[0].status = PaneStatus::Connected;
        let old_session = manager.tabs[0].completion.session().clone();

        let disconnected = manager.disconnect_serial(0).unwrap();

        assert_eq!(disconnected.pane_id, initial.pane_id);
        assert_eq!(manager.tabs[0].status, PaneStatus::Idle);
        assert_eq!(manager.tabs[0].serial_generation, initial.generation + 1);
        assert_eq!(manager.tabs[0].label, "ttyUSB2 · A10LCQIM (已断开)");
        assert_ne!(manager.tabs[0].completion.session(), &old_session);
        assert!(!manager.tabs[0].read_thread_started);
        assert!(manager.disconnect_serial(0).is_none());
    }
