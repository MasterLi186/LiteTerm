use super::*;

fn ssh_connection() -> sidebar::SshConnection {
    sidebar::SshConnection {
        label: "测试".into(),
        host: "example.invalid".into(),
        port: 22,
        user: "tester".into(),
        auth: "agent".into(),
        key_path: String::new(),
        password: String::new(),
        group: String::new(),
        group_color: [0, 0, 0],
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn expired_api_arm_skips_handler_and_user_event_postlude() {
    let (dispatch_tx, dispatch_rx) = mpsc::sync_channel(1);
    let bridge = api::Bridge::new(
        move |call| dispatch_tx.send(call).map_err(|error| Box::new(error.0)),
        Duration::from_millis(5),
    );
    let request = tokio::spawn(async move { bridge.call(api::ApiOperation::ListTabs).await });
    let call = dispatch_rx
        .recv_timeout(Duration::from_secs(1))
        .expect("call should be queued");
    tokio::time::sleep(Duration::from_millis(20)).await;
    assert!(request.await.unwrap().is_err());

    let completion_epoch = std::cell::Cell::new(41_u64);
    let pending_sidebar_ssh = std::cell::Cell::new(true);
    let delivered = dispatch_current_api_user_event(call, |_| {
        completion_epoch.set(99);
        pending_sidebar_ssh.set(false);
        true
    });
    if delivered {
        // Mirrors the do_render/check_ssh_connect postlude after the real
        // UserEvent match. The expired arm must return before this point.
        completion_epoch.set(completion_epoch.get() + 1);
        pending_sidebar_ssh.set(false);
    }
    assert!(!delivered);
    assert_eq!(completion_epoch.get(), 41);
    assert!(pending_sidebar_ssh.get());
}

#[tokio::test]
async fn undelivered_api_event_returns_service_unavailable() {
    let bridge = api::Bridge::with_default_timeout(|call| {
        cleanup_undelivered_user_event(UserEvent::Api(call));
        Ok(())
    });
    let error = bridge
        .call(api::ApiOperation::ListTabs)
        .await
        .expect_err("closed event loop should fail");
    assert_eq!(error.status(), axum::http::StatusCode::SERVICE_UNAVAILABLE);
}

#[test]
fn api_server_thread_starts_and_stops_with_isolated_discovery() {
    let temp = tempfile::tempdir().unwrap();
    let config_dir = temp.path().join("api");
    let bridge = api::Bridge::with_default_timeout(|call| Err::<(), _>(Box::new(call)));
    let mut server = HttpApiServer::start_with_bridge(
        api::ApiServerConfig::new(config_dir.clone(), 0),
        bridge,
        api::OutputRegistry::new(),
    )
    .unwrap();
    let deadline = Instant::now() + Duration::from_secs(2);
    while !config_dir.join(api::PORT_FILE_NAME).exists() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    let discovery: api::DiscoveryPort =
        serde_json::from_slice(&std::fs::read(config_dir.join(api::PORT_FILE_NAME)).unwrap())
            .unwrap();
    assert_ne!(discovery.port, 0);
    server.stop();
    let deadline = Instant::now() + Duration::from_secs(2);
    while (config_dir.join(api::TOKEN_FILE_NAME).exists()
        || config_dir.join(api::PORT_FILE_NAME).exists())
        && Instant::now() < deadline
    {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(!config_dir.join(api::TOKEN_FILE_NAME).exists());
    assert!(!config_dir.join(api::PORT_FILE_NAME).exists());
}

#[test]
fn api_ssh_credential_scope_survives_reconnect_split_and_duplicate_until_close() {
    let mut ephemeral_tabs = HashSet::new();
    ephemeral_tabs.insert("api-tab".to_string());

    assert!(!api_tab_allows_keyring(&ephemeral_tabs, "api-tab"));
    // Reconnect and split reuse the same tab identity.
    assert!(!api_tab_allows_keyring(&ephemeral_tabs, "api-tab"));
    assert!(!api_tab_allows_keyring(&ephemeral_tabs, "api-tab"));

    propagate_api_tab_credential_scope(&mut ephemeral_tabs, "api-tab", "duplicate-tab");
    assert!(!api_tab_allows_keyring(&ephemeral_tabs, "duplicate-tab"));

    clear_api_tab_credential_scope(&mut ephemeral_tabs, "api-tab");
    assert!(api_tab_allows_keyring(&ephemeral_tabs, "api-tab"));
    assert!(!api_tab_allows_keyring(&ephemeral_tabs, "duplicate-tab"));
    clear_api_tab_credential_scope(&mut ephemeral_tabs, "duplicate-tab");
    assert!(ephemeral_tabs.is_empty());
}

#[test]
fn pane_resolution_enforces_owner_and_defaults_to_active() {
    let mut manager = TabManager::new();
    let tab_id = manager.new_ssh_placeholder(&ssh_connection());
    assert_eq!(
        resolve_api_pane_in(&manager, &tab_id, None).unwrap(),
        tab_id
    );
    let error = resolve_api_pane_in(&manager, &tab_id, Some("other")).unwrap_err();
    assert_eq!(error.status(), axum::http::StatusCode::NOT_FOUND);
}

#[test]
fn list_dto_contains_panes_active_and_type() {
    let mut manager = TabManager::new();
    let tab_id = manager.new_ssh_placeholder(&ssh_connection());
    let dto = api_tab_dto(&manager.tabs[0]);
    assert_eq!(dto.id, tab_id);
    assert_eq!(dto.kind, "ssh");
    assert_eq!(dto.active_pane_id.as_deref(), Some(tab_id.as_str()));
    assert_eq!(dto.panes.len(), 1);
    assert!(dto.panes[0].active);
}

#[test]
fn invalid_local_path_is_a_bad_request_without_panicking() {
    let result = std::panic::catch_unwind(|| validate_api_shell_path("relative-shell"));
    let error = result.unwrap().unwrap_err();
    assert_eq!(error.status(), axum::http::StatusCode::BAD_REQUEST);
}

#[test]
fn write_failures_have_stable_http_statuses() {
    assert_eq!(
        map_api_write_error("ZMODEM 独占传输").status(),
        axum::http::StatusCode::CONFLICT
    );
    assert_eq!(
        map_api_write_error("终端写入队列已满").status(),
        axum::http::StatusCode::TOO_MANY_REQUESTS
    );
    assert_eq!(
        map_api_write_error("终端写入队列已断开").status(),
        axum::http::StatusCode::NOT_FOUND
    );
}
