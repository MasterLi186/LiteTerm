use std::io::Cursor;

use super::*;

const SAMPLE: &str = "\
===IP===
eth0@if123 10.0.0.2/24
eth1 192.168.1.5/24
===SS===
ESTAB 0 0 10.0.0.2:22 10.0.0.10:55432 users:((\"sshd\",pid=42,fd=4))
LISTEN 0 128 192.168.1.5:8080 0.0.0.0:* users:((\"server\",pid=7,fd=3))
TIME-WAIT 0 0 10.0.0.2:443 10.0.0.20:60000
";

fn state() -> NetworkDetailState {
    NetworkDetailState::new(
        MonitorKey::remote("dev", "example.com", 22),
        Some("eth0".to_string()),
    )
}

#[test]
fn parses_interfaces_connections_and_permission_limited_processes() {
    let snapshot = parse_network_detail(SAMPLE).unwrap();
    assert_eq!(snapshot.primary_address("eth0"), Some("10.0.0.2"));
    assert!(!snapshot.interface_addresses.contains_key("eth0@if123"));
    assert_eq!(snapshot.connections.len(), 3);
    assert_eq!(snapshot.connections[0].process, "sshd");
    assert_eq!(snapshot.connections[0].pid, Some(42));
    assert_eq!(snapshot.connections[2].process, "");
    assert_eq!(snapshot.connections[2].pid, None);
}

#[test]
fn filters_connections_by_selected_interface_address() {
    let snapshot = parse_network_detail(SAMPLE).unwrap();
    let eth0 = filtered_connections(&snapshot, Some("eth0"));
    assert_eq!(eth0.len(), 2);
    assert!(eth0
        .iter()
        .all(|connection| connection.local_address.starts_with("10.0.0.2:")));
    assert!(filtered_connections(&snapshot, Some("missing")).is_empty());
    assert_eq!(filtered_connections(&snapshot, None).len(), 3);
}

#[test]
fn selected_rates_follow_interface_switch_without_refreshing_connections() {
    let mut state = state();
    state.update_rates(&[
        NetIfaceInfo {
            name: "eth0".to_string(),
            rx_rate: 1_024,
            tx_rate: 2_048,
        },
        NetIfaceInfo {
            name: "eth1@if999".to_string(),
            rx_rate: 3_072,
            tx_rate: 4_096,
        },
    ]);

    assert_eq!(
        state.selected_rates(),
        Some(NetworkInterfaceRate {
            rx_rate: 1_024,
            tx_rate: 2_048,
        })
    );
    state.select_interface(Some("eth1@if999".to_string()));
    assert_eq!(state.selected_interface(), Some("eth1"));
    assert_eq!(
        state.selected_rates(),
        Some(NetworkInterfaceRate {
            rx_rate: 3_072,
            tx_rate: 4_096,
        })
    );
    state.select_interface(Some("missing".to_string()));
    assert_eq!(state.selected_rates(), None);
    assert!(state.snapshot().is_none());
    assert_eq!(state.pending_request_id(), None);
}

#[test]
fn sorts_all_five_columns_in_both_directions() {
    let snapshot = parse_network_detail(SAMPLE).unwrap();
    for key in [
        NetworkSortKey::State,
        NetworkSortKey::LocalAddress,
        NetworkSortKey::RemoteAddress,
        NetworkSortKey::Pid,
        NetworkSortKey::Process,
    ] {
        let ascending = sorted_connections(
            snapshot.connections.iter().collect(),
            key,
            SortDirection::Ascending,
        );
        let descending = sorted_connections(
            snapshot.connections.iter().collect(),
            key,
            SortDirection::Descending,
        );
        assert_eq!(
            ascending
                .iter()
                .map(|connection| *connection)
                .collect::<Vec<_>>(),
            descending
                .iter()
                .rev()
                .map(|connection| *connection)
                .collect::<Vec<_>>()
        );
    }
}

#[test]
fn rejects_missing_sentinels_and_oversized_output() {
    assert!(parse_network_detail("===SS===\n").is_err());
    assert!(parse_network_detail("===IP===\neth0 10.0.0.2/24\n").is_err());

    let oversized = vec![b'x'; MAX_NETWORK_DETAIL_BYTES + 1];
    let error = read_network_detail_bounded(Cursor::new(oversized)).unwrap_err();
    assert!(error.contains("512KiB"));
}

#[test]
fn command_is_fixed_and_never_contains_an_interface_placeholder() {
    assert!(NETWORK_DETAIL_COMMAND.contains("===IP==="));
    assert!(NETWORK_DETAIL_COMMAND.contains("===SS==="));
    assert!(NETWORK_DETAIL_COMMAND.contains("ss -Htnp4"));
    assert!(!NETWORK_DETAIL_COMMAND.contains("{iface}"));
    assert!(!NETWORK_DETAIL_COMMAND.contains("$IFACE"));
}

#[test]
fn bounded_reader_accepts_exact_limit() {
    let padding = " ".repeat(MAX_NETWORK_DETAIL_BYTES - "===IP===\n===SS===\n".len());
    let output = format!("===IP===\n===SS===\n{padding}");
    let snapshot = read_network_detail_bounded(Cursor::new(output)).unwrap();
    assert!(snapshot.connections.is_empty());
}

#[test]
fn local_bounded_reader_uses_local_error_context() {
    let snapshot = read_local_network_detail_bounded(Cursor::new(SAMPLE)).unwrap();
    assert_eq!(snapshot.connections.len(), 3);

    let oversized = vec![b'x'; MAX_NETWORK_DETAIL_BYTES + 1];
    let error = read_local_network_detail_bounded(Cursor::new(oversized)).unwrap_err();
    assert_eq!(error, "本地网络详情超过 512KiB 限制");
}

#[test]
fn refresh_is_deduplicated_and_stale_results_are_rejected() {
    let mut state = state();
    assert_eq!(
        state.request_refresh(),
        Some(NetworkDetailAction::Refresh { request_id: 1 })
    );
    assert_eq!(state.request_refresh(), None);
    assert!(!state.apply_snapshot(9, Ok(parse_network_detail(SAMPLE).unwrap())));
    assert_eq!(state.pending_request_id(), Some(1));
    assert!(state.apply_snapshot(1, Err("暂时失败".to_string())));
    assert_eq!(state.pending_request_id(), None);
    assert_eq!(state.error(), Some("暂时失败"));
    assert_eq!(
        state.request_refresh(),
        Some(NetworkDetailAction::Refresh { request_id: 2 })
    );
}

#[test]
fn monitor_restart_cancels_pending_refresh_and_allows_retry() {
    let mut state = state();
    assert_eq!(
        state.request_refresh(),
        Some(NetworkDetailAction::Refresh { request_id: 1 })
    );
    assert!(state.cancel_pending_refresh());
    assert!(!state.cancel_pending_refresh());
    assert_eq!(
        state.request_refresh(),
        Some(NetworkDetailAction::Refresh { request_id: 2 })
    );
}

#[test]
fn debug_does_not_expose_payload_or_error() {
    let mut state = state();
    state.request_refresh();
    let mut snapshot = parse_network_detail(SAMPLE).unwrap();
    snapshot.connections[0].process = "RAW_PROCESS_SENTINEL".to_string();
    state.apply_snapshot(1, Ok(snapshot));
    state.request_refresh();
    state.apply_snapshot(2, Err("RAW_ERROR_SENTINEL".to_string()));
    let debug = format!("{state:?}");
    assert!(!debug.contains("RAW_PROCESS_SENTINEL"));
    assert!(!debug.contains("RAW_ERROR_SENTINEL"));
}

#[test]
fn render_smoke_covers_loading_error_and_empty_states() {
    let ctx = egui::Context::default();
    let mut loading = state();
    let output = ctx.run(egui::RawInput::default(), |ctx| {
        let actions = render(ctx, &mut loading);
        assert_eq!(
            actions,
            vec![NetworkDetailAction::Refresh { request_id: 1 }]
        );
    });
    assert!(!output.shapes.is_empty());

    loading.apply_snapshot(1, Err("连接失败".to_string()));
    let output = ctx.run(egui::RawInput::default(), |ctx| {
        assert!(render(ctx, &mut loading).is_empty());
    });
    assert!(!output.shapes.is_empty());

    let mut empty = state();
    empty.request_refresh();
    empty.apply_snapshot(1, Ok(NetworkDetailSnapshot::default()));
    let output = ctx.run(egui::RawInput::default(), |ctx| {
        assert!(render(ctx, &mut empty).is_empty());
    });
    assert!(!output.shapes.is_empty());
}
