    #[test]
    fn completion_ordinary_input_invalidates_snapshot_and_cancels_pending_fill() {
        let session = CompletionSessionKey::new_for_test(4, "current");
        let mut state = super::smart_completion::CompletionState::new(session.clone());
        state.replace_history(vec!["git status".into()]);
        state.refresh("git");
        assert!(state.begin_fill(9, "git status"));
        let mut snapshot = super::completion_popup::CompletionPopupSnapshot::new(
            "tab-1".into(),
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

        super::apply_completion_user_input_state(&mut state, &mut snapshot, "x");

        assert!(snapshot.is_none());
        assert!(!super::completion_fill_may_commit(
            &state,
            &session,
            &session,
            9,
            &Ok(()),
        ));
    }

    #[test]
    fn completion_new_ssh_action_closes_selector_and_opens_empty_existing_editor() {
        let mut selector = super::new_tab_selector::NewTabSelector::new();
        selector.open();
        let mut sidebar = super::Sidebar::new();
        sidebar.new_conn.label = "stale".into();

        super::open_new_ssh_editor(&mut selector, &mut sidebar);

        assert!(!selector.is_open());
        assert!(sidebar.show_new_connection);
        assert!(sidebar.new_conn.label.is_empty());
        assert!(sidebar.new_conn.host.is_empty());
    }

    #[test]
    fn worker_envelope_rejects_replaced_worker_before_browser_state_changes() {
        let mut manager = super::TabManager::new();
        let (tab_id, _) = manager.new_local("bash", 80, 24);
        let current = manager.tabs[0].completion.session().clone();
        let mut browsers = HashMap::new();
        let mut workers = HashMap::new();
        let (old_worker, old_commands) = sftp::test_handle_for(&tab_id, &tab_id, current.clone());
        let old_worker_id = old_worker.id();
        let (current_worker, _current_commands) =
            sftp::test_handle_for(&tab_id, &tab_id, current.clone());
        let current_worker_id = current_worker.id();
        super::install_tab_scoped_resources(
            &mut browsers,
            &mut workers,
            tab_id.clone(),
            super::file_browser::FileBrowserState::new("/tmp/old".into()),
            old_worker,
        );
        super::install_tab_scoped_resources(
            &mut browsers,
            &mut workers,
            tab_id.clone(),
            super::file_browser::FileBrowserState::new("/tmp/current".into()),
            current_worker,
        );
        assert!(matches!(
            old_commands.recv_timeout(Duration::from_secs(1)),
            Ok(sftp::SftpCommand::Shutdown)
        ));

        let request_id = browsers
            .get_mut(&tab_id)
            .unwrap()
            .next_request(sftp::FileSide::Remote, "/stale".into());
        for event in [
            sftp::SftpEvent::Listed {
                tab_id: tab_id.clone(),
                request_id,
                side: sftp::FileSide::Remote,
                path: "/stale".into(),
                result: Ok(vec![sftp::FileEntry {
                    name: "stale".into(),
                    path: "/stale/stale".into(),
                    is_dir: false,
                    size: 1,
                    mtime: 1,
                }]),
            },
            sftp::SftpEvent::Ready {
                tab_id: tab_id.clone(),
                home: "/stale-home".into(),
            },
            sftp::SftpEvent::Failed {
                tab_id: tab_id.clone(),
                error: "stale failure".into(),
            },
        ] {
            let accepted = super::take_current_sftp_worker_event(
                &manager,
                &workers,
                sftp::SftpWorkerEvent {
                    worker_id: old_worker_id,
                    tab_id: tab_id.clone(),
                    pane_id: tab_id.clone(),
                    session: current.clone(),
                    event,
                },
            );
            assert!(accepted.is_none());
        }
        let browser = browsers.get(&tab_id).unwrap();
        assert!(!browser.ready);
        assert!(browser.remote.entries.is_empty());
        assert_eq!(browser.remote.error, None);

        let accepted = super::take_current_sftp_worker_event(
            &manager,
            &workers,
            sftp::SftpWorkerEvent {
                worker_id: current_worker_id,
                tab_id: tab_id.clone(),
                pane_id: tab_id.clone(),
                session: current.clone(),
                event: sftp::SftpEvent::Ready {
                    tab_id: tab_id.clone(),
                    home: "/current-home".into(),
                },
            },
        )
        .expect("当前 worker 的同会话事件应被接受");
        browsers
            .get_mut(&accepted.0)
            .unwrap()
            .apply_event(&accepted.2);
        assert!(browsers.get(&tab_id).unwrap().ready);
        assert_eq!(browsers.get(&tab_id).unwrap().remote.path, "/current-home");

        assert!(super::take_current_sftp_worker_event(
            &manager,
            &workers,
            sftp::SftpWorkerEvent {
                worker_id: current_worker_id,
                tab_id: tab_id.clone(),
                pane_id: tab_id.clone(),
                session: CompletionSessionKey::new_for_test(current.generation, "old"),
                event: sftp::SftpEvent::Failed {
                    tab_id: tab_id.clone(),
                    error: "old session".into(),
                },
            },
        )
        .is_none());
        assert!(super::take_current_sftp_worker_event(
            &manager,
            &workers,
            sftp::SftpWorkerEvent {
                worker_id: current_worker_id,
                tab_id: "closed-tab".into(),
                pane_id: "closed-tab".into(),
                session: current,
                event: sftp::SftpEvent::Failed {
                    tab_id: "closed-tab".into(),
                    error: "closed".into(),
                },
            },
        )
        .is_none());
    }

    #[test]
    fn worker_envelope_rejects_a_different_pane_in_the_same_tab() {
        let mut manager = super::TabManager::new();
        let tab_id = manager.new_ssh_placeholder(&placeholder_connection("split.example"));
        let super::tab_manager::SplitPanePlan::Ssh {
            pane_id: second_pane_id,
            session: second_session,
            ..
        } = manager
            .split_active_pane(super::SplitDirection::Vertical, 80, 24)
            .unwrap()
        else {
            panic!("SSH split must produce a pane-scoped connect plan");
        };
        let first_session = manager.tabs[0]
            .pane(&tab_id)
            .unwrap()
            .completion
            .session()
            .clone();
        let (first_worker, _) = sftp::test_handle_for(&tab_id, &tab_id, first_session);
        let first_worker_id = first_worker.id();
        let (second_worker, _) =
            sftp::test_handle_for(&tab_id, &second_pane_id, second_session.clone());
        let second_worker_id = second_worker.id();
        let workers = HashMap::from([
            (tab_id.clone(), first_worker),
            (second_pane_id.clone(), second_worker),
        ]);

        assert!(super::take_current_sftp_worker_event(
            &manager,
            &workers,
            sftp::SftpWorkerEvent {
                worker_id: first_worker_id,
                tab_id: tab_id.clone(),
                pane_id: second_pane_id.clone(),
                session: second_session.clone(),
                event: sftp::SftpEvent::Ready {
                    tab_id: tab_id.clone(),
                    home: "/wrong-pane".into(),
                },
            },
        )
        .is_none());

        let accepted = super::take_current_sftp_worker_event(
            &manager,
            &workers,
            sftp::SftpWorkerEvent {
                worker_id: second_worker_id,
                tab_id: tab_id.clone(),
                pane_id: second_pane_id.clone(),
                session: second_session,
                event: sftp::SftpEvent::Ready {
                    tab_id,
                    home: "/right-pane".into(),
                },
            },
        )
        .expect("matching tab, pane, worker and session must be accepted");
        assert_eq!(accepted.1, second_pane_id);
    }

    #[test]
    fn tab_change_cancellation_clears_pending_fill() {
        let mut manager = super::TabManager::new();
        manager.new_local("bash", 80, 24);
        manager.tabs[0].completion.begin_fill(17, "git status");

        super::cancel_pending_fill_for_tab(&mut manager, 0);

        assert!(!manager.tabs[0].completion.fill_pending());
    }

    #[test]
    fn sftp_reconnect_preparation_cancels_pending_fill_before_send() {
        let session = CompletionSessionKey::new_for_test(9, "reconnect");
        let mut completion = super::smart_completion::CompletionState::new(session);
        completion.set_sftp_ready(true);
        completion.begin_fill(41, "git status");
        let history_request = completion.mark_history_loading();

        super::prepare_sftp_reconnect_state(&mut completion);

        assert!(!completion.sftp_ready());
        assert!(!completion.fill_pending());
        assert!(matches!(
            completion.history_status(),
            super::smart_completion::HistoryStatus::Disabled { .. }
        ));
        assert!(!completion.apply_history_result(
            &history_request,
            Ok::<_, std::io::Error>(vec!["stale".into()])
        ));
    }

    #[test]
    fn completion_history_status_diagnostic_is_typed_and_redacted() {
        use super::smart_completion::HistoryStatus;

        let disabled = super::completion_history_status_diagnostic(HistoryStatus::Disabled {
            reason: "secret disabled path /home/alice",
        });
        let loading = super::completion_history_status_diagnostic(HistoryStatus::Loading);
        let ready = super::completion_history_status_diagnostic(HistoryStatus::Ready { items: 12 });
        let error = super::completion_history_status_diagnostic(HistoryStatus::Error {
            reason: "secret raw error token=abc",
        });

        assert_eq!(disabled, "补全历史状态：已禁用");
        assert_eq!(loading, "补全历史状态：加载中");
        assert_eq!(ready, "补全历史状态：就绪（12 条）");
        assert_eq!(error, "补全历史状态：加载失败");
        for diagnostic in [&disabled, &loading, &ready, &error] {
            assert!(!diagnostic.contains("/home/alice"));
            assert!(!diagnostic.contains("token=abc"));
            assert!(!diagnostic.contains("secret"));
        }
    }

    #[test]
    fn completion_request_ids_skip_zero_even_when_wrapping() {
        let mut counter = 0;
        assert_eq!(super::advance_completion_request_id(&mut counter), 1);
        counter = u64::MAX;
        assert_eq!(super::advance_completion_request_id(&mut counter), 1);
    }

    #[test]
    fn remote_history_request_joins_ready_and_path_in_either_order() {
        let session = CompletionSessionKey::new_for_test(7, "join");
        let mut ready_first = super::smart_completion::CompletionState::new(session.clone());
        super::mark_sftp_ready(&mut ready_first, "/home/test");
        assert_eq!(
            super::remote_history_request(true, &ready_first, true),
            Some((session.clone(), "/home/test/.bash_history".to_string()))
        );

        let mut path_first = super::smart_completion::CompletionState::new(session.clone());
        path_first.set_history_path("/var/tmp/custom-history".into());
        assert_eq!(super::remote_history_request(true, &path_first, true), None);
        super::mark_sftp_ready(&mut path_first, "/home/test");
        assert_eq!(
            super::remote_history_request(true, &path_first, true),
            Some((session, "/var/tmp/custom-history".to_string()))
        );
    }

    #[test]
    fn remote_history_request_requires_ssh_worker_and_safe_absolute_fallback() {
        let session = CompletionSessionKey::new_for_test(7, "fallback");
        let mut state = super::smart_completion::CompletionState::new(session);
        super::mark_sftp_ready(&mut state, "relative/home");
        assert_eq!(state.history_path(), None);
        assert_eq!(super::remote_history_request(true, &state, true), None);

        state.set_history_path("/home/test/.bash_history".into());
        assert_eq!(super::remote_history_request(false, &state, true), None);
        assert_eq!(super::remote_history_request(true, &state, false), None);
    }

    #[test]
    fn stale_successful_ssh_result_is_explicitly_shutdown() {
        let (write_tx, _write_rx) = super::zmodem::runtime::transport_write_channel(
            std::sync::Arc::new(super::zmodem::runtime::ProtocolGate::new()),
        );
        let (resize_tx, _resize_rx) = std::sync::mpsc::sync_channel(8);
        let (shutdown_tx, shutdown_rx) = std::sync::mpsc::channel();
        let (_io_done_tx, io_done_rx) = std::sync::mpsc::channel();
        let handle = super::ssh::SshHandle {
            reader: Box::new(std::io::empty()),
            write_tx,
            resize_tx,
            shutdown_tx,
            io_done_rx,
            bash_runtime: None,
        };

        super::reject_stale_ssh_result(Ok(handle));

        assert!(shutdown_rx.try_recv().is_ok());
    }

    #[test]
    fn undelivered_successful_ssh_ready_is_explicitly_shutdown() {
        let (write_tx, _write_rx) = super::zmodem::runtime::transport_write_channel(
            std::sync::Arc::new(super::zmodem::runtime::ProtocolGate::new()),
        );
        let (resize_tx, _resize_rx) = std::sync::mpsc::sync_channel(8);
        let (shutdown_tx, shutdown_rx) = std::sync::mpsc::channel();
        let (_io_done_tx, io_done_rx) = std::sync::mpsc::channel();
        let event = UserEvent::SshReady {
            tab_id: "closed-tab".into(),
            pane_id: "closed-pane".into(),
            session: CompletionSessionKey::new_for_test(1, "closed-session"),
            result: Ok(super::ssh::SshHandle {
                reader: Box::new(std::io::empty()),
                write_tx,
                resize_tx,
                shutdown_tx,
                io_done_rx,
                bash_runtime: None,
            }),
        };

        super::cleanup_undelivered_user_event(event);

        assert!(shutdown_rx.try_recv().is_ok());
    }

    #[test]
    fn completion_event_debug_redacts_token_and_history_bytes() {
        let session = CompletionSessionKey::new_for_test(7, "secret-token");
        let event = UserEvent::CompletionHistory {
            tab_id: "tab-1".into(),
            pane_id: "pane-1".into(),
            request: history_request(session.clone()),
            session,
            path: "/secret/history/path".into(),
            result: Ok(b"secret-history-command".to_vec()),
        };

        let debug = format!("{event:?}");

        assert!(debug.contains("CompletionHistory"));
        assert!(debug.contains("tab-1"));
        assert!(!debug.contains("secret-token"));
        assert!(!debug.contains("/secret/history/path"));
        assert!(!debug.contains("secret-history-command"));
    }

    #[test]
    fn adb_history_scope_is_shared_by_endpoint_but_not_non_terminal_pages() {
        assert_eq!(
            super::adb_history_scope(&super::TabType::Local {
                shell_path: "/bin/bash".into()
            }),
            Some(super::adb_history::HostScope::Local)
        );
        let params = remote_params("alice", "example.test", 2222);
        assert_eq!(
            super::adb_history_scope(&super::TabType::Ssh {
                label: "mutable label".into(),
                params: params.clone(),
            }),
            Some(super::adb_history::HostScope::Ssh {
                user: "alice".into(),
                host: "example.test".into(),
                port: 2222,
            })
        );
        assert_eq!(
            super::adb_history_scope(&super::TabType::Process {
                label: "process".into(),
                key: super::monitor::MonitorKey::from_ssh(&params),
                params: Some(params),
            }),
            None
        );
    }

    #[test]
    fn adb_completion_event_debug_and_handler_reject_secrets_and_stale_sessions() {
        let mut manager = super::TabManager::new();
        let (tab_id, _) = manager.new_local("bash", 80, 24);
        let session = manager.tabs[0].completion.session().clone();
        let serial = "SECRET-SERIAL";
        assert_eq!(
            manager.tabs[0]
                .completion
                .observe_submission(Some(&format!("adb -s {serial} shell")), true)
                .as_deref(),
            Some(serial)
        );
        let identity = super::adb_history::AdbHistoryIdentity::new(
            super::adb_history::HostScope::Local,
            serial,
        )
        .unwrap();
        assert!(manager.tabs[0].completion.activate_adb_history(identity));
        let request = manager.tabs[0]
            .completion
            .mark_adb_history_loading()
            .unwrap();
        let event = UserEvent::AdbCompletionHistory {
            tab_id: tab_id.clone(),
            pane_id: tab_id.clone(),
            session: session.clone(),
            request: request.clone(),
            result: Ok(vec!["SECRET-COMMAND".into()]),
        };
        let debug = format!("{event:?}");
        assert!(debug.contains("AdbCompletionHistory"));
        assert!(!debug.contains(serial));
        assert!(!debug.contains("SECRET-COMMAND"));

        assert!(!super::apply_adb_completion_history_event(
            &mut manager,
            &tab_id,
            &tab_id,
            &CompletionSessionKey::new_for_test(session.generation, "stale"),
            &request,
            Ok(vec!["stale".into()])
        ));
        assert!(super::apply_adb_completion_history_event(
            &mut manager,
            &tab_id,
            &tab_id,
            &session,
            &request,
            Ok(vec!["getprop".into()])
        ));
        assert_eq!(manager.tabs[0].completion.history(), ["getprop"]);
    }

    #[test]
    fn completion_history_handler_ignores_stale_tab_and_session() {
        let mut manager = super::TabManager::new();
        let (tab_id, _) = manager.new_local("bash", 80, 24);
        let current = manager.tabs[0].completion.session().clone();
        let current_path =
            std::path::PathBuf::from(manager.tabs[0].completion.history_path().unwrap());
        manager.tabs[0]
            .completion
            .replace_history(vec!["keep me".into()]);
        let request = manager.tabs[0].completion.mark_history_loading();

        assert!(!super::apply_completion_history_event(
            &mut manager,
            "missing-tab",
            &tab_id,
            &current,
            &request,
            &current_path,
            Ok(b"replace me\n".to_vec()),
        ));
        assert!(!super::apply_completion_history_event(
            &mut manager,
            &tab_id,
            &tab_id,
            &CompletionSessionKey::new_for_test(current.generation, "stale"),
            &request,
            &current_path,
            Ok(b"replace me\n".to_vec()),
        ));

        assert_eq!(manager.tabs[0].completion.history(), ["keep me"]);
    }

    #[test]
    fn completion_history_handler_clears_current_history_on_read_failure() {
        let mut manager = super::TabManager::new();
        let (tab_id, _) = manager.new_local("bash", 80, 24);
        let session = manager.tabs[0].completion.session().clone();
        let current_path =
            std::path::PathBuf::from(manager.tabs[0].completion.history_path().unwrap());
        manager.tabs[0]
            .completion
            .replace_history(vec!["stale history".into()]);
        let request = manager.tabs[0].completion.mark_history_loading();

        assert!(super::apply_completion_history_event(
            &mut manager,
            &tab_id,
            &tab_id,
            &session,
            &request,
            &current_path,
            Err("unreadable".into()),
        ));

        assert!(manager.tabs[0].completion.history().is_empty());
        assert!(matches!(
            manager.tabs[0].completion.history_status(),
            super::smart_completion::HistoryStatus::Error { .. }
        ));
    }

    #[test]
    fn completion_history_handler_keeps_empty_success_ready() {
        let mut manager = super::TabManager::new();
        let (tab_id, _) = manager.new_local("bash", 80, 24);
        let session = manager.tabs[0].completion.session().clone();
        let path = std::path::PathBuf::from(manager.tabs[0].completion.history_path().unwrap());
        let request = manager.tabs[0].completion.mark_history_loading();

        assert!(super::apply_completion_history_event(
            &mut manager,
            &tab_id,
            &tab_id,
            &session,
            &request,
            &path,
            Ok(Vec::new()),
        ));
        assert!(matches!(
            manager.tabs[0].completion.history_status(),
            super::smart_completion::HistoryStatus::Ready { items: 0 }
        ));
    }

    #[test]
    fn late_default_history_result_cannot_overwrite_new_custom_path_result() {
        let mut manager = super::TabManager::new();
        let (tab_id, _) = manager.new_local("bash", 80, 24);
        let session = manager.tabs[0].completion.session().clone();
        let default_path =
            std::path::PathBuf::from(manager.tabs[0].completion.history_path().unwrap());
        let default_request = manager.tabs[0].completion.mark_history_loading();
        let custom_path = std::path::PathBuf::from("/tmp/liteterm-custom-history");
        super::apply_history_path_event(
            &mut manager,
            &tab_id,
            &tab_id,
            &session,
            custom_path.to_string_lossy().into_owned(),
        );
        let custom_request = manager.tabs[0].completion.mark_history_loading();

        assert!(super::apply_completion_history_event(
            &mut manager,
            &tab_id,
            &tab_id,
            &session,
            &custom_request,
            &custom_path,
            Ok(b"custom command\n".to_vec()),
        ));
        assert!(!super::apply_completion_history_event(
            &mut manager,
            &tab_id,
            &tab_id,
            &session,
            &default_request,
            &default_path,
            Ok(b"default command\n".to_vec()),
        ));

        assert_eq!(manager.tabs[0].completion.history(), ["custom command"]);
    }

    #[test]
    fn completion_history_handler_rejects_stale_request_on_same_path() {
        let mut manager = super::TabManager::new();
        let (tab_id, _) = manager.new_local("bash", 80, 24);
        let session = manager.tabs[0].completion.session().clone();
        let path = std::path::PathBuf::from(manager.tabs[0].completion.history_path().unwrap());
        let stale_request = manager.tabs[0].completion.mark_history_loading();
        let current_request = manager.tabs[0].completion.mark_history_loading();

        assert!(!super::apply_completion_history_event(
            &mut manager,
            &tab_id,
            &tab_id,
            &session,
            &stale_request,
            &path,
            Ok(b"stale command\n".to_vec()),
        ));
        assert!(super::apply_completion_history_event(
            &mut manager,
            &tab_id,
            &tab_id,
            &session,
            &current_request,
            &path,
            Ok(b"current command\n".to_vec()),
        ));
        assert_eq!(manager.tabs[0].completion.history(), ["current command"]);
    }

    #[test]
    fn completion_history_handler_preserves_commands_executed_during_load() {
        let mut manager = super::TabManager::new();
        let (tab_id, _) = manager.new_local("bash", 80, 24);
        let session = manager.tabs[0].completion.session().clone();
        let path = std::path::PathBuf::from(manager.tabs[0].completion.history_path().unwrap());
        let request = manager.tabs[0].completion.mark_history_loading();
        manager.tabs[0].completion.merge_executed("just executed");

        assert!(super::apply_completion_history_event(
            &mut manager,
            &tab_id,
            &tab_id,
            &session,
            &request,
            &path,
            Ok(b"older command\n".to_vec()),
        ));
        assert_eq!(
            manager.tabs[0].completion.history(),
            ["just executed", "older command"]
        );
    }

    #[test]
    fn history_path_handler_reloads_only_a_new_current_absolute_path() {
        let mut manager = super::TabManager::new();
        let (tab_id, _) = manager.new_local("bash", 80, 24);
        let session = manager.tabs[0].completion.session().clone();
        manager.tabs[0]
            .completion
            .replace_history(vec!["old history".into()]);
        let new_path = std::path::PathBuf::from("/tmp/liteterm-other-history");

        let request = super::apply_history_path_event(
            &mut manager,
            &tab_id,
            &tab_id,
            &session,
            new_path.to_string_lossy().into_owned(),
        );

        assert_eq!(request, Some((session.clone(), new_path.clone())));
        assert_eq!(manager.tabs[0].completion.history_path(), new_path.to_str());
        assert!(manager.tabs[0].completion.history().is_empty());
        assert_eq!(
            super::apply_history_path_event(
                &mut manager,
                &tab_id,
                &tab_id,
                &session,
                new_path.to_string_lossy().into_owned(),
            ),
            None
        );
    }

    #[test]
    fn active_completion_refreshes_for_input_and_clears_for_none_or_empty() {
        let mut manager = super::TabManager::new();
        manager.new_local("bash", 80, 24);
        manager.tabs[0]
            .completion
            .replace_history(vec!["git status".into(), "git log".into()]);

        super::refresh_active_completion(&mut manager, Some("git"));
        assert_eq!(
            manager.tabs[0].completion.candidates(),
            ["git status", "git log"]
        );
        manager.tabs[0].completion.move_selection(1);

        super::refresh_active_completion(&mut manager, None);
        assert!(manager.tabs[0].completion.candidates().is_empty());
        assert_eq!(manager.tabs[0].completion.selected(), 0);

        super::refresh_active_completion(&mut manager, Some("git"));
        super::refresh_active_completion(&mut manager, Some(""));
        assert!(manager.tabs[0].completion.candidates().is_empty());
    }

    #[test]
    fn plain_click_anchor_is_not_a_visible_selection() {
        assert_eq!(drag_selection_range(Some((4, 2)), (4, 2)), None);
    }

    #[test]
    fn dragging_across_cells_creates_a_visible_selection() {
        assert_eq!(
            drag_selection_range(Some((4, 2)), (7, 2)),
            Some(((4, 2), (7, 2)))
        );
        assert_eq!(
            drag_selection_range(Some((7, 2)), (4, 2)),
            Some(((7, 2), (4, 2)))
        );
    }

    #[test]
    fn moving_without_a_pressed_anchor_does_not_create_selection() {
        assert_eq!(drag_selection_range(None, (7, 2)), None);
    }

    #[test]
    fn mouse_mode_without_shift_starts_terminal_report_gesture() {
        assert_eq!(
            left_mouse_gesture(true, false, (4, 2)),
            LeftMouseGesture::TerminalReport { last_cell: (4, 2) }
        );
    }
