    #[test]
    fn reconcile_actions_starts_one_worker_for_duplicate_connected_tabs() {
        let key = super::monitor::MonitorKey::remote("alice", "shared.example", 22);
        let params = remote_params("alice", "shared.example", 22);
        let mut required = HashMap::new();
        required.insert(key.clone(), params.clone());

        let actions = super::reconcile_actions(&required, &HashMap::new());

        assert_eq!(actions.starts, vec![(key, params)]);
        assert!(actions.stops.is_empty());
    }

    #[test]
    fn reconcile_actions_starts_distinct_remote_keys_separately() {
        let first_key = super::monitor::MonitorKey::remote("alice", "alpha.example", 22);
        let second_key = super::monitor::MonitorKey::remote("bob", "beta.example", 2200);
        let mut required = HashMap::new();
        required.insert(
            first_key.clone(),
            remote_params("alice", "alpha.example", 22),
        );
        required.insert(
            second_key.clone(),
            remote_params("bob", "beta.example", 2200),
        );

        let actions = super::reconcile_actions(&required, &HashMap::new());

        assert_eq!(actions.starts.len(), 2);
        assert_eq!(
            actions
                .starts
                .iter()
                .map(|(key, _)| key.clone())
                .collect::<HashSet<_>>(),
            HashSet::from([first_key, second_key])
        );
        assert!(actions.stops.is_empty());
    }

    #[test]
    fn reconcile_actions_stops_only_the_remote_without_a_remaining_reference() {
        let retained = super::monitor::MonitorKey::remote("alice", "alpha.example", 22);
        let removed = super::monitor::MonitorKey::remote("bob", "beta.example", 2200);
        let mut required = HashMap::new();
        required.insert(
            retained.clone(),
            remote_params("alice", "alpha.example", 22),
        );
        let running = HashMap::from([
            (retained, remote_params("alice", "alpha.example", 22)),
            (removed.clone(), remote_params("bob", "beta.example", 2200)),
        ]);

        let actions = super::reconcile_actions(&required, &running);

        assert!(actions.starts.is_empty());
        assert_eq!(actions.stops, vec![removed]);
    }

    #[test]
    fn reconcile_actions_preserves_workers_that_are_still_required() {
        let key = super::monitor::MonitorKey::remote("alice", "alpha.example", 22);
        let mut required = HashMap::new();
        required.insert(key.clone(), remote_params("alice", "alpha.example", 22));

        let actions = super::reconcile_actions(
            &required,
            &HashMap::from([(key, remote_params("alice", "alpha.example", 22))]),
        );

        assert!(actions.starts.is_empty());
        assert!(actions.stops.is_empty());
    }

    #[test]
    #[cfg(debug_assertions)]
    #[should_panic]
    fn tab_scoped_resource_install_rejects_worker_without_browser() {
        let mut browsers = HashMap::new();
        let session = CompletionSessionKey::new_for_test(1, "orphan");
        let (orphan_worker, _) =
            sftp::test_handle_for("orphan-tab", "orphan-pane", session.clone());
        let mut workers = HashMap::from([("orphan-pane".into(), orphan_worker)]);
        let (worker, _) = sftp::test_handle_for("new-tab", "new-pane", session);

        super::install_tab_scoped_resources(
            &mut browsers,
            &mut workers,
            "new-tab".into(),
            super::file_browser::FileBrowserState::new("/tmp".into()),
            worker,
        );
    }

    #[test]
    fn duplicate_tabs_share_monitor_but_keep_tab_resources_isolated_until_each_closes() {
        let mut manager = super::TabManager::new();
        let first_id = manager.new_ssh_placeholder(&placeholder_connection("shared.example"));
        let second_id = manager.new_ssh_placeholder(&placeholder_connection("shared.example"));
        manager.tabs[0].remote_monitor_leased = true;
        manager.tabs[1].remote_monitor_leased = true;
        let shared_key = manager.tabs[0].monitor_key();
        assert_eq!(manager.tabs[1].monitor_key(), shared_key);

        let mut browsers = HashMap::new();
        let mut workers = HashMap::new();
        let first_session = manager.tabs[0].completion.session().clone();
        let second_session = manager.tabs[1].completion.session().clone();
        let (first_worker, first_commands) =
            sftp::test_handle_for(&first_id, &first_id, first_session.clone());
        let (replacement_worker, replacement_commands) =
            sftp::test_handle_for(&first_id, &first_id, first_session);
        let (second_worker, second_commands) =
            sftp::test_handle_for(&second_id, &second_id, second_session);
        super::install_tab_scoped_resources(
            &mut browsers,
            &mut workers,
            first_id.clone(),
            super::file_browser::FileBrowserState::new("/tmp/a".into()),
            first_worker,
        );
        super::install_tab_scoped_resources(
            &mut browsers,
            &mut workers,
            first_id.clone(),
            super::file_browser::FileBrowserState::new("/tmp/a".into()),
            replacement_worker,
        );
        assert!(matches!(
            first_commands.recv_timeout(Duration::from_secs(1)),
            Ok(sftp::SftpCommand::Shutdown)
        ));
        super::install_tab_scoped_resources(
            &mut browsers,
            &mut workers,
            second_id.clone(),
            super::file_browser::FileBrowserState::new("/tmp/b".into()),
            second_worker,
        );

        let first = browsers.get_mut(&first_id).unwrap();
        let request_id = first.next_request(sftp::FileSide::Local, "/home/alice".into());
        first.apply_event(&sftp::SftpEvent::Listed {
            tab_id: first_id.clone(),
            request_id,
            side: sftp::FileSide::Local,
            path: "/home/alice".into(),
            result: Ok(Vec::new()),
        });
        first.apply_event(&sftp::SftpEvent::Ready {
            tab_id: first_id.clone(),
            home: "/srv/a".into(),
        });
        first.apply_event(&sftp::SftpEvent::Failed {
            tab_id: first_id.clone(),
            error: "A only".into(),
        });
        first.open = false;
        let second = browsers.get(&second_id).unwrap();
        assert_eq!(second.local.path, "/tmp/b");
        assert_eq!(second.local.request_id, 0);
        assert_eq!(second.remote.path, "/");
        assert_eq!(second.remote.error, None);
        assert!(second.open);
        assert!(workers.contains_key(&first_id));
        assert!(workers.contains_key(&second_id));

        let required = manager.remote_monitor_requirements();
        assert_eq!(required.len(), 1);
        assert!(required.contains_key(&shared_key));
        let running = required.clone();

        let after_first_close = super::close_tab_scoped_resources_and_plan(
            &mut manager,
            &mut browsers,
            &mut workers,
            &running,
            0,
        )
        .expect("第一个标签存在");
        assert!(matches!(
            replacement_commands.recv_timeout(Duration::from_secs(1)),
            Ok(sftp::SftpCommand::Shutdown)
        ));
        assert!(second_commands.try_recv().is_err());
        assert!(browsers.contains_key(&second_id));
        assert!(workers.contains_key(&second_id));
        assert_eq!(browsers.len(), workers.len());
        assert!(after_first_close.starts.is_empty());
        assert!(after_first_close.stops.is_empty());

        let after_last_close = super::close_tab_scoped_resources_and_plan(
            &mut manager,
            &mut browsers,
            &mut workers,
            &running,
            0,
        )
        .expect("最后一个标签存在");
        assert!(matches!(
            second_commands.recv_timeout(Duration::from_secs(1)),
            Ok(sftp::SftpCommand::Shutdown)
        ));
        assert_eq!(after_last_close.stops, vec![shared_key]);
        assert!(browsers.is_empty());
        assert!(workers.is_empty());
        assert_eq!(browsers.len(), workers.len());
    }

    #[test]
    fn closing_one_pane_shuts_down_only_its_sftp_worker() {
        let session_a = CompletionSessionKey::new_for_test(1, "pane-a");
        let session_b = CompletionSessionKey::new_for_test(1, "pane-b");
        let (worker_a, commands_a) = sftp::test_handle_for("tab", "pane-a", session_a);
        let (worker_b, commands_b) = sftp::test_handle_for("tab", "pane-b", session_b);
        let mut workers = HashMap::from([("pane-a".into(), worker_a), ("pane-b".into(), worker_b)]);

        super::shutdown_and_remove_pane_worker(&mut workers, "pane-a");

        assert!(matches!(
            commands_a.recv_timeout(Duration::from_secs(1)),
            Ok(sftp::SftpCommand::Shutdown)
        ));
        assert!(commands_b.try_recv().is_err());
        assert!(!workers.contains_key("pane-a"));
        assert!(workers.contains_key("pane-b"));
    }

    #[test]
    fn reconcile_actions_restarts_a_key_when_its_owner_params_change() {
        let key = super::monitor::MonitorKey::remote("alice", "alpha.example", 22);
        let mut old = remote_params("alice", "alpha.example", 22);
        old.key_path = "old-key-sentinel".into();
        old.password = "old-password-sentinel".into();
        let mut selected = remote_params("alice", "alpha.example", 22);
        selected.key_path = "selected-key-sentinel".into();
        selected.password = "selected-password-sentinel".into();
        let required = HashMap::from([(key.clone(), selected.clone())]);
        let running = HashMap::from([(key.clone(), old)]);

        let actions = super::reconcile_actions(&required, &running);

        assert_eq!(actions.stops, vec![key.clone()]);
        assert_eq!(actions.starts, vec![(key, selected)]);
        let debug = format!("{actions:?}");
        assert!(!debug.contains("old-key-sentinel"));
        assert!(!debug.contains("old-password-sentinel"));
        assert!(!debug.contains("selected-key-sentinel"));
        assert!(!debug.contains("selected-password-sentinel"));
    }

    #[test]
    fn remote_monitor_generations_skip_zero_when_wrapping() {
        let mut next = 0;
        assert_eq!(super::next_remote_monitor_generation(&mut next), 1);
        next = u64::MAX;
        assert_eq!(super::next_remote_monitor_generation(&mut next), 1);
    }

    fn monitor_data(name: &str) -> super::monitor::MonitorData {
        super::monitor::MonitorData {
            cpu_percent: 0.0,
            cpu_name: name.into(),
            memory_used: 0,
            memory_total: 0,
            memory_text: String::new(),
            memory_percent: 0.0,
            swap_used: 0,
            swap_total: 0,
            swap_text: String::new(),
            swap_percent: 0.0,
            uptime_text: String::new(),
            load_text: String::new(),
            disk_items: Vec::new(),
            processes: Vec::new(),
            zombie_processes: Vec::new(),
            process_stats: super::monitor::ProcessStats::default(),
            net_interfaces: Vec::new(),
            preferred_net_interface: None,
        }
    }

    fn process_detail(pid: u32) -> super::monitor::ProcessDetail {
        super::monitor::ProcessDetail {
            identity: super::monitor::ProcessIdentity {
                pid,
                start_ticks: 42,
            },
            user: "alice".into(),
            state: "S".into(),
            mem_mb: "1M".into(),
            mem_bytes: 1024 * 1024,
            platform_memory: None,
            cpu: 1.0,
            name: "sleep".into(),
            command: "sleep 30".into(),
            executable: "/usr/bin/sleep".into(),
            working_dir: "/tmp".into(),
            start_time: "Mon Jul 27 12:00:00 2026".into(),
            environ: Vec::new(),
            ancestors: Vec::new(),
        }
    }

    #[test]
    fn process_detail_event_requires_current_key_generation_requester_and_request() {
        let key = super::monitor::MonitorKey::remote("alice", "alpha.example", 22);
        let mut state = super::process_manager::ProcessManagerState::new(key.clone());
        let request_id = match state.select_process(41, "Mon Jul 27 12:00:00 2026") {
            super::process_manager::ProcessManagerAction::Select { request_id, .. } => request_id,
            _ => unreachable!(),
        };
        let mut states = HashMap::from([("process-tab".to_string(), state)]);
        let generations = HashMap::from([(key.clone(), 7)]);

        assert!(!super::apply_process_detail_event(
            &mut states,
            &generations,
            &key,
            6,
            "process-tab",
            request_id,
            Ok(Box::new(process_detail(41))),
        ));
        assert!(!super::apply_process_detail_event(
            &mut states,
            &generations,
            &key,
            7,
            "closed-tab",
            request_id,
            Ok(Box::new(process_detail(41))),
        ));
        assert!(!super::apply_process_detail_event(
            &mut states,
            &generations,
            &key,
            7,
            "process-tab",
            request_id + 1,
            Ok(Box::new(process_detail(41))),
        ));
        assert!(super::apply_process_detail_event(
            &mut states,
            &generations,
            &key,
            7,
            "process-tab",
            request_id,
            Ok(Box::new(process_detail(41))),
        ));
        assert_eq!(
            states
                .get("process-tab")
                .and_then(|state| state.detail())
                .map(|detail| detail.identity.pid),
            Some(41)
        );
    }

    #[test]
    fn monitor_generation_gate_accepts_only_current_remote_and_local_zero() {
        let remote = super::monitor::MonitorKey::remote("alice", "alpha.example", 22);
        let generations = HashMap::from([(remote.clone(), 4)]);

        assert!(super::monitor_event_is_current(
            &super::monitor::MonitorKey::Local,
            0,
            &generations
        ));
        assert!(!super::monitor_event_is_current(
            &super::monitor::MonitorKey::Local,
            1,
            &generations
        ));
        assert!(super::monitor_event_is_current(&remote, 4, &generations));
        assert!(!super::monitor_event_is_current(&remote, 3, &generations));
        assert!(!super::monitor_event_is_current(
            &super::monitor::MonitorKey::remote("bob", "missing.example", 22),
            4,
            &generations
        ));
    }

    #[test]
    fn stale_monitor_event_leaves_slot_unchanged() {
        let key = super::monitor::MonitorKey::remote("alice", "alpha.example", 22);
        let mut slots = HashMap::from([(
            key.clone(),
            super::monitor::MonitorSlot {
                data: Some(monitor_data("keep")),
                error: Some("keep error".into()),
            },
        )]);
        let event = super::monitor::MonitorEvent {
            key: key.clone(),
            generation: 3,
            result: Ok(Box::new(monitor_data("stale"))),
        };

        assert!(!super::apply_monitor_event(
            &mut slots,
            event,
            &HashMap::from([(key.clone(), 4)])
        ));
        let slot = slots.get(&key).unwrap();
        assert_eq!(slot.data.as_ref().unwrap().cpu_name, "keep");
        assert_eq!(slot.error.as_deref(), Some("keep error"));

        assert!(!super::apply_monitor_event(
            &mut slots,
            super::monitor::MonitorEvent {
                key: key.clone(),
                generation: 3,
                result: Err("stale-error-sentinel".into()),
            },
            &HashMap::from([(key.clone(), 4)])
        ));
        let slot = slots.get(&key).unwrap();
        assert_eq!(slot.data.as_ref().unwrap().cpu_name, "keep");
        assert_eq!(slot.error.as_deref(), Some("keep error"));
    }

    #[test]
    fn current_monitor_events_replace_data_clear_error_and_preserve_other_slots() {
        let key = super::monitor::MonitorKey::remote("alice", "alpha.example", 22);
        let other = super::monitor::MonitorKey::remote("bob", "beta.example", 22);
        let mut slots = HashMap::from([
            (
                key.clone(),
                super::monitor::MonitorSlot {
                    data: Some(monitor_data("old")),
                    error: Some("previous".into()),
                },
            ),
            (
                other.clone(),
                super::monitor::MonitorSlot {
                    data: Some(monitor_data("other")),
                    error: None,
                },
            ),
        ]);
        let generations = HashMap::from([(key.clone(), 4), (other.clone(), 9)]);

        assert!(super::apply_monitor_event(
            &mut slots,
            super::monitor::MonitorEvent {
                key: key.clone(),
                generation: 4,
                result: Ok(Box::new(monitor_data("new"))),
            },
            &generations
        ));
        let slot = slots.get(&key).unwrap();
        assert_eq!(slot.data.as_ref().unwrap().cpu_name, "new");
        assert_eq!(slot.error, None);
        assert_eq!(
            slots.get(&other).unwrap().data.as_ref().unwrap().cpu_name,
            "other"
        );
    }

    #[test]
    fn current_monitor_failure_retains_data_and_records_safe_error() {
        let key = super::monitor::MonitorKey::remote("alice", "alpha.example", 22);
        let mut slots = HashMap::from([(
            key.clone(),
            super::monitor::MonitorSlot {
                data: Some(monitor_data("keep")),
                error: None,
            },
        )]);

        assert!(super::apply_monitor_event(
            &mut slots,
            super::monitor::MonitorEvent {
                key: key.clone(),
                generation: 4,
                result: Err("unsafe\nerror\u{1b}[secret".repeat(32)),
            },
            &HashMap::from([(key.clone(), 4)])
        ));
        let slot = slots.get(&key).unwrap();
        assert_eq!(slot.data.as_ref().unwrap().cpu_name, "keep");
        assert!(slot.error.as_deref().is_some_and(|error| {
            !error.chars().any(char::is_control) && error.chars().count() <= 160
        }));
    }

    #[test]
    fn active_monitor_slot_uses_exact_key_without_local_fallback() {
        let slots = HashMap::from([(
            super::monitor::MonitorKey::Local,
            super::monitor::MonitorSlot {
                data: Some(monitor_data("local")),
                error: None,
            },
        )]);
        let missing = super::monitor::MonitorKey::remote("alice", "missing.example", 22);

        assert!(super::active_monitor_slot(&slots, &missing).is_none());
    }

    #[test]
    fn monitor_event_debug_and_remote_conversion_do_not_leak_payload() {
        let remote = super::remote_monitor::RemoteMonitorEvent::Failed {
            key: super::monitor::MonitorKey::remote("alice", "alpha.example", 22),
            generation: 7,
            error: "worker-error-sentinel".into(),
        };
        let event = super::user_event_from_remote(remote);
        let debug = format!("{event:?}");

        assert!(debug.contains("Monitor"));
        assert!(debug.contains("Err"));
        assert!(!debug.contains("worker-error-sentinel"));

        let update =
            super::user_event_from_remote(super::remote_monitor::RemoteMonitorEvent::Update {
                key: super::monitor::MonitorKey::Local,
                generation: 0,
                data: Box::new(monitor_data("payload-sentinel")),
            });
        assert!(matches!(update, UserEvent::Monitor(event) if event.result.is_ok()));
    }

    #[test]
    fn stopping_remote_monitor_removes_its_slot() {
        let remote = super::monitor::MonitorKey::remote("alice", "alpha.example", 22);
        let mut slots = HashMap::from([
            (
                super::monitor::MonitorKey::Local,
                super::monitor::MonitorSlot::default(),
            ),
            (remote.clone(), super::monitor::MonitorSlot::default()),
        ]);

        super::remove_monitor_slots(&mut slots, std::slice::from_ref(&remote));

        assert!(slots.contains_key(&super::monitor::MonitorKey::Local));
        assert!(!slots.contains_key(&remote));
    }

    #[test]
    fn main_monitor_event_path_routes_the_key_and_keeps_slot_error() {
        let key = super::monitor::MonitorKey::remote("alice", "alpha.example", 22);
        let generations = HashMap::from([(key.clone(), 7)]);
        let mut slots = HashMap::new();
        let mut sidebar = super::Sidebar::new();
        let mut data = monitor_data("remote");
        data.net_interfaces.push(super::monitor::NetIfaceInfo {
            name: "eth0".into(),
            rx_rate: 10,
            tx_rate: 20,
        });

        assert!(super::apply_monitor_event_and_update_sidebar(
            &mut slots,
            &mut sidebar,
            super::monitor::MonitorEvent {
                key: key.clone(),
                generation: 7,
                result: Ok(Box::new(data)),
            },
            &generations,
        ));
        assert!(sidebar.monitor_history_for_test(&key).is_some());

        assert!(super::apply_monitor_event_and_update_sidebar(
            &mut slots,
            &mut sidebar,
            super::monitor::MonitorEvent {
                key: key.clone(),
                generation: 7,
                result: Err("暂时断开".into()),
            },
            &generations,
        ));
        assert!(slots[&key].data.is_some());
        assert!(slots[&key].error.as_deref().is_some());
    }

    fn placeholder_connection(host: &str) -> super::sidebar::SshConnection {
        super::sidebar::SshConnection {
            label: host.into(),
            host: host.into(),
            port: 22,
            user: "alice".into(),
            auth: "key".into(),
            key_path: String::new(),
            password: String::new(),
            group: "test".into(),
            group_color: [0x58, 0xa6, 0xff],
        }
    }

    fn render_monitor_placeholder(sidebar: &mut super::Sidebar, key: &super::monitor::MonitorKey) {
        let context = egui::Context::default();
        let _ = context.run(egui::RawInput::default(), |context| {
            sidebar.ui_with_monitor(context, key, None, None);
        });
    }

    #[test]
    fn closing_last_placeholder_prunes_its_rendered_monitor_view() {
        let mut manager = super::TabManager::new();
        manager.new_ssh_placeholder(&placeholder_connection("alpha.example"));
        let key = manager.tabs[0].monitor_key();
        let mut sidebar = super::Sidebar::new();
        render_monitor_placeholder(&mut sidebar, &key);
        assert!(sidebar.has_monitor_view_for_test(&key));

        manager.close(0);
        super::prune_sidebar_monitor_views(&mut sidebar, &manager);

        assert!(!sidebar.has_monitor_view_for_test(&key));
    }

    #[test]
    fn pruning_placeholder_views_keeps_shared_keys_and_local_but_removes_only_distinct_target() {
        let mut manager = super::TabManager::new();
        manager.new_ssh_placeholder(&placeholder_connection("alpha.example"));
        manager.new_ssh_placeholder(&placeholder_connection("alpha.example"));
        manager.new_ssh_placeholder(&placeholder_connection("beta.example"));
        let shared = manager.tabs[0].monitor_key();
        let distinct = manager.tabs[2].monitor_key();
        let local = super::monitor::MonitorKey::Local;
        let mut sidebar = super::Sidebar::new();
        render_monitor_placeholder(&mut sidebar, &shared);
        render_monitor_placeholder(&mut sidebar, &distinct);
        render_monitor_placeholder(&mut sidebar, &local);

        manager.close_others(0);
        super::prune_sidebar_monitor_views(&mut sidebar, &manager);

        assert!(sidebar.has_monitor_view_for_test(&shared));
        assert!(!sidebar.has_monitor_view_for_test(&distinct));
        assert!(sidebar.has_monitor_view_for_test(&local));
    }

    #[test]
    fn remote_monitor_user_event_debug_redacts_worker_errors() {
        let event =
            super::user_event_from_remote(super::remote_monitor::RemoteMonitorEvent::Failed {
                key: super::monitor::MonitorKey::remote("alice", "alpha.example", 22),
                generation: 7,
                error: "worker-error-sentinel".into(),
            });

        let debug = format!("{event:?}");

        assert!(!debug.contains("worker-error-sentinel"));
    }

    #[test]
    fn mouse_tab_switch_is_deferred_until_after_present_and_requests_redraw() {
        assert_eq!(
            super::tab_action_frame_timing(&super::tab_bar::TabBarAction::SwitchTo(1)),
            FrameActionTiming::AfterPresent {
                request_redraw: true,
            }
        );
        assert_eq!(
            super::tab_action_frame_timing(&super::tab_bar::TabBarAction::None),
            FrameActionTiming::NoAction
        );
        assert_eq!(
            super::tab_action_frame_timing(&super::tab_bar::TabBarAction::Rename(1)),
            FrameActionTiming::AfterPresent {
                request_redraw: true,
            }
        );
    }

    #[test]
    fn right_mouse_physical_position_is_converted_to_egui_points() {
        let transition = super::terminal_context_menu_press_transition((642.0, 246.5), 2.0);

        assert_eq!(transition.position, egui::pos2(321.0, 123.25));
    }

    #[test]
    fn right_mouse_menu_transition_only_requests_a_future_redraw() {
        let transition = super::terminal_context_menu_press_transition((321.5, 123.25), 1.0);
        let mut visible = false;
        let mut position = egui::Pos2::ZERO;
        let mut ignore_pointer_press_once = false;
        let mut redraw_count = 0;

        super::apply_terminal_context_menu_transition(
            &mut visible,
            &mut position,
            &mut ignore_pointer_press_once,
            transition,
            || redraw_count += 1,
        );

        assert!(transition.visible);
        assert!(transition.ignore_pointer_press_once);
        assert_eq!(
            transition.frame_action,
            super::InputFrameScheduling::RequestRedraw
        );
        assert_ne!(
            transition.frame_action,
            super::InputFrameScheduling::RenderNow
        );
        assert!(visible);
        assert_eq!(position, egui::pos2(321.5, 123.25));
        assert!(ignore_pointer_press_once);
        assert_eq!(redraw_count, 1);
        assert_eq!(
            super::consumed_event_frame_scheduling(true, false, false),
            super::InputFrameScheduling::RequestRedraw
        );
        assert_eq!(
            super::consumed_event_frame_scheduling(false, false, false),
            super::InputFrameScheduling::RenderNow
        );
        assert_eq!(
            super::consumed_event_frame_scheduling(false, true, false),
            super::InputFrameScheduling::RequestRedraw
        );
        assert_eq!(
            super::consumed_event_frame_scheduling(false, false, true),
            super::InputFrameScheduling::RequestRedraw
        );
    }

    #[test]
    fn title_drag_starts_only_after_a_real_pointer_movement() {
        assert!(!super::window_drag_threshold_reached(
            (100.0, 100.0),
            (102.0, 102.0),
            3.0
        ));
        assert!(super::window_drag_threshold_reached(
            (100.0, 100.0),
            (104.0, 100.0),
            3.0
        ));
        assert!(!super::window_drag_threshold_reached(
            (f64::NAN, 0.0),
            (4.0, 0.0),
            3.0
        ));
    }

    #[test]
    fn ui_line_wheel_is_expanded_into_same_frame_point_events() {
        let modifiers = egui::Modifiers::NONE;
        let mut events = vec![egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Line,
            delta: egui::vec2(0.0, -1.0),
            modifiers,
        }];

        super::normalize_ui_wheel_events(&mut events);

        assert!(events.len() > 1);
        let total = events.iter().fold(egui::Vec2::ZERO, |sum, event| {
            let egui::Event::MouseWheel { unit, delta, .. } = event else {
                panic!("归一化结果只能包含滚轮事件");
            };
            assert_eq!(*unit, egui::MouseWheelUnit::Point);
            assert!(delta.abs().max_elem() <= 7.0);
            sum + *delta
        });
        assert!((total.y + 80.0).abs() < 0.01);
    }

    #[test]
    fn ctrl_or_command_f_is_the_primary_find_shortcut() {
        use winit::keyboard::{Key, ModifiersState};

        let key = Key::Character("f".into());
        assert!(super::is_primary_find_shortcut(&key, ModifiersState::CONTROL));
        assert!(super::is_primary_find_shortcut(&key, ModifiersState::SUPER));
        assert!(!super::is_primary_find_shortcut(
            &key,
            ModifiersState::CONTROL | ModifiersState::SHIFT
        ));
        assert!(!super::is_primary_find_shortcut(
            &Key::Character("g".into()),
            ModifiersState::CONTROL
        ));
    }

    #[test]
    #[should_panic(expected = "禁止同步渲染或 present")]
    fn terminal_menu_executor_rejects_render_now_and_present_work() {
        let mut transition = super::terminal_context_menu_press_transition((321.5, 123.25), 1.0);
        transition.frame_action = super::InputFrameScheduling::RenderNow;
        let mut visible = false;
        let mut position = egui::Pos2::ZERO;
        let mut ignore_pointer_press_once = false;

        super::apply_terminal_context_menu_transition(
            &mut visible,
            &mut position,
            &mut ignore_pointer_press_once,
            transition,
            || panic!("禁止请求 redraw"),
        );
    }
