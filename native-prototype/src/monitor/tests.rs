use super::{
    bounded_local_environment, collect_local_process_detail, format_bytes, process_refresh_kind,
    MonitorKey, ProcessAncestor, ProcessDetail, ProcessEnvironment, ProcessIdentity, ProcessStats,
    LOCAL_ANCESTORS, LOCAL_ANCESTOR_COMMAND_BYTES, LOCAL_DETAIL_FIELD_BYTES,
    LOCAL_ENVIRONMENT_BYTES, LOCAL_ENVIRONMENT_ENTRIES,
};

const KIB: u64 = 1_024;
const MIB: u64 = 1_048_576;
const GIB: u64 = 1_073_741_824;
const TIB: u64 = 1_099_511_627_776;

#[cfg(target_os = "linux")]
#[test]
fn linux_pss_parser_reads_kib_and_rejects_missing_or_overflowed_values() {
    assert_eq!(
        super::parse_linux_pss_bytes("Rss: 4096 kB\nPss: 1536 kB\n"),
        Some(1536 * KIB)
    );
    assert_eq!(super::parse_linux_pss_bytes("Rss: 4096 kB\n"), None);
    assert_eq!(
        super::parse_linux_pss_bytes("Pss: 18446744073709551615 kB\n"),
        None
    );
}

#[cfg(target_os = "linux")]
#[test]
fn linux_application_memory_uses_rss_anon_instead_of_total_rss() {
    let status = "Name:\tworker\nVmRSS:\t350000 kB\nRssAnon:\t98304 kB\nRssFile:\t251696 kB\n";

    assert_eq!(super::parse_linux_rss_anon_bytes(status), Some(96 * MIB));
    assert_eq!(
        super::parse_linux_rss_anon_bytes("VmRSS:\t350000 kB\n"),
        None
    );
}

#[test]
fn disk_capacity_formats_existing_binary_units() {
    assert_eq!(format_bytes(0), "0K");
    assert_eq!(format_bytes(KIB), "1K");
    assert_eq!(format_bytes(MIB), "1M");
    assert_eq!(format_bytes(GIB), "1.0G");
}

#[test]
fn disk_capacity_switches_to_tib_at_boundary() {
    let approximately_1033_7_gib = (1033.7 * GIB as f64) as u64;

    assert_eq!(format_bytes(TIB - 1), "1024.0G");
    assert_eq!(format_bytes(TIB), "1.0T");
    assert_eq!(format_bytes(TIB + TIB / 10), "1.1T");
    assert_eq!(format_bytes(approximately_1033_7_gib), "1.0T");
}

#[test]
fn monitor_process_refresh_avoids_tasks() {
    let kind = process_refresh_kind();

    assert!(kind.cpu());
    assert!(kind.memory());
    assert!(!kind.tasks());
}

#[test]
fn remote_monitor_keys_compare_by_user_host_and_port() {
    let key = MonitorKey::remote("alice", "server.example", 22);

    assert_eq!(key, MonitorKey::remote("alice", "server.example", 22));
    assert_ne!(key, MonitorKey::remote("bob", "server.example", 22));
    assert_ne!(key, MonitorKey::remote("alice", "other.example", 22));
    assert_ne!(key, MonitorKey::remote("alice", "server.example", 2200));
}

#[test]
fn monitor_key_status_text_is_exact() {
    assert_eq!(MonitorKey::Local.status_text(), "本机");
    assert_eq!(
        MonitorKey::remote("alice", "server.example", 2200).status_text(),
        "alice@server.example:2200"
    );
}

#[test]
fn monitor_key_status_text_brackets_unbracketed_ipv6_hosts() {
    assert_eq!(
        MonitorKey::remote("alice", "2001:db8::1", 2200).status_text(),
        "alice@[2001:db8::1]:2200"
    );
    assert_eq!(
        MonitorKey::remote("alice", "[2001:db8::1]", 2200).status_text(),
        "alice@[2001:db8::1]:2200"
    );
}

#[test]
fn process_stats_classify_linux_states() {
    let mut stats = ProcessStats::default();
    for state in ["R", "S", "D", "I", "Z", "T", "t", "X"] {
        stats.record_state(state);
    }

    assert_eq!(stats.total, 8);
    assert_eq!(stats.running, 1);
    assert_eq!(stats.sleeping, 3);
    assert_eq!(stats.zombie, 1);
    assert_eq!(stats.stopped, 2);
}

