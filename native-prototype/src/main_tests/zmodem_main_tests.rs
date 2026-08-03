use super::*;

fn session(generation: u64, token: &str) -> CompletionSessionKey {
    CompletionSessionKey::new_for_test(generation, token)
}

fn slot(
    tab_id: &str,
    pane_id: &str,
    session: CompletionSessionKey,
    commands: zmodem::runtime::RuntimeCommandSender,
) -> ZmodemControlSlot {
    ZmodemControlSlot {
        tab_id: tab_id.into(),
        pane_id: pane_id.into(),
        session,
        commands,
        pending_send: None,
        capability: zmodem::runtime::RuntimeCapability::Local,
        unavailable_reason: None,
    }
}

fn event(
    transfer_id: u64,
    generation: u64,
    kind: zmodem::runtime::RuntimeEventKind,
) -> zmodem::runtime::RuntimeEvent {
    zmodem::runtime::RuntimeEvent {
        identity: zmodem::runtime::TransferIdentity {
            transfer_id,
            generation,
        },
        kind,
    }
}

#[test]
fn zmodem_identity_gate_rejects_each_stale_dimension() {
    let current = session(4, "current");
    let stale_token = session(4, "stale-token");
    let (commands, _receiver) = zmodem::runtime::runtime_command_channel();
    let slot = slot("tab-a", "pane-a", current.clone(), commands);
    let is_current = |tab_exists,
                      pane_exists,
                      pane_session: Option<&CompletionSessionKey>,
                      event_session: &CompletionSessionKey,
                      generation| {
        zmodem_identity_components_are_current(
            tab_exists,
            pane_exists,
            pane_session,
            Some(&slot),
            "tab-a",
            "pane-a",
            event_session,
            generation,
        )
    };

    assert!(is_current(true, true, Some(&current), &current, 4));
    assert!(!is_current(false, true, Some(&current), &current, 4));
    assert!(!is_current(true, false, Some(&current), &current, 4));
    assert!(!is_current(true, true, Some(&current), &stale_token, 4));
    assert!(!is_current(true, true, Some(&current), &current, 3));
}

#[test]
fn zmodem_control_try_send_reports_full_and_disconnected() {
    let (commands, receiver) = zmodem::runtime::runtime_command_channel();
    for _ in 0..zmodem::runtime::RUNTIME_COMMAND_CAPACITY {
        assert_eq!(
            try_send_zmodem_command(&commands, zmodem::runtime::RuntimeCommand::Shutdown),
            Ok(())
        );
    }
    assert_eq!(
        try_send_zmodem_command(&commands, zmodem::runtime::RuntimeCommand::Shutdown),
        Err(ZmodemControlSendError::Full)
    );
    drop(receiver);
    assert_eq!(
        try_send_zmodem_command(&commands, zmodem::runtime::RuntimeCommand::Shutdown),
        Err(ZmodemControlSendError::Disconnected)
    );
}

#[test]
fn zmodem_settings_publish_updates_one_shared_snapshot() {
    let before = settings::Settings::default();
    let source =
        zmodem::runtime::RuntimeSettingsSource::new(zmodem_runtime_settings(&before).unwrap());
    let mut after = before;
    after.zmodem.enabled = false;
    after.zmodem.auto_detect = false;
    after.zmodem.download_dir = "/var/tmp/zmodem-updated".into();
    after.zmodem.timeout_secs = 180;

    persist_and_publish_zmodem_settings(&source, &after, true, |_| Ok(())).unwrap();
    let (version, published) = source.snapshot();
    assert_eq!(version, 1);
    assert_eq!(
        published,
        zmodem::runtime::RuntimeSettings {
            enabled: false,
            auto_detect: false,
            receive_directory: "/var/tmp/zmodem-updated".into(),
            transfer_timeout: Some(Duration::from_secs(180)),
        }
    );
}

