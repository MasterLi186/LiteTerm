    #[test]
    fn worker_spawn_failure_returns_an_error_without_a_handle() {
        let result = start_worker_with_sink_for_test_and_spawner(
            MonitorKey::remote("alice", "alpha.example", 22),
            1,
            || -> Result<FakeSource, String> {
                Ok(FakeSource {
                    results: VecDeque::new(),
                    collects: Arc::new(AtomicUsize::new(0)),
                    drops: Arc::new(AtomicUsize::new(0)),
                })
            },
            |_| Ok(()),
            WorkerTiming::new(Duration::ZERO, Duration::ZERO),
            mpsc::channel().0,
            |_| Err(io::Error::other("spawn failed")),
        );

        assert!(result.is_err());
    }

    #[test]
    fn queued_shutdown_skips_source_factory() {
        let factory_calls = Arc::new(AtomicUsize::new(0));
        let calls = Arc::clone(&factory_calls);
        let mut factory = move || -> Result<FakeSource, String> {
            calls.fetch_add(1, Ordering::SeqCst);
            Err("不应连接".to_string())
        };
        let (commands_tx, commands_rx) = mpsc::channel();
        commands_tx.send(RemoteMonitorCommand::Shutdown).unwrap();

        super::run_worker(
            MonitorKey::remote("alice", "alpha.example", 22),
            1,
            &mut factory,
            &|_| Ok(()),
            timing(),
            &commands_rx,
        );

        assert_eq!(factory_calls.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn parses_complete_snapshot_into_monitor_shape() {
        let mut parser = RemoteSnapshotParser::default();
        let data = parser.parse(SAMPLE, Duration::from_secs(2)).unwrap();

        assert_eq!(data.cpu_percent, 0.0);
        assert_eq!(data.cpu_name, "Example CPU");
        assert_eq!(data.memory_used, 1_048_576_000);
        assert_eq!(data.memory_total, 2_147_483_648);
        assert_eq!(data.memory_text, "1000M / 2.0G");
        assert_eq!(data.memory_percent, 48.828125);
        assert_eq!(data.swap_used, 262_144_000);
        assert_eq!(data.swap_total, 524_288_000);
        assert_eq!(data.swap_text, "250M / 500M");
        assert_eq!(data.swap_percent, 50.0);
        assert_eq!(data.uptime_text, "1天1小时1分钟");
        assert_eq!(data.load_text, "0.10, 0.20, 0.30");
        assert_eq!(data.disk_items.len(), 1);
        assert_eq!(data.disk_items[0].mount, "/");
        assert_eq!(data.disk_items[0].avail, "512M");
        assert_eq!(data.disk_items[0].size, "1.0G");
        assert_eq!(data.disk_items[0].percent, 50);
        assert_eq!(data.processes[0].mem_bytes, 4_194_304);
        assert_eq!(data.processes[0].mem_mb, "4M");
        assert_eq!(data.processes[0].resident_mem_bytes, 10_485_760);
        assert_eq!(data.processes[0].resident_mem_mb, "10M");
        assert_eq!(data.processes[0].cpu, 12.5);
        assert_eq!(data.processes[0].pid, 42);
        assert_eq!(data.processes[0].user, "alice");
        assert_eq!(data.processes[0].state, "S");
        assert_eq!(data.processes[0].name, "worker");
        assert_eq!(data.processes[0].command, "/usr/bin/test process --flag");
        assert_eq!(data.processes[0].start_time, "Mon Jul 27 10:00:00 2026");
        assert_eq!(data.process_stats.total, 28);
        assert_eq!(data.process_stats.running, 1);
        assert_eq!(data.process_stats.sleeping, 12);
        assert_eq!(data.process_stats.zombie, 4);
        assert_eq!(data.process_stats.stopped, 11);
        assert_eq!(data.net_interfaces.len(), 1);
        assert_eq!(data.net_interfaces[0].name, "eth0");
        assert_eq!(data.preferred_net_interface.as_deref(), Some("eth0"));
        assert_eq!(data.net_interfaces[0].rx_rate, 0);
        assert_eq!(data.net_interfaces[0].tx_rate, 0);
    }

    #[test]
    fn computes_second_sample_cpu_and_network_rates() {
        let mut parser = RemoteSnapshotParser::default();
        parser.parse(SAMPLE, Duration::from_secs(2)).unwrap();
        let second = SAMPLE
            .replace(
                "cpu  100 0 50 800 50 0 0 0 0 0",
                "cpu  120 0 60 870 50 0 0 0 0 0",
            )
            .replace("eth0: 1000", "eth0: 5000")
            .replace("0 0 0 0 2000", "0 0 0 0 10000");

        let data = parser.parse(&second, Duration::from_secs(2)).unwrap();

        assert_eq!(data.cpu_percent, 30.0);
        assert_eq!(data.net_interfaces[0].rx_rate, 2000);
        assert_eq!(data.net_interfaces[0].tx_rate, 4000);
    }

    #[test]
    fn first_network_sample_has_zero_rates() {
        let mut parser = RemoteSnapshotParser::default();
        let data = parser.parse(SAMPLE, Duration::from_millis(1)).unwrap();

        assert!(data
            .net_interfaces
            .iter()
            .all(|item| item.rx_rate == 0 && item.tx_rate == 0));
    }

    #[test]
    fn rejects_missing_required_stat_or_mem_sections() {
        let mut parser = RemoteSnapshotParser::default();
        assert!(parser
            .parse("MEM\nMemTotal: 1 kB\nEND\n", Duration::from_secs(1))
            .is_err());
        assert!(parser
            .parse("STAT\ncpu 1 0 0 1\nEND\n", Duration::from_secs(1))
            .is_err());
    }

    #[test]
    fn missing_end_rejects_snapshot_without_advancing_parser_history() {
        let mut parser = RemoteSnapshotParser::default();
        parser.parse(SAMPLE, Duration::from_secs(2)).unwrap();
        let advanced = SAMPLE
            .replace(
                "cpu  100 0 50 800 50 0 0 0 0 0",
                "cpu  120 0 60 870 50 0 0 0 0 0",
            )
            .replace("eth0: 1000", "eth0: 5000")
            .replace("0 0 0 0 2000", "0 0 0 0 10000");
        let incomplete = advanced.trim_end_matches("END\n");

        assert!(parser.parse(incomplete, Duration::from_secs(2)).is_err());
        let data = parser.parse(&advanced, Duration::from_secs(2)).unwrap();

        assert!(data.cpu_percent > 0.0);
        assert!(data
            .net_interfaces
            .iter()
            .any(|iface| iface.rx_rate > 0 || iface.tx_rate > 0));
    }

    #[test]
    fn rejects_required_sections_without_valid_keys() {
        let mut parser = RemoteSnapshotParser::default();
        assert!(parser
            .parse(
                "STAT\ncpu 1 nope 2 3\nMEM\nMemTotal: 1 kB\nEND\n",
                Duration::from_secs(1),
            )
            .is_err());
        assert!(parser
            .parse(
                "STAT\ncpu 1 0 2 3\nMEM\nMemTotal: nope kB\nEND\n",
                Duration::from_secs(1),
            )
            .is_err());
    }

    #[test]
    fn end_marker_discards_later_replacement_sections() {
        let output = format!("{SAMPLE}STAT\ncpu 1 nope 2 3\nMEM\nMemTotal: nope kB\n");

        let sections = super::split_sections(&output).unwrap();
        assert_eq!(
            sections["STAT"],
            "cpu  100 0 50 800 50 0 0 0 0 0\ncpu0 50 0 25 400 25 0 0 0 0 0\n"
        );
        assert_eq!(sections["MEM"], "MemTotal:       2097152 kB\nMemAvailable:   1073152 kB\nSwapTotal:       512000 kB\nSwapFree:        256000 kB\n");
    }

    #[test]
    fn end_marker_cannot_be_followed_by_required_sections() {
        let mut parser = RemoteSnapshotParser::default();
        let output = "END\nSTAT\ncpu 1 0 2 3\nMEM\nMemTotal: 1 kB\n";

        assert!(parser.parse(output, Duration::from_secs(1)).is_err());
    }

    #[test]
    fn malformed_optional_sections_are_safe() {
        let mut parser = RemoteSnapshotParser::default();
        let output = "STAT\ncpu 1 0 2 3\nMEM\nMemTotal: 1 kB\nDISK\nbad\nNET\neth0: nope\nLOAD\nnot load\nUPTIME\nnope\nPS\nbad\nCPUINFO\ninvalid\nEND\n";

        let data = parser.parse(output, Duration::ZERO).unwrap();
        assert_eq!(data.cpu_name, "Unknown CPU");
        assert!(data.disk_items.is_empty());
        assert!(data.processes.is_empty());
        assert!(data.net_interfaces.is_empty());
    }

    #[test]
    fn mem_free_is_used_when_mem_available_is_absent() {
        let mut parser = RemoteSnapshotParser::default();
        let output = SAMPLE.replace("MemAvailable:   1073152 kB", "MemFree:        1048576 kB");

        let data = parser.parse(&output, Duration::from_secs(1)).unwrap();
        assert_eq!(data.memory_used, 1_073_741_824);
        assert_eq!(data.memory_text, "1.0G / 2.0G");
    }

    #[test]
    fn network_interfaces_are_sorted_and_loopback_is_ignored() {
        let mut parser = RemoteSnapshotParser::default();
        let output = SAMPLE.replace(
            " lo: 999 0 0 0 0 0 0 0 999 0 0 0 0 0 0 0",
            " zeta: 1 0 0 0 0 0 0 0 1 0 0 0 0 0 0 0\n alpha: 1 0 0 0 0 0 0 0 1 0 0 0 0 0 0 0\n lo: 999 0 0 0 0 0 0 0 999 0 0 0 0 0 0 0",
        );

        let data = parser.parse(&output, Duration::from_secs(1)).unwrap();
        let names: Vec<_> = data
            .net_interfaces
            .iter()
            .map(|item| item.name.as_str())
            .collect();
        assert_eq!(names, ["alpha", "eth0", "zeta"]);
    }

    #[test]
    fn parses_space_padded_ps_rows() {
        let mut parser = RemoteSnapshotParser::default();
        let output = SAMPLE.replace(
            "42 alice S 10240 12.5 worker Mon Jul 27 10:00:00 2026 /usr/bin/test process --flag",
            "  7   bob   R   1024   3.5   helper   Tue Jul 28 11:22:33 2026 worker --flag",
        );

        let data = parser.parse(&output, Duration::from_secs(1)).unwrap();
        assert_eq!(data.processes.len(), 1);
        assert_eq!(data.processes[0].pid, 7);
        assert_eq!(data.processes[0].name, "helper");
        assert_eq!(data.processes[0].command, "worker --flag");
    }

    #[test]
    fn network_counter_reset_never_overflows() {
        let mut parser = RemoteSnapshotParser::default();
        parser.parse(SAMPLE, Duration::from_secs(2)).unwrap();
        let reset = SAMPLE
            .replace("eth0: 1000", "eth0: 1")
            .replace("0 0 0 0 2000", "0 0 0 0 1");

        let data = parser.parse(&reset, Duration::from_secs(2)).unwrap();
        assert_eq!(data.net_interfaces[0].rx_rate, 0);
        assert_eq!(data.net_interfaces[0].tx_rate, 0);
    }

    #[test]
    fn cpu_counter_reset_never_overflows() {
        let mut parser = RemoteSnapshotParser::default();
        parser.parse(SAMPLE, Duration::from_secs(2)).unwrap();
        let reset = SAMPLE.replace("cpu  100 0 50 800 50 0 0 0 0 0", "cpu  1 0 1 1 0");

        let data = parser.parse(&reset, Duration::from_secs(2)).unwrap();
        assert_eq!(data.cpu_percent, 0.0);
    }

    #[test]
    fn uptime_rejects_non_finite_and_negative_values() {
        for value in ["NaN", "+inf", "-inf", "-1.0"] {
            assert_eq!(super::parse_uptime(value), "0分钟");
        }
    }

    #[test]
    fn load_rejects_non_finite_and_negative_values() {
        for load in ["NaN 0.2 0.3", "+inf 0.2 0.3", "-1.0 0.2 0.3"] {
            assert_eq!(super::parse_load(load), "");
        }
    }

    #[test]
    fn process_parser_rejects_invalid_cpu_and_overflowing_rss() {
        let output = format!(
            "1 user S 1 NaN nan Mon Jul 27 10:00:00 2026 nan\n\
             2 user S 1 inf infinite Mon Jul 27 10:00:00 2026 infinite\n\
             3 user S 1 -1 negative Mon Jul 27 10:00:00 2026 negative\n\
             4 user S {} 1 overflow Mon Jul 27 10:00:00 2026 overflow\n\
             5 user S 1024 2.5 valid Mon Jul 27 10:00:00 2026 valid --flag\n",
            u64::MAX
        );

        let processes = super::parse_processes(&output);
        assert_eq!(processes.len(), 1);
        assert_eq!(processes[0].name, "valid");
        assert_eq!(processes[0].mem_bytes, 0);
        assert_eq!(processes[0].mem_mb, "—");
        assert_eq!(processes[0].resident_mem_bytes, 1_048_576);
    }

    #[test]
    fn application_memory_is_joined_by_pid_and_missing_values_stay_unavailable() {
        let output = "5 user S 1024 2.5 first Mon Jul 27 10:00:00 2026 first\n\
                      6 user S 2048 1.5 second Mon Jul 27 10:00:00 2026 second\n";
        let mut processes = super::parse_processes(output);

        super::apply_process_application_memory(
            &mut processes,
            "5 256\ninvalid\n6 overflow\n999 100\n",
        );

        assert_eq!(processes[0].mem_bytes, 256 * 1024);
        assert_eq!(processes[0].mem_mb, "256K");
        assert_eq!(processes[1].mem_bytes, 0);
        assert_eq!(processes[1].mem_mb, "—");
        assert_eq!(processes[1].resident_mem_bytes, 2 * 1024 * 1024);
    }

    #[test]
    fn memory_parser_rejects_total_overflow_and_ignores_optional_overflow() {
        let total_overflow = format!("MemTotal: {} kB\n", u64::MAX);
        assert!(super::parse_memory(&total_overflow).is_err());

        let optional_overflow = format!("MemTotal: 1 kB\nMemAvailable: {} kB\n", u64::MAX);
        let memory = super::parse_memory(&optional_overflow).unwrap();
        assert_eq!(memory.used, 1024);
    }

    #[test]
    fn disk_parser_rejects_overflow_and_clamps_percentages() {
        let disks = format!(
            "Filesystem 1024-blocks Used Available Capacity Mounted on\n/dev/overflow {} 0 1 1% /overflow\n/dev/avail-overflow 100 0 {} 1% /avail-overflow\n/dev/one 100 0 50 101% /one\n/dev/two 100 0 50 999% /two\n/dev/invalid 100 0 50 nope /invalid\n/dev/negative 100 0 50 -1% /negative\n",
            u64::MAX
            , u64::MAX
        );

        let parsed = super::parse_disks(&disks);
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].mount, "/one");
        assert_eq!(parsed[0].percent, 100);
        assert_eq!(parsed[1].mount, "/two");
        assert_eq!(parsed[1].percent, 100);
    }

    #[test]
    fn cpu_parser_rejects_aggregate_counter_overflow() {
        let mut parser = RemoteSnapshotParser::default();
        let output = format!("STAT\ncpu {} 1 0 0\nMEM\nMemTotal: 1 kB\nEND\n", u64::MAX);

        assert!(parser.parse(&output, Duration::from_secs(1)).is_err());
    }

    #[test]
    fn reappearing_network_interface_starts_at_zero_rate() {
        let mut parser = RemoteSnapshotParser::default();
        parser.parse(SAMPLE, Duration::from_secs(2)).unwrap();
        let absent = SAMPLE.replacen(" eth0: 1000 0 0 0 0 0 0 0 2000 0 0 0 0 0 0 0\n", "", 1);
        parser.parse(&absent, Duration::from_secs(2)).unwrap();

        let data = parser.parse(SAMPLE, Duration::from_secs(2)).unwrap();
        assert_eq!(data.net_interfaces[0].name, "eth0");
        assert_eq!(data.net_interfaces[0].rx_rate, 0);
        assert_eq!(data.net_interfaces[0].tx_rate, 0);
    }

    #[test]
    fn command_is_fixed_and_contains_all_markers() {
        for fragment in [
            "LC_ALL=C",
            "cat /proc/stat",
            "cat /proc/meminfo",
            "df -Pk",
            "cat /proc/net/dev",
            "cat /proc/loadavg",
            "cat /proc/uptime",
            "ps -eo pid=,user=,stat=,rss=,pcpu=,comm=,lstart=,args= --sort=-pcpu | head -n 100",
            "RssAnon:",
            "ps h -eo stat= | cut -c1 | sort | uniq -c",
            "grep -m1 -E '^(model name|Hardware|Processor)[[:space:]]*:' /proc/cpuinfo",
        ] {
            assert!(REMOTE_SNAPSHOT_COMMAND.contains(fragment));
        }
        let markers = [
            "STAT",
            "MEM",
            "DISK",
            "NETDEFAULT",
            "NET",
            "LOAD",
            "UPTIME",
            "PS",
            "PSANON",
            "PSSTATS",
            "CPUINFO",
            "END",
        ];
        let mut previous = 0;
        for marker in markers {
            let position = REMOTE_SNAPSHOT_COMMAND.find(marker).unwrap();
            assert!(position >= previous);
            previous = position;
        }
        assert!(!REMOTE_SNAPSHOT_COMMAND.contains("{user}"));
        assert!(!REMOTE_SNAPSHOT_COMMAND.contains("{host}"));
        assert!(!REMOTE_SNAPSHOT_COMMAND.contains("$USER"));
        assert!(!REMOTE_SNAPSHOT_COMMAND.contains("$HOST"));
    }

    struct FakeSource {
        results: VecDeque<Result<String, String>>,
        collects: Arc<AtomicUsize>,
        drops: Arc<AtomicUsize>,
    }

    impl SnapshotSource for FakeSource {
        fn collect(&mut self) -> Result<String, String> {
            self.collects.fetch_add(1, Ordering::SeqCst);
            self.results
                .pop_front()
                .unwrap_or_else(|| Err("fake source exhausted".to_string()))
        }
    }

    impl Drop for FakeSource {
        fn drop(&mut self) {
            self.drops.fetch_add(1, Ordering::SeqCst);
        }
    }

    fn timing() -> WorkerTiming {
        WorkerTiming::new(Duration::from_secs(60), Duration::from_secs(60))
    }

    fn source(
        results: impl IntoIterator<Item = Result<String, String>>,
        collects: &Arc<AtomicUsize>,
        drops: &Arc<AtomicUsize>,
    ) -> FakeSource {
        FakeSource {
            results: results.into_iter().collect(),
            collects: Arc::clone(collects),
            drops: Arc::clone(drops),
        }
    }

    struct DetailSource {
        snapshots: VecDeque<Result<String, String>>,
        collects: Arc<AtomicUsize>,
        detail_requests: Arc<Mutex<Vec<u32>>>,
        network_requests: Arc<AtomicUsize>,
    }

    impl SnapshotSource for DetailSource {
        fn collect(&mut self) -> Result<String, String> {
            self.collects.fetch_add(1, Ordering::SeqCst);
            self.snapshots
                .pop_front()
                .unwrap_or_else(|| Err("fake source exhausted".to_string()))
        }

        fn fetch_process_detail(
            &mut self,
            pid: u32,
        ) -> Result<crate::monitor::ProcessDetail, String> {
            self.detail_requests.lock().unwrap().push(pid);
            parse_process_detail(pid, &detail_fixture())
        }

        fn fetch_network_detail(&mut self) -> Result<NetworkDetailSnapshot, String> {
            self.network_requests.fetch_add(1, Ordering::SeqCst);
            parse_network_detail(NETWORK_SAMPLE)
        }
    }

    #[test]
    fn remote_event_debug_never_leaks_error_or_monitor_data() {
        let mut data = RemoteSnapshotParser::default()
            .parse(SAMPLE, Duration::ZERO)
            .unwrap();
        data.cpu_name = "RAW_MONITOR_SENTINEL".to_string();
        let update = format!(
            "{:?}",
            RemoteMonitorEvent::Update {
                key: MonitorKey::remote("user", "host", 22),
                generation: 7,
                data: Box::new(data),
            }
        );
        let failed = format!(
            "{:?}",
            RemoteMonitorEvent::Failed {
                key: MonitorKey::remote("user", "host", 22),
                generation: 8,
                error: "RAW_PASSWORD_SENTINEL".to_string(),
            }
        );
        let mut detail = parse_process_detail(42, &detail_fixture()).unwrap();
        detail.command = "RAW_DETAIL_SENTINEL".to_string();
        detail.environ[0].value = "RAW_ENV_SENTINEL".to_string();
        let detail_event = format!(
            "{:?}",
            RemoteMonitorEvent::ProcessDetail {
                key: MonitorKey::remote("user", "host", 22),
                generation: 9,
                requester: "RAW_REQUESTER_SENTINEL".to_string(),
                request_id: 77,
                result: Ok(Box::new(detail)),
            }
        );
        let mut network = parse_network_detail(NETWORK_SAMPLE).unwrap();
        network.connections[0].process = "RAW_NETWORK_SENTINEL".to_string();
        let network_event = format!(
            "{:?}",
            RemoteMonitorEvent::NetworkDetail {
                key: MonitorKey::remote("user", "host", 22),
                generation: 10,
                requester: "RAW_NETWORK_REQUESTER_SENTINEL".to_string(),
                request_id: 78,
                result: Ok(Box::new(network)),
            }
        );

        assert!(update.contains("Update"));
        assert!(failed.contains("Failed"));
        assert!(detail_event.contains("ProcessDetail"));
        assert!(detail_event.contains("request_id: 77"));
        assert!(network_event.contains("NetworkDetail"));
        assert!(network_event.contains("request_id: 78"));
        assert!(!update.contains("RAW_MONITOR_SENTINEL"));
        assert!(!failed.contains("RAW_PASSWORD_SENTINEL"));
        assert!(!detail_event.contains("RAW_DETAIL_SENTINEL"));
        assert!(!detail_event.contains("RAW_ENV_SENTINEL"));
        assert!(!detail_event.contains("RAW_REQUESTER_SENTINEL"));
        assert!(!network_event.contains("RAW_NETWORK_SENTINEL"));
        assert!(!network_event.contains("RAW_NETWORK_REQUESTER_SENTINEL"));
    }

    #[test]
    fn bounded_reader_accepts_limit_and_rejects_one_byte_over() {
        let at_limit = vec![b'x'; MAX_SNAPSHOT_BYTES];
        assert_eq!(
            read_snapshot_bounded(Cursor::new(at_limit)).unwrap().len(),
            MAX_SNAPSHOT_BYTES
        );

        let too_large = vec![b'x'; MAX_SNAPSHOT_BYTES + 1];
        assert!(read_snapshot_bounded(Cursor::new(too_large)).is_err());
    }

    #[test]
    fn detail_reader_is_independently_bounded() {
        let at_limit = vec![b'x'; MAX_PROCESS_DETAIL_BYTES];
        assert_eq!(
            read_process_detail_bounded(Cursor::new(at_limit))
                .unwrap()
                .len(),
            MAX_PROCESS_DETAIL_BYTES
        );

        let too_large = vec![b'x'; MAX_PROCESS_DETAIL_BYTES + 1];
        assert!(read_process_detail_bounded(Cursor::new(too_large)).is_err());
    }

    #[test]
    fn generated_detail_command_round_trips_for_a_live_process() {
        let pid = std::process::id();
        let output = std::process::Command::new("sh")
            .arg("-c")
            .arg(super::process_detail_command(pid))
            .output()
            .unwrap();

        assert!(output.status.success());
        let protocol = String::from_utf8(output.stdout).unwrap();
        let detail = parse_process_detail(pid, &protocol).unwrap();
        assert_eq!(detail.identity.pid, pid);
        assert!(detail.identity.start_ticks > 0);
        assert!(!detail.name.is_empty());
    }

    #[test]
    fn parses_bounded_process_detail_protocol() {
        let detail = parse_process_detail(42, &detail_fixture()).unwrap();

        assert_eq!(detail.identity.pid, 42);
        assert_eq!(detail.identity.start_ticks, 12345);
        assert_eq!(detail.user, "alice");
        assert_eq!(detail.state, "S");
        assert_eq!(detail.mem_bytes, 2 * 1024 * 1024);
        assert_eq!(detail.mem_mb, "2M");
        let platform_memory = detail.platform_memory.as_ref().unwrap();
        assert_eq!(platform_memory.label, "平台占用（PSS）");
        assert_eq!(platform_memory.bytes, 1536 * 1024);
        assert_eq!(platform_memory.text, "2M");
        assert_eq!(detail.cpu, 3.5);
        assert_eq!(detail.name, "worker");
        assert_eq!(detail.command, "/usr/bin/worker --token secret");
        assert_eq!(detail.executable, "/usr/bin/worker");
        assert_eq!(detail.working_dir, "/srv/app");
        assert_eq!(detail.environ.len(), 2);
        assert_eq!(detail.ancestors.len(), 2);
        assert_eq!(detail.ancestors[1].pid, 1);
    }

    #[test]
    fn detail_parser_rejects_missing_end_and_process_error() {
        let detail = detail_fixture();
        assert!(parse_process_detail(42, detail.trim_end_matches("DETAIL_END\n")).is_err());
        let error = detail_protocol(&[("ERROR", "进程不存在或无权读取".as_bytes())]);
        assert!(parse_process_detail(42, &error).is_err());
    }

    #[test]
    fn detail_parser_caps_environment_and_ancestors() {
        let environment = (0..300)
            .map(|index| format!("KEY{index}=VALUE{index}\0"))
            .collect::<String>();
        let ancestors = (1..=80)
            .map(|pid| format!("{pid}|proc{pid}|command {pid}"))
            .collect::<Vec<_>>()
            .join("\n");
        let output = detail_protocol(&[
            (
                "PROCSTAT",
                b"42 (worker name) S 1 2 3 4 5 6 7 8 9 10 11 12 13 14 15 16 17 18 12345 20",
            ),
            ("STATUS", b"Name:\tworker\n"),
            ("USER", b"alice\n"),
            ("CPU", b"3.5\n"),
            ("CMDLINE", b"/usr/bin/worker\0"),
            ("EXE", b"/usr/bin/worker\n"),
            ("CWD", b"/srv/app\n"),
            ("START", b"Mon Jul 27 10:00:00 2026\n"),
            ("ENV", environment.as_bytes()),
            ("ANCESTORS", ancestors.as_bytes()),
        ]);

        let detail = parse_process_detail(42, &output).unwrap();
        assert_eq!(detail.environ.len(), super::MAX_ENVIRONMENT_ENTRIES);
        assert_eq!(detail.ancestors.len(), super::MAX_ANCESTORS);
    }

    #[test]
    fn detail_parser_keeps_marker_text_and_newlines_inside_encoded_payloads() {
        let command = b"/bin/sh\0-c\0printf 'DETAIL_END\\nENV\\n'\0";
        let environment = b"NOTE=line one\nDETAIL_END\nENV\0TOKEN=secret\0";
        let output = detail_protocol(&[
            (
                "PROCSTAT",
                b"42 (worker name) S 1 2 3 4 5 6 7 8 9 10 11 12 13 14 15 16 17 18 12345 20",
            ),
            ("STATUS", b"Name:\tworker\n"),
            ("USER", b"alice\n"),
            ("CPU", b"3.5\n"),
            ("CMDLINE", command),
            ("EXE", b"/bin/sh\n"),
            ("CWD", b"/tmp\n"),
            ("START", b"Mon Jul 27 10:00:00 2026\n"),
            ("ENV", environment),
            ("ANCESTORS", b"42|worker|DETAIL_END ENV\n"),
        ]);

        let detail = parse_process_detail(42, &output).unwrap();
        assert!(detail.command.contains("DETAIL_END\\nENV\\n"));
        assert_eq!(detail.environ[0].value, "line one\nDETAIL_END\nENV");
        assert_eq!(detail.ancestors[0].command, "DETAIL_END ENV");
    }

    #[test]
    fn detail_parser_normalizes_start_time_whitespace_like_the_list_parser() {
        let output = detail_fixture().replace(
            &encode_hex(b"Mon Jul 27 10:00:00 2026\n"),
            &encode_hex(b"Mon  Jul  27   10:00:00  2026\n"),
        );

        let detail = parse_process_detail(42, &output).unwrap();
        assert_eq!(detail.start_time, "Mon Jul 27 10:00:00 2026");
    }

    #[test]
    fn detail_parser_rejects_invalid_or_oversized_hex_sections() {
        let invalid = detail_fixture().replace(
            &encode_hex(b"/usr/bin/worker\0--token\0secret\0"),
            "not-hex",
        );
        assert!(parse_process_detail(42, &invalid).is_err());

        let oversized = vec![b'x'; super::MAX_DETAIL_FIELD_BYTES + 1];
        let output = detail_protocol(&[
            (
                "PROCSTAT",
                b"42 (worker name) S 1 2 3 4 5 6 7 8 9 10 11 12 13 14 15 16 17 18 12345 20",
            ),
            ("STATUS", b"Name:\tworker\n"),
            ("USER", b"alice\n"),
            ("CPU", b"3.5\n"),
            ("CMDLINE", &oversized),
            ("EXE", b""),
            ("CWD", b""),
            ("START", b""),
            ("ENV", b""),
            ("ANCESTORS", b""),
        ]);
        assert!(parse_process_detail(42, &output).is_err());
    }