#[test]
fn local_process_detail_reports_missing_process_without_sensitive_data() {
    let error = collect_local_process_detail(u32::MAX).unwrap_err();

    assert!(error.contains("本机进程"));
    assert!(!error.contains("/proc/"));
}

#[test]
fn local_process_detail_collects_current_process_with_bounded_fields() {
    let pid = std::process::id();
    let detail = collect_local_process_detail(pid).expect("current process should be readable");

    assert_eq!(detail.identity.pid, pid);
    assert!(detail.identity.start_ticks > 0);
    assert_ne!(detail.start_time, detail.identity.start_ticks.to_string());
    assert_eq!(detail.start_time.len(), 19);
    assert_eq!(detail.start_time.chars().nth(4), Some('-'));
    assert!(!detail.name.is_empty());
    assert!(!detail.state.is_empty());
    assert!(detail.command.len() <= LOCAL_DETAIL_FIELD_BYTES);
    assert!(detail.executable.len() <= LOCAL_DETAIL_FIELD_BYTES);
    assert!(detail.working_dir.len() <= LOCAL_DETAIL_FIELD_BYTES);
    assert!(detail.environ.len() <= LOCAL_ENVIRONMENT_ENTRIES);
    assert!(
        detail
            .environ
            .iter()
            .map(|entry| entry.key.len() + entry.value.len())
            .sum::<usize>()
            <= LOCAL_ENVIRONMENT_BYTES
    );
    assert!(!detail.ancestors.is_empty());
    assert_eq!(detail.ancestors[0].pid, pid);
    assert!(detail.ancestors.len() <= LOCAL_ANCESTORS);
    assert!(detail
        .ancestors
        .iter()
        .all(|ancestor| ancestor.command.len() <= LOCAL_ANCESTOR_COMMAND_BYTES));
}

#[test]
fn local_environment_collection_caps_entries_total_bytes_and_control_text() {
    let environment = (0..100)
        .map(|index| {
            std::ffi::OsString::from(format!("KEY_{index}=value\n{}", "x".repeat(10 * 1024)))
        })
        .collect::<Vec<_>>();

    let bounded = bounded_local_environment(&environment);

    assert!(bounded.len() <= LOCAL_ENVIRONMENT_ENTRIES);
    assert!(
        bounded
            .iter()
            .map(|entry| entry.key.len() + entry.value.len())
            .sum::<usize>()
            <= LOCAL_ENVIRONMENT_BYTES
    );
    assert!(bounded.iter().all(|entry| !entry.value.contains('\n')));
}

#[test]
fn process_detail_debug_redacts_sensitive_fields() {
    let detail = ProcessDetail {
        identity: ProcessIdentity {
            pid: 42,
            start_ticks: 99,
        },
        user: "alice".to_string(),
        state: "S".to_string(),
        mem_mb: "1M".to_string(),
        mem_bytes: 1_048_576,
        platform_memory: None,
        cpu: 1.0,
        name: "worker".to_string(),
        command: "RAW_COMMAND_SENTINEL".to_string(),
        executable: "RAW_EXE_SENTINEL".to_string(),
        working_dir: "RAW_CWD_SENTINEL".to_string(),
        start_time: "Mon Jul 27 10:00:00 2026".to_string(),
        environ: vec![ProcessEnvironment {
            key: "TOKEN".to_string(),
            value: "RAW_ENV_SENTINEL".to_string(),
        }],
        ancestors: Vec::new(),
    };

    let debug = format!("{detail:?}");
    assert!(debug.contains("pid: 42"));
    assert!(!debug.contains("RAW_COMMAND_SENTINEL"));
    assert!(!debug.contains("RAW_EXE_SENTINEL"));
    assert!(!debug.contains("RAW_CWD_SENTINEL"));
    assert!(!debug.contains("RAW_ENV_SENTINEL"));
}

#[test]
fn process_detail_child_debug_types_redact_sensitive_text() {
    let environment = ProcessEnvironment {
        key: "TOKEN_SENTINEL".into(),
        value: "VALUE_SENTINEL".into(),
    };
    let ancestor = ProcessAncestor {
        pid: 7,
        name: "worker".into(),
        command: "COMMAND_SENTINEL".into(),
    };

    let environment_debug = format!("{environment:?}");
    let ancestor_debug = format!("{ancestor:?}");
    assert!(!environment_debug.contains("TOKEN_SENTINEL"));
    assert!(!environment_debug.contains("VALUE_SENTINEL"));
    assert!(!ancestor_debug.contains("COMMAND_SENTINEL"));
    assert!(ancestor_debug.contains("pid: 7"));
}