#[test]
fn failed_settings_save_does_not_publish_runtime_snapshot() {
    let before = settings::Settings::default();
    let source =
        zmodem::runtime::RuntimeSettingsSource::new(zmodem_runtime_settings(&before).unwrap());
    let mut after = before.clone();
    after.zmodem.auto_detect = false;
    after.zmodem.download_dir = "/var/tmp/zmodem-updated".into();
    after.zmodem.timeout_secs = 90;

    let error = persist_and_publish_zmodem_settings(&source, &after, true, |_| {
        Err(std::io::Error::other("injected save failure"))
    })
    .unwrap_err();

    assert!(error.contains("injected save failure"));
    let (version, published) = source.snapshot();
    assert_eq!(version, 0);
    assert_eq!(published, zmodem_runtime_settings(&before).unwrap());
    assert!(
        before.zmodem.auto_detect,
        "caller UI settings stay unchanged"
    );
}

#[test]
fn zmodem_slot_replacement_and_cleanup_shutdown_old_controls() {
    let current = session(1, "current");
    let (old_commands, old_receiver) = zmodem::runtime::runtime_command_channel();
    let (new_commands, new_receiver) = zmodem::runtime::runtime_command_channel();
    let mut slots = HashMap::new();
    let mut views = HashMap::new();
    slots.insert(
        "pane-a".into(),
        slot("tab-a", "pane-a", current.clone(), old_commands),
    );
    views.insert("pane-a".into(), zmodem::ui::PaneZmodemView::default());

    replace_zmodem_slot(
        &mut slots,
        &mut views,
        slot("tab-a", "pane-a", current, new_commands),
    );
    assert!(matches!(
        old_receiver.try_recv(),
        Ok(zmodem::runtime::RuntimeCommand::Shutdown)
    ));
    assert!(!views.contains_key("pane-a"));

    remove_zmodem_pane_resources(&mut slots, &mut views, "pane-a");
    assert!(matches!(
        new_receiver.try_recv(),
        Ok(zmodem::runtime::RuntimeCommand::Shutdown)
    ));
    assert!(!slots.contains_key("pane-a"));
}

#[test]
fn zmodem_capability_explains_settings_serial_and_readiness() {
    use zmodem::runtime::RuntimeCapability;
    assert_eq!(
        zmodem_ui_capability(RuntimeCapability::Local, true, true, None),
        zmodem::ui::ZmodemCapability::Enabled
    );
    assert_eq!(
        zmodem_ui_capability(RuntimeCapability::Local, false, true, None).disabled_reason(),
        Some("设置中已禁用 ZMODEM")
    );
    assert_eq!(
        zmodem_ui_capability(RuntimeCapability::SerialDisabled, true, true, None).disabled_reason(),
        Some("串口终端不支持 ZMODEM")
    );
    assert_eq!(
        zmodem_ui_capability(RuntimeCapability::DirectSsh, true, false, None).disabled_reason(),
        Some("ZMODEM 控制通道尚未就绪")
    );
}

#[test]
fn zmodem_transfer_ids_exhaust_without_reusing_u64_max() {
    let mut next = Some(u64::MAX);
    assert_eq!(allocate_zmodem_transfer_id(&mut next), Ok(u64::MAX));
    assert_eq!(next, None);
    assert_eq!(
        allocate_zmodem_transfer_id(&mut next),
        Err("ZMODEM 传输编号已耗尽，请重启应用")
    );
    observe_zmodem_transfer_id(&mut next, 1);
    assert_eq!(next, None);

    let mut observed = Some(7);
    observe_zmodem_transfer_id(&mut observed, u64::MAX);
    assert_eq!(observed, None);
}

#[test]
fn queued_auto_receive_started_wins_over_unconfirmed_manual_send() {
    use zmodem::runtime::{RuntimeCommand, RuntimeEventKind, TransferDirection};

    let (commands, receiver) = zmodem::runtime::runtime_command_channel();
    let mut pending_send = None;
    let mut next_transfer_id = Some(12);
    let mut view = zmodem::ui::PaneZmodemView::default();

    assert!(request_zmodem_send(
        &commands,
        &mut pending_send,
        &mut view,
        &mut next_transfer_id,
        4,
        vec!["/tmp/manual.bin".into()],
    ));
    assert_eq!(pending_send, Some(12));
    assert!(
        view.transfer.is_none(),
        "Started 前不能创建发送 TransferView"
    );
    assert!(matches!(
        receiver.try_recv(),
        Ok(RuntimeCommand::StartSend {
            identity: zmodem::runtime::TransferIdentity {
                transfer_id: 12,
                generation: 4
            },
            ..
        })
    ));

    assert!(apply_zmodem_runtime_event_arbitrated(
        &mut view,
        &mut pending_send,
        event(
            11,
            4,
            RuntimeEventKind::Started {
                direction: TransferDirection::Receive,
                filename: None,
                total: None,
            },
        ),
    ));
    assert_eq!(view.active_transfer_id(), Some(11));
    assert_eq!(pending_send, Some(12));

    assert!(apply_zmodem_runtime_event_arbitrated(
        &mut view,
        &mut pending_send,
        event(
            12,
            4,
            RuntimeEventKind::Error(zmodem::ZmodemError::Protocol(
                "已有 ZMODEM 传输正在进行".into(),
            )),
        ),
    ));
    assert_eq!(pending_send, None);
    assert_eq!(
        view.active_transfer_id(),
        Some(11),
        "被 runtime 拒绝的较高发送 ID 不能覆盖真实自动接收"
    );
}

