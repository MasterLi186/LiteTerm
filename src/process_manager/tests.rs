use super::*;
use crate::monitor::{ProcessAncestor, ProcessEnvironment, ProcessIdentity};

const DETAIL_START: &str = "Mon Jul 27 10:00:00 2026";

fn process(
    pid: u32,
    user: &str,
    memory: u64,
    cpu: f32,
    command: &str,
    start_time: &str,
) -> ProcessInfo {
    ProcessInfo {
        pid,
        user: user.to_string(),
        state: "S".to_string(),
        mem_mb: format!("{}K", memory / 1024),
        mem_bytes: memory,
        resident_mem_mb: format!("{}K", memory / 1024),
        resident_mem_bytes: memory,
        cpu,
        name: command.to_string(),
        command: command.to_string(),
        start_time: start_time.to_string(),
    }
}

fn state() -> ProcessManagerState {
    ProcessManagerState::new(MonitorKey::remote("dev", "example.com", 22))
}

fn detail(pid: u32, secret: &str) -> ProcessDetail {
    ProcessDetail {
        identity: ProcessIdentity {
            pid,
            start_ticks: 88,
        },
        user: "dev".to_string(),
        state: "S".to_string(),
        mem_mb: "2M".to_string(),
        mem_bytes: 2 * 1024 * 1024,
        platform_memory: None,
        cpu: 0.5,
        name: "worker".to_string(),
        command: format!("worker --token={secret}"),
        executable: "/opt/worker".to_string(),
        working_dir: "/srv/private".to_string(),
        start_time: DETAIL_START.to_string(),
        environ: vec![ProcessEnvironment {
            key: "TOKEN".to_string(),
            value: secret.to_string(),
        }],
        ancestors: vec![ProcessAncestor {
            pid: 1,
            name: "systemd".to_string(),
            command: "/sbin/init".to_string(),
        }],
    }
}

#[test]
fn defaults_to_cpu_descending() {
    let state = state();
    assert_eq!(state.sort_key(), ProcessSortKey::Cpu);
    assert_eq!(state.sort_direction(), SortDirection::Descending);

    let processes = vec![
        process(1, "a", 1, 1.0, "slow", "2026-01-01"),
        process(2, "b", 2, 30.0, "fast", "2026-01-02"),
    ];
    let sorted = sorted_processes(&processes, state.sort_key, state.sort_direction);
    assert_eq!(
        sorted.iter().map(|process| process.pid).collect::<Vec<_>>(),
        vec![2, 1]
    );
}

#[test]
fn current_column_toggles_and_new_column_starts_descending() {
    let mut state = state();
    state.select_sort(ProcessSortKey::Cpu);
    assert_eq!(state.sort_direction(), SortDirection::Ascending);
    state.select_sort(ProcessSortKey::Pid);
    assert_eq!(state.sort_key(), ProcessSortKey::Pid);
    assert_eq!(state.sort_direction(), SortDirection::Descending);
}

#[test]
fn process_search_matches_pid_user_name_and_full_command_case_insensitively() {
    let processes = vec![
        process(42, "Alice", 1024, 1.0, "Worker --TOKEN abc", "2026-01-01"),
        process(73, "root", 2048, 2.0, "sshd: root", "2026-01-02"),
    ];

    assert_eq!(filtered_processes(&processes, "42")[0].pid, 42);
    assert_eq!(filtered_processes(&processes, "alice")[0].pid, 42);
    assert_eq!(filtered_processes(&processes, "token")[0].pid, 42);
    assert_eq!(filtered_processes(&processes, "SSHD")[0].pid, 73);
    assert!(filtered_processes(&processes, "missing").is_empty());
}

#[test]
fn process_table_uses_readable_font_and_row_metrics() {
    assert!(PROCESS_TEXT_SIZE >= 12.0);
    assert!(PROCESS_META_SIZE >= 11.0);
    assert!(PROCESS_ROW_HEIGHT >= 26.0);
    assert!(PROCESS_TABLE_MIN_WIDTH >= 1_160.0);
}

