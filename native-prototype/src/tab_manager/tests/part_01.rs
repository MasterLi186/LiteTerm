    #[test]
    fn network_tabs_are_unique_by_monitor_key_and_remote_owns_a_lease() {
        let mut manager = TabManager::new();
        let connection = test_ssh_connection_for("network.example", "alice", 2202);
        let params = crate::ssh::ConnectionParams::from(&connection);
        let key = MonitorKey::from_ssh(&params);
        let first = manager.open_network(key.clone(), Some(params.clone()), Some("eth0".into()));
        let second = manager.open_network(key.clone(), Some(params.clone()), Some("eth1".into()));

        assert_eq!(first, second);
        assert_eq!(manager.len(), 1);
        assert!(manager.tabs[0].remote_monitor_leased);
        assert_eq!(
            manager.remote_monitor_requirements().get(&key),
            Some(&params)
        );
    }

    #[test]
    fn local_network_and_serial_tabs_never_own_remote_monitor_leases() {
        let mut manager = TabManager::new();
        manager.open_network(MonitorKey::Local, None, Some("lo".into()));
        manager.new_serial_placeholder(crate::serial::SerialSpec {
            device: "/dev/ttyTEST".into(),
            display_name: "测试串口".into(),
            serial_number: None,
            baud_rate: crate::serial::DEFAULT_BAUD_RATE,
        });

        assert!(!manager.tabs.iter().any(|tab| tab.remote_monitor_leased));
        assert!(manager.tabs[1].tab_type.is_terminal());
        assert!(manager.remote_monitor_requirements().is_empty());
    }

    #[test]
    fn settings_page_is_a_singleton_and_reopening_focuses_it() {
        let mut manager = TabManager::new();
        let (settings_id, created) = manager.open_settings();
        assert!(created);
        manager.open_network(MonitorKey::Local, None, Some("lo".into()));

        let (reopened_id, created_again) = manager.open_settings();

        assert_eq!(reopened_id, settings_id);
        assert!(!created_again);
        assert_eq!(manager.len(), 2);
        assert_eq!(manager.active().map(|tab| tab.id.as_str()), Some(settings_id.as_str()));
    }

    #[test]
    fn zmodem_capability_is_local_and_direct_ssh_only() {
        let local = TabType::Local {
            shell_path: "/bin/bash".into(),
        };
        let ssh = TabType::Ssh {
            label: "ssh".into(),
            params: crate::ssh::ConnectionParams::from(&test_ssh_connection()),
        };
        let serial = TabType::Serial {
            spec: crate::serial::SerialSpec {
                device: "/dev/ttyUSB0".into(),
                display_name: "ttyUSB0".into(),
                serial_number: None,
                baud_rate: crate::serial::DEFAULT_BAUD_RATE,
            },
        };
        assert_eq!(
            local.zmodem_capability(),
            crate::zmodem::runtime::RuntimeCapability::Local
        );
        assert_eq!(
            ssh.zmodem_capability(),
            crate::zmodem::runtime::RuntimeCapability::DirectSsh
        );
        assert_eq!(
            serial.zmodem_capability(),
            crate::zmodem::runtime::RuntimeCapability::SerialDisabled
        );
    }

    #[test]
    fn rename_trims_updates_display_and_persists_ssh_identity_label() {
        let mut manager = TabManager::new();
        let id = manager.new_ssh_placeholder(&test_ssh_connection());

        assert!(manager.rename(&id, "  新标签  "));
        let tab = manager.tabs.iter().find(|tab| tab.id == id).unwrap();
        assert_eq!(tab.label, "新标签");
        assert!(
            matches!(&tab.tab_type, TabType::Ssh { label, .. } if label == "新标签"),
            "SSH reconnect/apply must retain the renamed label"
        );
    }

    #[test]
    fn rename_rejects_blank_and_caps_unicode_label_length() {
        let mut manager = TabManager::new();
        let id = manager.new_ssh_placeholder(&test_ssh_connection());
        let original = manager.tabs[0].label.clone();

        assert!(!manager.rename(&id, " \n\t "));
        assert_eq!(manager.tabs[0].label, original);

        assert!(manager.rename(&id, &"终".repeat(MAX_TAB_LABEL_CHARS + 5)));
        assert_eq!(manager.tabs[0].label.chars().count(), MAX_TAB_LABEL_CHARS);
    }

    fn test_ssh_handle(
        bash_runtime: Option<RemoteBashRuntime>,
    ) -> (crate::ssh::SshHandle, mpsc::Receiver<()>) {
        let (write_tx, _write_rx) = crate::zmodem::runtime::transport_write_channel(Arc::new(
            crate::zmodem::runtime::ProtocolGate::new(),
        ));
        let (resize_tx, _resize_rx) = mpsc::sync_channel(8);
        let (shutdown_tx, shutdown_rx) = mpsc::channel();
        let (_io_done_tx, io_done_rx) = mpsc::channel();
        (
            crate::ssh::SshHandle {
                reader: Box::new(std::io::empty()),
                write_tx,
                resize_tx,
                shutdown_tx,
                io_done_rx,
                bash_runtime,
            },
            shutdown_rx,
        )
    }

    struct ReadStarted<R> {
        reader: R,
        started_tx: Option<mpsc::Sender<()>>,
    }

    impl<R: Read> Read for ReadStarted<R> {
        fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
            if let Some(started_tx) = self.started_tx.take() {
                let _ = started_tx.send(());
            }
            self.reader.read(buffer)
        }
    }

    struct BlockingSshProbe {
        terminal: Arc<Mutex<TerminalState>>,
        shutdown_seen_rx: mpsc::Receiver<()>,
        release_worker_tx: mpsc::Sender<()>,
        read_done_rx: mpsc::Receiver<()>,
        worker_thread: thread::JoinHandle<()>,
        read_thread: thread::JoinHandle<()>,
    }

    impl BlockingSshProbe {
        fn wait_for_shutdown(&self) -> bool {
            let shutdown_requested = self
                .shutdown_seen_rx
                .recv_timeout(Duration::from_secs(1))
                .is_ok();
            if !shutdown_requested {
                self.terminal.lock().unwrap().shutdown();
                self.shutdown_seen_rx
                    .recv_timeout(Duration::from_secs(1))
                    .expect("测试清理必须送达 SSH shutdown");
            }
            shutdown_requested
        }

        fn release_and_wait(self) {
            let _ = self.release_worker_tx.send(());
            self.read_done_rx
                .recv_timeout(Duration::from_secs(1))
                .expect("SSH pipe 关闭后 read_loop 必须有界退出");
            self.worker_thread.join().unwrap();
            self.read_thread.join().unwrap();
        }
    }

    fn add_blocked_ssh_tab(manager: &mut TabManager) -> BlockingSshProbe {
        let tab_id = manager.new_ssh_placeholder(&test_ssh_connection());
        let session = manager.tabs[0].completion.session().clone();
        let (pipe_read, pipe_write) = os_pipe::pipe().unwrap();
        let (read_started_tx, read_started_rx) = mpsc::channel();
        let (write_tx, write_rx) = crate::zmodem::runtime::transport_write_channel(Arc::new(
            crate::zmodem::runtime::ProtocolGate::new(),
        ));
        let (resize_tx, resize_rx) = mpsc::sync_channel(8);
        let (shutdown_tx, shutdown_rx) = mpsc::channel();
        let (io_done_tx, io_done_rx) = mpsc::channel();
        let (shutdown_seen_tx, shutdown_seen_rx) = mpsc::channel();
        let (release_worker_tx, release_worker_rx) = mpsc::channel();
        let handle = crate::ssh::SshHandle {
            reader: Box::new(ReadStarted {
                reader: pipe_read,
                started_tx: Some(read_started_tx),
            }),
            write_tx,
            resize_tx,
            shutdown_tx,
            io_done_rx,
            bash_runtime: None,
        };
        let terminal = manager
            .apply_ssh(&tab_id, &session, handle, 80, 24)
            .unwrap();
        let read_terminal = terminal.clone();
        let (read_done_tx, read_done_rx) = mpsc::channel();
        let read_thread = thread::spawn(move || {
            crate::terminal::read_loop(read_terminal, || {}, |_| {});
            let _ = read_done_tx.send(());
        });
        let worker_thread = thread::spawn(move || {
            let _write_rx = write_rx;
            let _resize_rx = resize_rx;
            shutdown_rx.recv().unwrap();
            let _ = shutdown_seen_tx.send(());
            let _ = release_worker_rx.recv();
            drop(pipe_write);
            let _ = io_done_tx.send(());
        });
        read_started_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("read_loop 必须先进入阻塞 pipe read");

        BlockingSshProbe {
            terminal,
            shutdown_seen_rx,
            release_worker_tx,
            read_done_rx,
            worker_thread,
            read_thread,
        }
    }

    #[test]
    fn close_shuts_down_arc_held_ssh_before_removing_tab() {
        let mut manager = TabManager::new();
        let probe = add_blocked_ssh_tab(&mut manager);
        let (close_done_tx, close_done_rx) = mpsc::channel();
        let close_thread = thread::spawn(move || {
            manager.close(0);
            let _ = close_done_tx.send(());
        });

        let shutdown_requested = probe.wait_for_shutdown();
        close_done_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("close 不得等待 SSH worker 或 read_loop");
        probe.release_and_wait();
        close_thread.join().unwrap();

        assert!(shutdown_requested);
    }

    #[test]
    fn close_others_shuts_down_removed_arc_held_ssh_tabs() {
        let mut manager = TabManager::new();
        let probe = add_blocked_ssh_tab(&mut manager);
        manager.new_ssh_placeholder(&test_ssh_connection_for("keep.example", "keeper", 22));
        let (close_done_tx, close_done_rx) = mpsc::channel();
        let close_thread = thread::spawn(move || {
            manager.close_others(1);
            let _ = close_done_tx.send(());
        });

        let shutdown_requested = probe.wait_for_shutdown();
        close_done_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("close_others 不得等待 SSH worker 或 read_loop");
        probe.release_and_wait();
        close_thread.join().unwrap();

        assert!(shutdown_requested);
    }

    #[test]
    fn reconnect_reset_shuts_down_replaced_arc_held_ssh() {
        let mut manager = TabManager::new();
        let probe = add_blocked_ssh_tab(&mut manager);
        let (reset_done_tx, reset_done_rx) = mpsc::channel();
        let reset_thread = thread::spawn(move || {
            let plan = manager.reset_ssh_for_reconnect(0);
            let _ = reset_done_tx.send(plan.is_some());
        });

        let shutdown_requested = probe.wait_for_shutdown();
        assert!(reset_done_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("reset 不得等待 SSH worker 或 read_loop"));
        probe.release_and_wait();
        reset_thread.join().unwrap();

        assert!(shutdown_requested);
    }

    #[test]
    fn local_bash_tab_shares_one_generation_one_session_with_runtime() {
        let mut manager = TabManager::new();
        let (_, terminal) = manager.new_local("bash", 80, 24);
        let terminal = terminal.lock().unwrap();
        let runtime = terminal.local_bash_runtime.as_ref().unwrap();

        assert_eq!(manager.tabs[0].completion.session().generation, 1);
        assert_eq!(manager.tabs[0].completion.session(), runtime.session());
    }

    #[test]
    fn try_new_local_invalid_path_returns_error_without_mutating_manager() {
        let mut manager = TabManager::new();
        let existing_id = manager.new_ssh_placeholder(&test_ssh_connection());
        let original_len = manager.len();
        let original_active_idx = manager.active_idx;
        let original_counter = manager.local_counter;

        let result = manager.try_new_local("/definitely/not/a/real/liteterm-shell", 80, 24);

        assert!(result.is_err());
        assert_eq!(manager.len(), original_len);
        assert_eq!(manager.active_idx, original_active_idx);
        assert_eq!(manager.local_counter, original_counter);
        assert_eq!(manager.tabs[manager.active_idx].id, existing_id);
    }

    #[test]
    fn local_bash_tab_starts_with_an_absolute_default_history_path() {
        let mut manager = TabManager::new();
        manager.new_local("bash", 80, 24);

        let path = std::path::Path::new(manager.tabs[0].completion.history_path().unwrap());
        assert!(path.is_absolute());
        assert!(path.ends_with(".bash_history"));
    }

    #[test]
    fn default_bash_history_path_requires_an_absolute_home() {
        assert_eq!(
            super::default_bash_history_path(Some(std::path::PathBuf::from("relative-home"))),
            None
        );
        assert_eq!(super::default_bash_history_path(None), None);

        let absolute =
            super::default_bash_history_path(Some(std::path::PathBuf::from("/home/test-user")))
                .unwrap();
        assert_eq!(
            std::path::Path::new(&absolute),
            std::path::Path::new("/home/test-user/.bash_history")
        );
        assert!(std::path::Path::new(&absolute).is_absolute());
    }

    #[test]
    fn non_bash_local_tab_does_not_get_a_bash_history_path() {
        let mut manager = TabManager::new();
        manager.new_local("fish", 80, 24);

        assert_eq!(manager.tabs[0].completion.history_path(), None);
    }

    #[test]
    fn ssh_placeholder_starts_with_generation_one_completion() {
        let connection = test_ssh_connection();
        let mut manager = TabManager::new();

        manager.new_ssh_placeholder(&connection);

        assert_eq!(manager.tabs[0].completion.session().generation, 1);
        assert!(manager.tabs[0]
            .terminal
            .lock()
            .unwrap()
            .local_bash_runtime
            .is_none());
        assert!(!manager.tabs[0].ssh_connected);
        assert!(!manager.tabs[0].remote_monitor_leased);
    }

    #[test]
    fn ssh_reconnect_keeps_tab_id_and_rotates_session() {
        let mut manager = TabManager::new();
        let connection = test_ssh_connection();
        let tab_id = manager.new_ssh_placeholder(&connection);
        manager.tabs[0]
            .completion
            .replace_history(vec!["old history".into()]);
        let previous = manager.tabs[0].completion.session().clone();

        let plan = manager.reset_ssh_for_reconnect(0).unwrap();

        assert_eq!(plan.tab_id, tab_id);
        assert_eq!(plan.session.generation, previous.generation + 1);
        assert_ne!(plan.session.token(), previous.token());
        assert!(manager.tabs[0].completion.history().is_empty());
        assert_eq!(manager.tabs[0].completion.session(), &plan.session);
    }

    #[test]
    fn apply_ssh_rejects_mismatched_integrated_runtime_and_shuts_it_down() {
        let mut manager = TabManager::new();
        let tab_id = manager.new_ssh_placeholder(&test_ssh_connection());
        let current = manager.tabs[0].completion.session().clone();
        let stale = current.successor();
        let runtime = RemoteBashRuntime {
            session: stale,
            bash_path: "/bin/bash".into(),
            rc_path: "/tmp/stale.rc".into(),
            candidate_path: "/tmp/stale.candidate".into(),
            widget_sequence: "\x1b[777;1~".into(),
            snapshot_sequence: "\x1b[778;1~".into(),
        };
        let (handle, shutdown_rx) = test_ssh_handle(Some(runtime));

        assert!(manager
            .apply_ssh(&tab_id, &current, handle, 80, 24)
            .is_none());
        assert!(shutdown_rx.try_recv().is_ok());
        assert_eq!(manager.tabs[0].completion.session(), &current);
        assert!(!manager.tabs[0].ssh_connected);
        assert!(!manager.tabs[0].remote_monitor_leased);
        assert!(manager.remote_monitor_requirements().is_empty());
    }

    #[test]
    fn apply_ssh_accepts_plain_shell_fallback_without_runtime() {
        let mut manager = TabManager::new();
        let tab_id = manager.new_ssh_placeholder(&test_ssh_connection());
        let session = manager.tabs[0].completion.session().clone();
        let (handle, _shutdown_rx) = test_ssh_handle(None);

        assert!(manager
            .apply_ssh(&tab_id, &session, handle, 80, 24)
            .is_some());
        assert_eq!(manager.tabs[0].label, "测试");
        assert!(manager.tabs[0].ssh_connected);
    }

    #[test]
    fn late_plain_shell_result_cannot_replace_newer_reconnect() {
        let mut manager = TabManager::new();
        let tab_id = manager.new_ssh_placeholder(&test_ssh_connection());
        let old_session = manager.tabs[0].completion.session().clone();
        let reconnect = manager.reset_ssh_for_reconnect(0).unwrap();
        let (new_handle, _new_shutdown_rx) = test_ssh_handle(None);
        let (old_handle, old_shutdown_rx) = test_ssh_handle(None);

        assert!(manager
            .apply_ssh(&tab_id, &reconnect.session, new_handle, 80, 24)
            .is_some());
        assert!(manager
            .apply_ssh(&tab_id, &old_session, old_handle, 80, 24)
            .is_none());

        assert!(old_shutdown_rx.try_recv().is_ok());
        assert_eq!(
            manager.tabs[0].completion.session(),
            &reconnect.session,
            "晚到的旧结果不得回退当前会话"
        );
    }

    #[test]
    fn ssh_placeholders_have_distinct_tab_ids_but_share_monitor_key() {
        let mut manager = TabManager::new();
        let connection = test_ssh_connection();

        let first_id = manager.new_ssh_placeholder(&connection);
        let second_id = manager.new_ssh_placeholder(&connection);

        assert_ne!(first_id, second_id);
        assert_eq!(manager.tabs[0].monitor_key(), manager.tabs[1].monitor_key());
        assert_eq!(
            manager.active_monitor_key(),
            MonitorKey::remote("test", "127.0.0.1", 22)
        );
    }

    #[test]
    fn close_others_keeps_the_shared_remote_monitor_requirement() {
        let mut manager = TabManager::new();
        let connection = test_ssh_connection();
        let first_id = manager.new_ssh_placeholder(&connection);
        let second_id = manager.new_ssh_placeholder(&connection);
        manager.tabs[0].ssh_connected = true;
        manager.tabs[1].ssh_connected = true;
        manager.tabs[0].remote_monitor_leased = true;
        manager.tabs[1].remote_monitor_leased = true;

        manager.close_others(1);

        assert_ne!(first_id, second_id);
        assert_eq!(manager.len(), 1);
        assert_eq!(manager.remote_monitor_requirements().len(), 1);
    }

    #[test]
    fn close_others_removing_the_last_remote_leaves_no_requirement() {
        let mut manager = TabManager::new();
        let connection = test_ssh_connection();
        manager.new_ssh_placeholder(&connection);
        manager.tabs[0].ssh_connected = true;
        manager.tabs[0].remote_monitor_leased = true;
        manager.new_local("sh", 80, 24);

        manager.close_others(1);

        assert!(manager.remote_monitor_requirements().is_empty());
    }

    #[test]
    fn remote_monitor_requirements_include_each_connected_remote_once() {
        let mut manager = TabManager::new();
        let shared = test_ssh_connection_for("shared.example", "alice", 22);
        let distinct = test_ssh_connection_for("shared.example", "bob", 22);
        let first_id = manager.new_ssh_placeholder(&shared);
        let second_id = manager.new_ssh_placeholder(&shared);
        let third_id = manager.new_ssh_placeholder(&distinct);
        manager.new_ssh_placeholder(&test_ssh_connection_for("pending.example", "carol", 22));
        let first_session = manager.tabs[0].completion.session().clone();
        let second_session = manager.tabs[1].completion.session().clone();
        let third_session = manager.tabs[2].completion.session().clone();

        manager.new_local("sh", 80, 24);
        assert!(manager
            .apply_ssh(&first_id, &first_session, test_ssh_handle(None).0, 80, 24)
            .is_some());
        assert!(manager
            .apply_ssh(&second_id, &second_session, test_ssh_handle(None).0, 80, 24)
            .is_some());
        assert!(manager
            .apply_ssh(&third_id, &third_session, test_ssh_handle(None).0, 80, 24)
            .is_some());

        let requirements = manager.remote_monitor_requirements();

        assert_eq!(requirements.len(), 2);
        assert_eq!(
            requirements.get(&MonitorKey::remote("alice", "shared.example", 22)),
            Some(&crate::ssh::ConnectionParams::from(&shared))
        );
        assert_eq!(
            requirements.get(&MonitorKey::remote("bob", "shared.example", 22)),
            Some(&crate::ssh::ConnectionParams::from(&distinct))
        );
    }

    #[test]
    fn monitor_requirement_owner_is_tab_ordered_not_connection_ordered() {
        let mut manager = TabManager::new();
        let mut owner = test_ssh_connection_for("shared.example", "alice", 22);
        owner.key_path = "owner-key-sentinel".into();
        owner.password = "owner-password-sentinel".into();
        let mut fallback = test_ssh_connection_for("shared.example", "alice", 22);
        fallback.key_path = "fallback-key-sentinel".into();
        fallback.password = "fallback-password-sentinel".into();
        let owner_id = manager.new_ssh_placeholder(&owner);
        let fallback_id = manager.new_ssh_placeholder(&fallback);
        let owner_session = manager.tabs[0].completion.session().clone();
        let fallback_session = manager.tabs[1].completion.session().clone();

        assert!(manager
            .apply_ssh(
                &fallback_id,
                &fallback_session,
                test_ssh_handle(None).0,
                80,
                24
            )
            .is_some());
        assert!(manager
            .apply_ssh(&owner_id, &owner_session, test_ssh_handle(None).0, 80, 24)
            .is_some());
        let key = MonitorKey::remote("alice", "shared.example", 22);
        assert_eq!(
            manager.remote_monitor_requirements().get(&key),
            Some(&crate::ssh::ConnectionParams::from(&owner))
        );

        manager.close(0);

        assert_eq!(
            manager.remote_monitor_requirements().get(&key),
            Some(&crate::ssh::ConnectionParams::from(&fallback))
        );
    }

    #[test]
    fn reconnect_reset_marks_an_applied_ssh_tab_disconnected() {
        let mut manager = TabManager::new();
        let connection = test_ssh_connection();
        let tab_id = manager.new_ssh_placeholder(&connection);
        let session = manager.tabs[0].completion.session().clone();

        assert!(manager
            .apply_ssh(&tab_id, &session, test_ssh_handle(None).0, 80, 24)
            .is_some());
        assert!(manager.tabs[0].ssh_connected);

        manager.reset_ssh_for_reconnect(0).unwrap();

        assert!(!manager.tabs[0].ssh_connected);
        assert!(manager.tabs[0].remote_monitor_leased);
        assert_eq!(manager.remote_monitor_requirements().len(), 1);
    }

    #[test]
    fn reconnect_lease_survives_closing_an_unrelated_local_tab() {
        let mut manager = TabManager::new();
        let connection = test_ssh_connection();
        let tab_id = manager.new_ssh_placeholder(&connection);
        let session = manager.tabs[0].completion.session().clone();
        assert!(manager
            .apply_ssh(&tab_id, &session, test_ssh_handle(None).0, 80, 24)
            .is_some());
        manager.new_local("sh", 80, 24);

        manager.reset_ssh_for_reconnect(0).unwrap();
        manager.close(1);

        assert_eq!(manager.remote_monitor_requirements().len(), 1);
    }

    #[test]
    fn failed_reconnect_keeps_the_existing_remote_monitor_lease() {
        let mut manager = TabManager::new();
        let connection = test_ssh_connection();
        let tab_id = manager.new_ssh_placeholder(&connection);
        let session = manager.tabs[0].completion.session().clone();
        assert!(manager
            .apply_ssh(&tab_id, &session, test_ssh_handle(None).0, 80, 24)
            .is_some());

        let reconnect = manager.reset_ssh_for_reconnect(0).unwrap();
        assert!(manager.ssh_failed(
            &tab_id,
            &reconnect.pane_id,
            &reconnect.session,
            "reconnect failed"
        ));

        assert!(!manager.tabs[0].ssh_connected);
        assert!(manager.tabs[0].remote_monitor_leased);
        assert_eq!(manager.remote_monitor_requirements().len(), 1);
    }

    #[test]
    fn reconnect_lease_and_new_remote_key_both_remain_required() {
        let mut manager = TabManager::new();
        let a = test_ssh_connection_for("alpha.example", "alice", 22);
        let b = test_ssh_connection_for("beta.example", "bob", 2200);
        let a_id = manager.new_ssh_placeholder(&a);
        let a_session = manager.tabs[0].completion.session().clone();
        assert!(manager
            .apply_ssh(&a_id, &a_session, test_ssh_handle(None).0, 80, 24)
            .is_some());
        manager.reset_ssh_for_reconnect(0).unwrap();

        let b_id = manager.new_ssh_placeholder(&b);
        let b_session = manager.tabs[1].completion.session().clone();
        assert!(manager
            .apply_ssh(&b_id, &b_session, test_ssh_handle(None).0, 80, 24)
            .is_some());

        assert_eq!(manager.remote_monitor_requirements().len(), 2);
    }

    #[test]
    fn reconnect_lease_remains_after_closing_a_connected_same_key_sibling() {
        let mut manager = TabManager::new();
        let connection = test_ssh_connection();
        let a_id = manager.new_ssh_placeholder(&connection);
        let b_id = manager.new_ssh_placeholder(&connection);
        let a_session = manager.tabs[0].completion.session().clone();
        let b_session = manager.tabs[1].completion.session().clone();
        assert!(manager
            .apply_ssh(&a_id, &a_session, test_ssh_handle(None).0, 80, 24)
            .is_some());
        assert!(manager
            .apply_ssh(&b_id, &b_session, test_ssh_handle(None).0, 80, 24)
            .is_some());

        manager.reset_ssh_for_reconnect(0).unwrap();
        manager.close(1);

        assert_eq!(manager.remote_monitor_requirements().len(), 1);
    }

    #[test]
    fn closing_the_last_leased_remote_removes_its_requirement() {
        let mut manager = TabManager::new();
        let connection = test_ssh_connection();
        let tab_id = manager.new_ssh_placeholder(&connection);
        let session = manager.tabs[0].completion.session().clone();
        assert!(manager
            .apply_ssh(&tab_id, &session, test_ssh_handle(None).0, 80, 24)
            .is_some());
        manager.new_local("sh", 80, 24);

        manager.close_others(1);

        assert!(manager.remote_monitor_requirements().is_empty());
    }

    #[test]
    fn local_and_empty_managers_use_the_local_monitor_key() {
        let mut manager = TabManager::new();

        assert_eq!(manager.active_monitor_key(), MonitorKey::Local);

        manager.new_local("sh", 80, 24);
        assert_eq!(manager.tabs[0].monitor_key(), MonitorKey::Local);
        assert_eq!(manager.active_monitor_key(), MonitorKey::Local);
    }

    #[test]
    fn tab_type_debug_does_not_expose_ssh_credentials() {
        let mut connection = test_ssh_connection();
        connection.password = "password-sentinel".into();
        connection.key_path = "key-path-sentinel".into();
        let mut manager = TabManager::new();
        manager.new_ssh_placeholder(&connection);

        let debug = format!("{:?}", manager.tabs[0].tab_type);

        assert!(!debug.contains("password-sentinel"));
        assert!(!debug.contains("key-path-sentinel"));
    }

    #[test]
    fn active_monitor_key_follows_the_active_tab_and_handles_invalid_indices() {
        let mut manager = TabManager::new();
        let ssh_a = test_ssh_connection_for("alpha.example", "alice", 22);
        let ssh_b = test_ssh_connection_for("beta.example", "bob", 2200);

        manager.new_local("sh", 80, 24);
        manager.new_ssh_placeholder(&ssh_a);
        manager.new_ssh_placeholder(&ssh_b);

        manager.switch_to(0);
        assert_eq!(manager.active_monitor_key(), MonitorKey::Local);
        manager.switch_to(1);
        assert_eq!(
            manager.active_monitor_key(),
            MonitorKey::remote("alice", "alpha.example", 22)
        );
        manager.switch_to(2);
        assert_eq!(
            manager.active_monitor_key(),
            MonitorKey::remote("bob", "beta.example", 2200)
        );
        manager.switch_to(99);
        assert_eq!(
            manager.active_monitor_key(),
            MonitorKey::remote("bob", "beta.example", 2200)
        );

        let empty = TabManager::new();
        assert_eq!(empty.active_monitor_key(), MonitorKey::Local);
    }