#[test]
fn active_auto_receive_rejects_manual_send_without_queueing_or_allocating_id() {
    use zmodem::runtime::{RuntimeEventKind, TransferDirection};

    let (commands, receiver) = zmodem::runtime::runtime_command_channel();
    let mut pending_send = None;
    let mut next_transfer_id = Some(21);
    let mut view = zmodem::ui::PaneZmodemView::default();
    assert!(apply_zmodem_runtime_event_arbitrated(
        &mut view,
        &mut pending_send,
        event(
            20,
            8,
            RuntimeEventKind::Started {
                direction: TransferDirection::Receive,
                filename: None,
                total: None,
            },
        ),
    ));

    assert!(!request_zmodem_send(
        &commands,
        &mut pending_send,
        &mut view,
        &mut next_transfer_id,
        8,
        vec!["/tmp/manual.bin".into()],
    ));
    assert!(receiver.try_recv().is_err());
    assert_eq!(next_transfer_id, Some(21));
    assert_eq!(pending_send, None);
    assert_eq!(view.active_transfer_id(), Some(20));
}

#[test]
fn reader_eof_rejection_clears_pending_and_allows_a_later_send_request() {
    use zmodem::runtime::RuntimeEventKind;

    let (commands, receiver) = zmodem::runtime::runtime_command_channel();
    let mut pending_send = Some(40);
    let mut next_transfer_id = Some(41);
    let mut view = zmodem::ui::PaneZmodemView::default();

    assert!(apply_zmodem_runtime_event_arbitrated(
        &mut view,
        &mut pending_send,
        event(
            40,
            7,
            RuntimeEventKind::Error(zmodem::ZmodemError::Protocol(
                "终端连接已结束，无法开始 ZMODEM 发送".into(),
            )),
        ),
    ));
    assert_eq!(pending_send, None);

    assert!(request_zmodem_send(
        &commands,
        &mut pending_send,
        &mut view,
        &mut next_transfer_id,
        8,
        vec!["/tmp/later.bin".into()],
    ));
    assert_eq!(pending_send, Some(41));
    assert!(matches!(
        receiver.try_recv(),
        Ok(zmodem::runtime::RuntimeCommand::StartSend {
            identity: zmodem::runtime::TransferIdentity {
                transfer_id: 41,
                generation: 8
            },
            ..
        })
    ));
}

#[test]
fn replacement_drops_pending_send_and_stale_generation_cannot_build_view() {
    let old_session = session(3, "old");
    let new_session = session(4, "new");
    let (old_commands, old_receiver) = zmodem::runtime::runtime_command_channel();
    let (new_commands, _new_receiver) = zmodem::runtime::runtime_command_channel();
    let mut old_slot = slot("tab-a", "pane-a", old_session.clone(), old_commands);
    old_slot.pending_send = Some(30);
    let mut slots = HashMap::from([("pane-a".into(), old_slot)]);
    let mut views = HashMap::new();

    replace_zmodem_slot(
        &mut slots,
        &mut views,
        slot("tab-a", "pane-a", new_session.clone(), new_commands),
    );
    assert!(matches!(
        old_receiver.try_recv(),
        Ok(zmodem::runtime::RuntimeCommand::Shutdown)
    ));
    assert_eq!(slots["pane-a"].pending_send, None);
    assert!(!zmodem_identity_components_are_current(
        true,
        true,
        Some(&new_session),
        slots.get("pane-a"),
        "tab-a",
        "pane-a",
        &old_session,
        3,
    ));
    assert!(views.get("pane-a").is_none());
}