#[test]
fn zombie_dialog_does_not_replace_the_selected_process_detail() {
    let ctx = egui::Context::default();
    let mut state = state();
    state.select_process(10, DETAIL_START);
    assert!(state.apply_detail(1, Ok(detail(10, "CURRENT_SECRET"))));

    let mut zombie = process(73, "root", 2048, 0.0, "defunct", "2026-01-02");
    zombie.state = "Z".into();
    let processes = vec![process(10, "dev", 1024, 1.0, "worker", DETAIL_START)];
    let zombies = vec![zombie];
    let stats = ProcessStats {
        zombie: 1,
        total: 2,
        ..ProcessStats::default()
    };
    state.set_zombie_dialog_open(true);

    let input = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(1_400.0, 900.0),
        )),
        ..Default::default()
    };
    let output = ctx.run(input, |ctx| {
        render(
            ctx,
            &mut state,
            Some(&processes),
            Some(&zombies),
            Some(&stats),
            None,
        );
    });

    assert!(output.shapes.len() > 0);
    assert!(state.zombie_dialog_open());
    assert_eq!(state.selected_pid(), Some(10));
    assert!(state.detail().is_some());
}

#[test]
fn copied_process_detail_contains_forensic_path_and_start_time() {
    let detail = detail(42, "COPY_SECRET");
    let copied = format_process_detail(&detail);

    assert!(copied.contains("/proc/42/exe: /opt/worker"));
    assert!(copied.contains("启动时间: Mon Jul 27 10:00:00 2026"));
    assert!(copied.contains("工作目录: /srv/private"));
    assert!(copied.contains("完整命令行: worker --token=COPY_SECRET"));
}

#[test]
fn selecting_a_row_uses_monotonic_request_ids_and_clears_old_state() {
    let mut state = state();
    let first = state.select_process(41, "start-41");
    let second = state.select_process(42, "start-42");
    assert_eq!(
        first,
        ProcessManagerAction::Select {
            pid: 41,
            request_id: 1
        }
    );
    assert_eq!(
        second,
        ProcessManagerAction::Select {
            pid: 42,
            request_id: 2
        }
    );
    assert_eq!(state.selected_pid(), Some(42));
    assert_eq!(state.pending_request_id(), Some(2));
    assert!(!state.is_current_request(1));
    assert!(state.is_current_request(2));
}

#[test]
fn stale_detail_error_is_ignored() {
    let mut state = state();
    state.select_process(10, "start-10");
    state.select_process(11, "start-11");
    assert!(!state.apply_detail(1, Err("旧请求".to_string())));
    assert_eq!(state.pending_request_id(), Some(2));
    assert_eq!(state.detail_error(), None);
}

#[test]
fn stale_detail_payload_is_ignored_without_leaking_into_state() {
    let mut state = state();
    state.select_process(10, DETAIL_START);
    state.select_process(11, DETAIL_START);
    assert!(!state.apply_detail(1, Ok(detail(10, "STALE_SECRET"))));
    assert_eq!(state.pending_request_id(), Some(2));
    assert!(state.detail().is_none());
    assert!(!format!("{state:?}").contains("STALE_SECRET"));
}

#[test]
fn closing_detail_clears_selection_pending_request_and_sensitive_payload() {
    let mut state = state();
    state.select_process(10, DETAIL_START);
    assert!(state.apply_detail(1, Ok(detail(10, "CURRENT_SECRET"))));
    assert!(state.detail().is_some());
    assert!(!format!("{state:?}").contains("CURRENT_SECRET"));
    state.clear_detail();
    assert_eq!(state.selected_pid(), None);
    assert_eq!(state.pending_request_id(), None);
    assert_eq!(state.detail_error(), None);
    assert!(state.detail().is_none());
}

#[test]
fn detail_with_reused_pid_start_time_finishes_pending_with_error() {
    let mut state = state();
    state.select_process(10, "old-start");

    assert!(state.apply_detail(1, Ok(detail(10, "NEW_PROCESS_SECRET"))));
    assert_eq!(state.pending_request_id(), None);
    assert!(state.detail().is_none());
    assert_eq!(state.detail_error(), Some("进程已退出或 PID 已被复用"));
}

