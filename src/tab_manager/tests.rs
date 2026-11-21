use super::*;
use crate::bash_integration::RemoteBashRuntime;
use crate::monitor::MonitorKey;
use std::io::{self, Read};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

fn test_ssh_connection() -> SshConnection {
    SshConnection {
        label: "测试".into(),
        host: "127.0.0.1".into(),
        port: 22,
        user: "test".into(),
        auth: "key".into(),
        key_path: String::new(),
        password: String::new(),
        group: String::new(),
        group_color: [0, 0, 0],
    }
}

fn test_ssh_connection_for(host: &str, user: &str, port: u16) -> SshConnection {
    SshConnection {
        label: format!("{user}@{host}"),
        host: host.into(),
        port,
        user: user.into(),
        auth: "key".into(),
        key_path: String::new(),
        password: String::new(),
        group: String::new(),
        group_color: [0, 0, 0],
    }
}
include!("tests/part_01.rs");
include!("tests/part_02.rs");
include!("tests/part_03.rs");