#[test]
fn multi_file_completion_stays_active_until_all_complete() {
    use zmodem::runtime::{RuntimeEventKind, TransferDirection};
    use zmodem::sender::SenderAction;
    use zmodem::ui::TransferStatus;

    let mut view = zmodem::ui::PaneZmodemView::default();
    assert!(apply_zmodem_runtime_event(
        &mut view,
        event(
            10,
            2,
            RuntimeEventKind::Started {
                direction: TransferDirection::Send,
                filename: Some("first.bin".into()),
                total: Some(5),
            },
        ),
    ));
    assert!(apply_zmodem_runtime_event(
        &mut view,
        event(
            10,
            2,
            RuntimeEventKind::Sender(SenderAction::FileComplete("first.bin".into())),
        ),
    ));
    assert_eq!(view.active_transfer_id(), Some(10));
    assert_eq!(
        view.transfer.as_ref().unwrap().status,
        TransferStatus::Transferring
    );

    assert!(apply_zmodem_runtime_event(
        &mut view,
        event(
            10,
            2,
            RuntimeEventKind::Sender(SenderAction::Progress {
                bytes_sent: 3,
                total: 8,
                filename: "second.bin".into(),
            }),
        ),
    ));
    assert!(!apply_zmodem_runtime_event(
        &mut view,
        event(
            9,
            2,
            RuntimeEventKind::Sender(SenderAction::Progress {
                bytes_sent: 99,
                total: 100,
                filename: "stale.bin".into(),
            }),
        ),
    ));
    assert_eq!(view.transfer.as_ref().unwrap().filename, "second.bin");
    assert!(apply_zmodem_runtime_event(
        &mut view,
        event(10, 2, RuntimeEventKind::Sender(SenderAction::AllComplete),),
    ));
    assert_eq!(
        view.transfer.as_ref().unwrap().status,
        TransferStatus::Completed
    );
}

#[test]
fn receiver_file_complete_stays_active_and_error_without_started_is_visible() {
    use zmodem::receiver::ReceiverEvent;
    use zmodem::runtime::{RuntimeEventKind, TransferDirection};
    use zmodem::ui::TransferStatus;

    let mut view = zmodem::ui::PaneZmodemView::default();
    assert!(apply_zmodem_runtime_event(
        &mut view,
        event(
            12,
            3,
            RuntimeEventKind::Started {
                direction: TransferDirection::Receive,
                filename: Some("one.bin".into()),
                total: Some(4),
            },
        ),
    ));
    assert!(apply_zmodem_runtime_event(
        &mut view,
        event(
            12,
            3,
            RuntimeEventKind::Receiver(ReceiverEvent::FileComplete {
                path: "/tmp/one.bin".into(),
                size: 4,
            }),
        ),
    ));
    assert_eq!(view.active_transfer_id(), Some(12));
    assert_eq!(
        view.transfer.as_ref().unwrap().status,
        TransferStatus::Transferring
    );

    let mut failed = zmodem::ui::PaneZmodemView::default();
    assert!(apply_zmodem_runtime_event(
        &mut failed,
        event(
            13,
            3,
            RuntimeEventKind::Error(zmodem::ZmodemError::Protocol("接收目录不可用".into())),
        ),
    ));
    assert!(matches!(
        failed.transfer.as_ref().map(|transfer| &transfer.status),
        Some(TransferStatus::Failed(error)) if error == "接收目录不可用"
    ));
}

#[test]
fn zmodem_user_event_debug_redacts_paths_errors_and_session_token() {
    let user_event = UserEvent::Zmodem {
        tab_id: "tab-a".into(),
        pane_id: "pane-a".into(),
        session: session(5, "secret-session-token"),
        event: event(
            9,
            5,
            zmodem::runtime::RuntimeEventKind::Error(zmodem::ZmodemError::Protocol(
                "/secret/path/file.bin".into(),
            )),
        ),
    };
    let debug = format!("{user_event:?}");
    assert!(!debug.contains("secret-session-token"));
    assert!(!debug.contains("/secret/path"));
    assert!(debug.contains("transfer_id"));
    assert!(debug.contains("Error"));
}
