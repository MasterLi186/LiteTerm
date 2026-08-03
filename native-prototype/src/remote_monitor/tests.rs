use std::{
    collections::VecDeque,
    io::{self, Cursor},
    sync::{
        atomic::{AtomicUsize, Ordering},
        mpsc, Arc, Mutex,
    },
    time::Duration,
};

use crate::{
    monitor::MonitorKey,
    network_detail::{parse_network_detail, NetworkDetailSnapshot},
};

use super::{
    parse_process_detail, read_process_detail_bounded, read_snapshot_bounded,
    start_worker_with_sink_for_test, start_worker_with_sink_for_test_and_spawner,
    RemoteMonitorCommand, RemoteMonitorEvent, RemoteSnapshotParser, SnapshotSource, WorkerTiming,
    MAX_PROCESS_DETAIL_BYTES, MAX_SNAPSHOT_BYTES, REMOTE_SNAPSHOT_COMMAND,
};

const SAMPLE: &str = "STAT\ncpu  100 0 50 800 50 0 0 0 0 0\ncpu0 50 0 25 400 25 0 0 0 0 0\nMEM\nMemTotal:       2097152 kB\nMemAvailable:   1073152 kB\nSwapTotal:       512000 kB\nSwapFree:        256000 kB\nDISK\nFilesystem 1024-blocks Used Available Capacity Mounted on\n/dev/sda1 1048576 524288 524288 50% /\nNETDEFAULT\neth0\nNET\nInter-|   Receive                                                |  Transmit\n eth0: 1000 0 0 0 0 0 0 0 2000 0 0 0 0 0 0 0\n lo: 999 0 0 0 0 0 0 0 999 0 0 0 0 0 0 0\nLOAD\n0.10 0.20 0.30 1/100 100\nUPTIME\n90060.00 0.00\nPS\n42 alice S 10240 12.5 worker Mon Jul 27 10:00:00 2026 /usr/bin/test process --flag\nPSANON\n42 4096\nPSSTATS\n1 R\n7 S\n2 D\n3 I\n4 Z\n5 T\n6 t\nCPUINFO\nmodel name : Example CPU\nEND\n";
const NETWORK_SAMPLE: &str = "===IP===\neth0 10.0.0.2/24\n===SS===\nESTAB 0 0 10.0.0.2:22 10.0.0.10:50000 users:((\"sshd\",pid=42,fd=4))\n";

fn encode_hex(value: &[u8]) -> String {
    value.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn detail_protocol(sections: &[(&str, &[u8])]) -> String {
    let mut output = String::from("DETAIL_V1\n");
    for (marker, value) in sections {
        output.push_str(marker);
        output.push('\n');
        output.push_str(&encode_hex(value));
        output.push('\n');
    }
    output.push_str("DETAIL_END\n");
    output
}

fn detail_fixture() -> String {
    detail_protocol(&[
        (
            "PROCSTAT",
            b"42 (worker name) S 1 2 3 4 5 6 7 8 9 10 11 12 13 14 15 16 17 18 12345 20",
        ),
        (
            "STATUS",
            b"Name:\tworker\nState:\tS (sleeping)\nVmRSS:\t2048 kB\n",
        ),
        ("PSS", b"Pss:\t1536 kB\n"),
        ("USER", b"alice\n"),
        ("CPU", b"3.5\n"),
        ("CMDLINE", b"/usr/bin/worker\0--token\0secret\0"),
        ("EXE", b"/usr/bin/worker\n"),
        ("CWD", b"/srv/app\n"),
        ("START", b"Mon Jul 27 10:00:00 2026\n"),
        ("ENV", b"PATH=/usr/bin\0TOKEN=secret\0"),
        (
            "ANCESTORS",
            b"42|worker|/usr/bin/worker --token secret\n1|init|/sbin/init\n",
        ),
    ])
}
include!("tests/part_01.rs");
include!("tests/part_02.rs");
include!("tests/part_03.rs");
