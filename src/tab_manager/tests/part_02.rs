    #[test]
    fn process_tabs_are_unique_by_monitor_key_and_switch_to_the_existing_tab() {
        let mut manager = TabManager::new();
        let alice = crate::ssh::ConnectionParams::from(&test_ssh_connection_for(
            "shared.example",
            "alice",
            22,
        ));
        let same_target = alice.clone();
        let bob = crate::ssh::ConnectionParams::from(&test_ssh_connection_for(
            "shared.example",
            "bob",
            22,
        ));

        let alice_key = MonitorKey::from_ssh(&alice);
        let bob_key = MonitorKey::from_ssh(&bob);
        let first = manager.open_process("alice 的进程", alice_key.clone(), Some(alice));
        manager.new_ssh_placeholder(&test_ssh_connection());
        let duplicate = manager.open_process("不应覆盖原标签", alice_key, Some(same_target));
        let distinct = manager.open_process("bob 的进程", bob_key, Some(bob));

        assert_eq!(first, duplicate);
        assert_ne!(first, distinct);
        assert_eq!(manager.len(), 3);
        assert_eq!(
            manager.active().map(|tab| tab.id.as_str()),
            Some(distinct.as_str())
        );
        assert_eq!(manager.tabs[0].label, "alice 的进程");
    }

    #[test]
    fn local_and_remote_process_tabs_preserve_identity_and_lease_rules() {
        let mut manager = TabManager::new();
        let params = crate::ssh::ConnectionParams::from(&test_ssh_connection_for(
            "process.example",
            "alice",
            2200,
        ));
        let remote_key = MonitorKey::from_ssh(&params);

        let local_id = manager.open_process("本机进程", MonitorKey::Local, None);
        let duplicate_local = manager.open_process("不应覆盖", MonitorKey::Local, None);
        let remote_id = manager.open_process("远端进程", remote_key.clone(), Some(params.clone()));

        assert_eq!(local_id, duplicate_local);
        assert_ne!(local_id, remote_id);
        assert_eq!(manager.len(), 2);
        assert_eq!(manager.tabs[0].monitor_key(), MonitorKey::Local);
        assert!(!manager.tabs[0].remote_monitor_leased);
        assert_eq!(manager.tabs[0].tab_type.remote_params(), None);
        assert_eq!(manager.tabs[1].monitor_key(), remote_key.clone());
        assert!(manager.tabs[1].remote_monitor_leased);
        assert_eq!(manager.tabs[1].tab_type.remote_params(), Some(&params));
        assert_eq!(
            manager.remote_monitor_requirements().get(&remote_key),
            Some(&params)
        );
    }

    #[test]
    fn process_tab_is_a_non_terminal_monitor_lease() {
        let mut manager = TabManager::new();
        let params = crate::ssh::ConnectionParams::from(&test_ssh_connection_for(
            "process.example",
            "alice",
            2200,
        ));
        let key = MonitorKey::from_ssh(&params);

        let process_id = manager.open_process("远端进程", key.clone(), Some(params.clone()));

        assert_eq!(
            manager.active().map(|tab| tab.id.as_str()),
            Some(process_id.as_str())
        );
        assert!(manager.active_terminal().is_none());
        assert_eq!(
            manager.remote_monitor_requirements().get(&key),
            Some(&params)
        );
        assert!(manager
            .reset_ssh_for_reconnect(manager.active_idx)
            .is_none());
    }

    #[test]
    fn process_lease_survives_closing_its_source_ssh_tab() {
        let mut manager = TabManager::new();
        let connection = test_ssh_connection_for("lease.example", "alice", 22);
        let ssh_id = manager.new_ssh_placeholder(&connection);
        let session = manager.tabs[0].completion.session().clone();
        assert!(manager
            .apply_ssh(&ssh_id, &session, test_ssh_handle(None).0, 80, 24)
            .is_some());
        let params = crate::ssh::ConnectionParams::from(&connection);
        let process_id =
            manager.open_process("远端进程", MonitorKey::from_ssh(&params), Some(params));

        manager.close(0);

        assert_eq!(
            manager.active().map(|tab| tab.id.as_str()),
            Some(process_id.as_str())
        );
        assert_eq!(manager.remote_monitor_requirements().len(), 1);
        manager.close(0);
        assert!(manager.remote_monitor_requirements().is_empty());
    }

    #[test]
    fn resize_all_does_not_resize_process_placeholder_terminal() {
        let mut manager = TabManager::new();
        let params = crate::ssh::ConnectionParams::from(&test_ssh_connection());
        manager.open_process("远端进程", MonitorKey::from_ssh(&params), Some(params));
        let before = {
            let terminal = manager.tabs[0].terminal.lock().unwrap();
            (terminal.cols(), terminal.rows())
        };

        manager.resize_all(132, 41);

        let terminal = manager.tabs[0].terminal.lock().unwrap();
        assert_eq!((terminal.cols(), terminal.rows()), before);
    }

    #[test]
    fn process_tab_debug_redacts_authentication_material() {
        let mut params = crate::ssh::ConnectionParams::from(&test_ssh_connection());
        params.key_path = "PROCESS_KEY_PATH_SENTINEL".into();
        params.password = "PROCESS_PASSWORD_SENTINEL".into();
        let tab_type = TabType::Process {
            label: "远端进程".into(),
            key: MonitorKey::from_ssh(&params),
            params: Some(params),
        };

        let debug = format!("{tab_type:?}");

        assert!(debug.contains("远端进程"));
        assert!(!debug.contains("PROCESS_KEY_PATH_SENTINEL"));
        assert!(!debug.contains("PROCESS_PASSWORD_SENTINEL"));
    }

    /// P0 Task 3 RED: `resize_all` 必须把所有标签页终端网格更新为给定尺寸。
    /// 使用 `new_ssh_placeholder` 避免启动本地 PTY；方法尚不存在时应编译失败。
    #[test]
    fn resize_all_applies_grid_to_all_tabs() {
        let mut manager = TabManager::new();
        manager.new_ssh_placeholder(&test_ssh_connection_for("a.example", "alice", 22));
        manager.new_ssh_placeholder(&test_ssh_connection_for("b.example", "bob", 22));
        assert_eq!(manager.tabs.len(), 2);

        manager.resize_all(132, 41);

        for tab in &manager.tabs {
            let terminal = tab.terminal.lock().unwrap();
            assert_eq!(terminal.cols(), 132);
            assert_eq!(terminal.rows(), 41);
        }
    }

    // =========================================================================
    // P0 Task 4 RED-C: per-tab TerminalSearchState isolation
    // Each Tab owns independent `search`. GREEN must add the field and default
    // it empty+hidden on new_local / new_ssh_placeholder. Filter: search_
    // =========================================================================

    fn assert_search_default_empty_hidden(search: &crate::terminal_search::TerminalSearchState) {
        assert!(!search.visible, "new tab search must be hidden");
        assert!(
            search.query.is_empty(),
            "new tab search query must be empty"
        );
        assert!(
            search.matches.is_empty(),
            "new tab search matches must be empty"
        );
        assert_eq!(search.current, None, "new tab search current must be None");
        assert!(
            !search.case_sensitive,
            "new tab search defaults to case-insensitive"
        );
    }

    /// New local tab must own a default-empty, hidden TerminalSearchState.
    #[test]
    fn search_new_local_defaults_empty_and_hidden() {
        let mut manager = TabManager::new();
        manager.new_local("sh", 80, 24);

        assert_search_default_empty_hidden(&manager.tabs[0].search);
    }

    /// New SSH placeholder must own a default-empty, hidden TerminalSearchState.
    #[test]
    fn search_new_ssh_placeholder_defaults_empty_and_hidden() {
        let mut manager = TabManager::new();
        manager.new_ssh_placeholder(&test_ssh_connection());

        assert_search_default_empty_hidden(&manager.tabs[0].search);
    }

    /// Mutating one tab's query/visible must not affect another tab's search.
    #[test]
    fn search_state_is_isolated_across_tabs() {
        let mut manager = TabManager::new();
        manager.new_local("sh", 80, 24);
        manager.new_ssh_placeholder(&test_ssh_connection_for("iso.example", "alice", 22));
        assert_eq!(manager.tabs.len(), 2);

        manager.tabs[0].search.visible = true;
        manager.tabs[0].search.query = "needle-a".into();
        manager.tabs[0].search.replace_results(
            "needle-a",
            vec![crate::terminal_search::SearchMatch {
                line: 0,
                start_col: 0,
                end_col: 7,
            }],
        );

        // Peer tab remains default.
        assert_search_default_empty_hidden(&manager.tabs[1].search);

        // Peer mutations must not reverse-affect the first tab.
        manager.tabs[1].search.visible = true;
        manager.tabs[1].search.query = "needle-b".into();
        manager.tabs[1].search.case_sensitive = true;

        assert!(manager.tabs[0].search.visible);
        assert_eq!(manager.tabs[0].search.query, "needle-a");
        assert_eq!(manager.tabs[0].search.matches.len(), 1);
        assert_eq!(manager.tabs[0].search.current, Some(0));
        assert!(!manager.tabs[0].search.case_sensitive);

        assert!(manager.tabs[1].search.visible);
        assert_eq!(manager.tabs[1].search.query, "needle-b");
        assert!(manager.tabs[1].search.matches.is_empty());
        assert!(manager.tabs[1].search.case_sensitive);
    }

    /// switch_to only repositions active_idx; each tab retains its own search.
    #[test]
    fn search_survives_switch_to_and_remains_per_tab() {
        let mut manager = TabManager::new();
        manager.new_ssh_placeholder(&test_ssh_connection_for("a.example", "alice", 22));
        manager.new_ssh_placeholder(&test_ssh_connection_for("b.example", "bob", 22));

        manager.tabs[0].search.visible = true;
        manager.tabs[0].search.query = "alpha".into();
        manager.tabs[1].search.query = "beta".into();

        manager.switch_to(1);
        assert_eq!(manager.active_idx, 1);
        assert_eq!(
            manager.active().map(|t| t.search.query.as_str()),
            Some("beta")
        );
        assert!(!manager.tabs[1].search.visible);
        assert!(manager.tabs[0].search.visible);
        assert_eq!(manager.tabs[0].search.query, "alpha");

        manager.switch_to(0);
        assert_eq!(
            manager.active().map(|t| t.search.query.as_str()),
            Some("alpha")
        );
        assert_eq!(manager.tabs[1].search.query, "beta");
    }

    /// Duplicate-style construction (two independent tabs of same origin) keeps
    /// independent search state — no shared search object across tab ids.
    #[test]
    fn search_duplicate_ssh_placeholders_are_independent() {
        let mut manager = TabManager::new();
        let conn = test_ssh_connection();
        let first_id = manager.new_ssh_placeholder(&conn);
        let second_id = manager.new_ssh_placeholder(&conn);
        assert_ne!(first_id, second_id);

        let first = manager.find_by_id(&first_id).unwrap();
        let second = manager.find_by_id(&second_id).unwrap();

        manager.tabs[first].search.visible = true;
        manager.tabs[first].search.query = "only-first".into();
        manager.tabs[first].search.replace_results(
            "only-first",
            vec![crate::terminal_search::SearchMatch {
                line: -3,
                start_col: 2,
                end_col: 5,
            }],
        );

        assert_search_default_empty_hidden(&manager.tabs[second].search);
        assert_eq!(manager.tabs[first].search.query, "only-first");
        assert_eq!(manager.tabs[first].search.matches.len(), 1);

        // Clear first; second must still be independently default / untouched.
        manager.tabs[first].search = crate::terminal_search::TerminalSearchState::default();
        manager.tabs[second].search.visible = true;
        manager.tabs[second].search.query = "only-second".into();

        assert_search_default_empty_hidden(&manager.tabs[first].search);
        assert!(manager.tabs[second].search.visible);
        assert_eq!(manager.tabs[second].search.query, "only-second");
    }

    #[test]
    fn reorder_by_id_moves_tabs_before_and_after_without_changing_active_identity() {
        let mut manager = TabManager::new();
        let a = manager.new_ssh_placeholder(&test_ssh_connection_for("a.example", "a", 22));
        let b = manager.new_ssh_placeholder(&test_ssh_connection_for("b.example", "b", 22));
        let c = manager.new_ssh_placeholder(&test_ssh_connection_for("c.example", "c", 22));
        manager.switch_to(1);

        assert!(manager.reorder_by_id(&a, &c, TabPlacement::After));
        assert_eq!(
            manager
                .tabs
                .iter()
                .map(|tab| tab.id.as_str())
                .collect::<Vec<_>>(),
            vec![b.as_str(), c.as_str(), a.as_str()]
        );
        assert_eq!(
            manager.active().map(|tab| tab.id.as_str()),
            Some(b.as_str())
        );

        assert!(manager.reorder_by_id(&a, &b, TabPlacement::Before));
        assert_eq!(
            manager
                .tabs
                .iter()
                .map(|tab| tab.id.as_str())
                .collect::<Vec<_>>(),
            vec![a.as_str(), b.as_str(), c.as_str()]
        );
        assert_eq!(
            manager.active().map(|tab| tab.id.as_str()),
            Some(b.as_str())
        );
    }

    #[test]
    fn reorder_by_id_rejects_invalid_and_noop_moves() {
        let mut manager = TabManager::new();
        let a = manager.new_ssh_placeholder(&test_ssh_connection_for("a.example", "a", 22));
        let b = manager.new_ssh_placeholder(&test_ssh_connection_for("b.example", "b", 22));

        assert!(!manager.reorder_by_id(&a, &a, TabPlacement::Before));
        assert!(!manager.reorder_by_id("missing", &b, TabPlacement::After));
        assert!(!manager.reorder_by_id(&a, "missing", TabPlacement::After));
        assert!(!manager.reorder_by_id(&a, &b, TabPlacement::Before));
        assert_eq!(
            manager
                .tabs
                .iter()
                .map(|tab| tab.id.as_str())
                .collect::<Vec<_>>(),
            vec![a.as_str(), b.as_str()]
        );
    }

    #[test]
    fn split_ssh_pane_has_stable_identity_and_isolated_terminal_state() {
        let mut manager = TabManager::new();
        let tab_id = manager.new_ssh_placeholder(&test_ssh_connection());
        let original_terminal = manager.tabs[0].terminal.clone();
        let original_session = manager.tabs[0].completion.session().clone();

        let SplitPanePlan::Ssh {
            tab_id: planned_tab,
            pane_id,
            session,
            ..
        } = manager
            .split_active_pane(SplitDirection::Vertical, 80, 24)
            .unwrap()
        else {
            panic!("SSH split must produce an SSH connect plan");
        };

        assert_eq!(planned_tab, tab_id);
        assert_ne!(pane_id, tab_id);
        assert_ne!(session, original_session);
        assert_eq!(manager.tabs[0].pane_count(), 2);
        assert_eq!(manager.tabs[0].layout.active_pane_id(), pane_id);
        assert!(!Arc::ptr_eq(&original_terminal, &manager.tabs[0].terminal));
        assert_eq!(
            manager.tabs[0].layout.tree().pane_ids(),
            vec![tab_id, pane_id]
        );
    }

    #[test]
    fn pane_search_is_isolated_and_active_deref_follows_focus() {
        let mut manager = TabManager::new();
        let tab_id = manager.new_ssh_placeholder(&test_ssh_connection());
        let SplitPanePlan::Ssh { pane_id, .. } = manager
            .split_active_pane(SplitDirection::Horizontal, 80, 24)
            .unwrap()
        else {
            panic!("SSH split must produce an SSH connect plan");
        };

        manager.tabs[0].search.query = "secondary".into();
        assert!(manager.set_active_pane(&tab_id, &tab_id));
        manager.tabs[0].search.query = "primary".into();
        assert_eq!(manager.tabs[0].search.query, "primary");
        assert_eq!(
            manager.tabs[0].pane(&pane_id).unwrap().search.query,
            "secondary"
        );
    }

    #[test]
    fn closing_active_pane_collapses_tree_and_restores_sibling_focus() {
        let mut manager = TabManager::new();
        let tab_id = manager.new_ssh_placeholder(&test_ssh_connection());
        let SplitPanePlan::Ssh { pane_id, .. } = manager
            .split_active_pane(SplitDirection::Vertical, 80, 24)
            .unwrap()
        else {
            panic!("SSH split must produce an SSH connect plan");
        };

        assert_eq!(
            manager.close_active_pane(),
            CloseActivePaneResult::Closed {
                tab_id: tab_id.clone(),
                pane_id,
                active_pane_id: tab_id.clone(),
            }
        );
        assert_eq!(manager.tabs[0].pane_count(), 1);
        assert_eq!(manager.tabs[0].layout.tree().pane_ids(), vec![tab_id]);
        assert_eq!(manager.close_active_pane(), CloseActivePaneResult::CloseTab);
    }

    #[test]
    fn terminal_layout_controlled_mutations_preserve_invariants() {
        let mut manager = TabManager::new();
        let tab_id = manager.new_ssh_placeholder(&test_ssh_connection());
        assert!(manager.tabs[0].layout.invariants_hold());

        let SplitPanePlan::Ssh { pane_id, .. } = manager
            .split_active_pane(SplitDirection::Vertical, 80, 24)
            .unwrap()
        else {
            panic!("SSH split must produce an SSH connect plan");
        };
        let tab = &manager.tabs[0];
        let leaf_ids: HashSet<_> = tab.layout.tree().pane_ids().into_iter().collect();
        let map_ids: HashSet<_> = tab.panes().map(|pane| pane.id().to_string()).collect();
        assert_eq!(leaf_ids, map_ids);
        assert!(tab.pane(tab.active_pane_id()).is_some());
        assert!(tab.layout.invariants_hold());

        assert!(manager.set_active_pane(&tab_id, &tab_id));
        assert!(manager.tabs[0].layout.invariants_hold());
        assert!(manager.set_active_pane(&tab_id, &pane_id));
        assert!(matches!(
            manager.close_active_pane(),
            CloseActivePaneResult::Closed { .. }
        ));
        assert!(manager.tabs[0].layout.invariants_hold());
    }

    #[test]
    fn ssh_failure_is_session_and_pane_scoped() {
        let mut manager = TabManager::new();
        let tab_id = manager.new_ssh_placeholder(&test_ssh_connection());
        let primary_session = manager.tabs[0].completion.session().clone();
        let SplitPanePlan::Ssh {
            pane_id,
            session: secondary_session,
            ..
        } = manager
            .split_active_pane(SplitDirection::Vertical, 80, 24)
            .unwrap()
        else {
            panic!("SSH split must produce an SSH connect plan");
        };

        assert!(manager
            .apply_ssh_pane(
                &tab_id,
                &tab_id,
                &primary_session,
                test_ssh_handle(None).0,
                80,
                24,
            )
            .is_some());
        let stale_session = secondary_session.successor();
        assert!(!manager.ssh_failed(&tab_id, &pane_id, &stale_session, "stale failure"));
        assert_eq!(
            manager.tabs[0].pane(&pane_id).unwrap().status,
            PaneStatus::Connecting
        );

        assert!(manager.ssh_failed(&tab_id, &pane_id, &secondary_session, "secondary failed"));
        assert_eq!(
            manager.tabs[0].pane(&tab_id).unwrap().status,
            PaneStatus::Connected
        );
        assert!(manager.tabs[0].pane(&tab_id).unwrap().ssh_connected);
        assert_eq!(
            manager.tabs[0].pane(&pane_id).unwrap().status,
            PaneStatus::Failed("secondary failed".into())
        );
        assert!(!manager.tabs[0].pane(&pane_id).unwrap().ssh_connected);
        assert_eq!(manager.tabs[0].label, "测试");
    }