#[test]
fn process_identity_normalizes_internal_start_time_whitespace() {
    let mut state = state();
    state.select_process(10, "Mon  Jul 27   10:00:00 2026");

    assert!(state.apply_detail(1, Ok(detail(10, "CURRENT_SECRET"))));
    assert!(state.detail().is_some());
    assert_eq!(state.pending_request_id(), None);
    assert_eq!(state.detail_error(), None);
}

#[test]
fn reconciliation_clears_detail_when_pid_is_reused_but_not_when_row_is_capped() {
    let mut state = state();
    state.select_process(10, DETAIL_START);
    assert!(state.apply_detail(1, Ok(detail(10, "CURRENT_SECRET"))));

    assert!(!state.reconcile_processes(&[]));
    assert!(state.detail().is_some());

    let reused = vec![process(10, "dev", 1024, 1.0, "worker", "new-start")];
    assert!(state.reconcile_processes(&reused));
    assert_eq!(state.selected_pid(), None);
    assert!(state.detail().is_none());
    assert_eq!(state.detail_error(), Some("PID 已被复用，旧进程详情已清理"));
    assert!(!state.reconcile_processes(&reused));
}

#[test]
fn detail_panel_keeps_a_bounded_height_when_the_window_grows() {
    assert_eq!(detail_panel_max_height(600.0), 330.0);
    assert_eq!(detail_panel_max_height(1_400.0), 770.0);
    assert_eq!(detail_panel_max_height(100.0), 120.0);
    assert_eq!(detail_panel_max_height(f32::NAN), 120.0);

    let mut state = state();
    assert!(!has_detail_area(&state));
    state.select_process(10, DETAIL_START);
    assert!(has_detail_area(&state));
}

#[test]
fn maximizing_keeps_detail_panel_height_and_gives_new_space_to_process_list() {
    let ctx = egui::Context::default();
    let mut state = state();
    let processes = vec![process(10, "dev", 1024, 1.0, "worker", DETAIL_START)];
    let stats = ProcessStats::default();
    state.select_process(10, DETAIL_START);
    assert!(state.apply_detail(1, Ok(detail(10, "CURRENT_SECRET"))));

    let render_at = |ctx: &egui::Context, state: &mut ProcessManagerState, size: egui::Vec2| {
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
            ..Default::default()
        };
        let _ = ctx.run(input, |ctx| {
            render(ctx, state, Some(&processes), Some(&[]), Some(&stats), None);
        });
        egui::containers::panel::PanelState::load(
            ctx,
            egui::Id::new("process_manager_detail_panel"),
        )
        .expect("detail panel state should exist")
        .rect
        .height()
    };

    let medium_height = render_at(&ctx, &mut state, egui::vec2(1_000.0, 700.0));
    let maximized_height = render_at(&ctx, &mut state, egui::vec2(2_000.0, 1_400.0));

    assert!((medium_height - 300.0).abs() <= 1.0);
    assert!((maximized_height - medium_height).abs() <= 1.0);
}

#[test]
fn render_smoke_covers_loading_empty_and_stale_states() {
    let ctx = egui::Context::default();
    let mut state = state();
    let empty = Vec::<ProcessInfo>::new();
    let stats = ProcessStats::default();

    let loading = ctx.run(egui::RawInput::default(), |ctx| {
        render(ctx, &mut state, None, None, None, None);
    });
    assert!(loading.shapes.len() > 0);

    let empty_output = ctx.run(egui::RawInput::default(), |ctx| {
        render(
            ctx,
            &mut state,
            Some(&empty),
            Some(&empty),
            Some(&stats),
            None,
        );
    });
    assert!(empty_output.shapes.len() > 0);

    let stale = ctx.run(egui::RawInput::default(), |ctx| {
        render(
            ctx,
            &mut state,
            Some(&empty),
            Some(&empty),
            Some(&stats),
            Some("连接已断开"),
        );
    });
    assert!(stale.shapes.len() > 0);
}
