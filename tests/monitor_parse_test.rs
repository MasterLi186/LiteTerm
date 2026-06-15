use guishell::core::monitor::{CpuMetric, MemoryMetric, DiskMetric, NetworkMetric, LoadMetric, parse_proc_stat_cpu, parse_proc_meminfo, parse_df_output, parse_proc_net_dev, parse_loadavg, MetricBuffer};

#[test]
fn test_parse_proc_stat_cpu() {
    let input = "cpu  4705 356 584 3699 23 0 0 0 0 0\ncpu0 2353 178 292 1849 12 0 0 0 0 0\n";
    let metrics = parse_proc_stat_cpu(input);
    assert_eq!(metrics.len(), 2);
    assert_eq!(metrics[0].label, "cpu");
    assert!(metrics[0].user > 0);
    assert_eq!(metrics[1].label, "cpu0");
}

#[test]
fn test_parse_meminfo() {
    let input = "MemTotal:        8028508 kB\nMemFree:          204508 kB\nMemAvailable:    2458792 kB\nBuffers:          123456 kB\nCached:          1234567 kB\nSwapTotal:       2097148 kB\nSwapFree:        2097148 kB\n";
    let mem = parse_proc_meminfo(input).unwrap();
    assert_eq!(mem.total_kb, 8028508);
    assert_eq!(mem.free_kb, 204508);
    assert_eq!(mem.cached_kb, 1234567);
}

#[test]
fn test_parse_df_output() {
    let input = "Filesystem      Size  Used Avail Use% Mounted on\n/dev/sda1        50G   34G   14G  71% /\n/dev/sda2       200G   46G  144G  25% /home\n";
    let disks = parse_df_output(input);
    assert_eq!(disks.len(), 2);
    assert_eq!(disks[0].mount_point, "/");
    assert_eq!(disks[0].use_percent, 71);
    assert_eq!(disks[1].mount_point, "/home");
}

#[test]
fn test_parse_net_dev() {
    let input = "Inter-|   Receive                                                |  Transmit\n face |bytes    packets errs drop fifo frame compressed multicast|bytes    packets errs drop fifo colls carrier compressed\n  eth0: 123456789   12345    0    0    0     0          0         0 987654321   54321    0    0    0     0       0          0\n    lo:   456789    1234    0    0    0     0          0         0   456789    1234    0    0    0     0       0          0\n";
    let nets = parse_proc_net_dev(input);
    assert!(nets.len() >= 1);
    let eth0 = nets.iter().find(|n| n.interface == "eth0").unwrap();
    assert_eq!(eth0.rx_bytes, 123456789);
    assert_eq!(eth0.tx_bytes, 987654321);
}

#[test]
fn test_parse_loadavg() {
    let input = "1.23 0.87 0.45 3/234 12345\n";
    let load = parse_loadavg(input).unwrap();
    assert!((load.load_1m - 1.23).abs() < 0.01);
    assert!((load.load_5m - 0.87).abs() < 0.01);
    assert!((load.load_15m - 0.45).abs() < 0.01);
}

#[test]
fn test_metric_buffer_ring() {
    let mut buf = MetricBuffer::<f64>::new(3);
    buf.push(1.0);
    buf.push(2.0);
    buf.push(3.0);
    assert_eq!(buf.as_slice(), &[1.0, 2.0, 3.0]);
    buf.push(4.0);
    assert_eq!(buf.as_slice(), &[2.0, 3.0, 4.0]);
}
