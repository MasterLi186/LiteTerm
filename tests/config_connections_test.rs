use guishell::config::connections::{AuthMethod, ConnectionConfig, ConnectionStore, HostConfig};
use tempfile::TempDir;

#[test]
fn test_add_group_and_host() {
    let mut store = ConnectionStore::default();
    store.add_group("production", "生产环境", "#e74c3c");

    let host = HostConfig {
        label: "Web Server 01".to_string(),
        host: "192.168.1.10".to_string(),
        port: 22,
        user: "root".to_string(),
        auth: AuthMethod::Keyring,
        key_path: String::new(),
        charset: "utf-8".to_string(),
    };
    store.add_host("production", "web01", host);

    let groups = store.groups();
    assert_eq!(groups.len(), 1);
    assert_eq!(groups[0].label, "生产环境");

    let hosts = store.hosts_in_group("production");
    assert_eq!(hosts.len(), 1);
    assert_eq!(hosts[0].1.host, "192.168.1.10");
}

#[test]
fn test_save_and_load_roundtrip() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("connections.toml");

    let mut store = ConnectionStore::default();
    store.add_group("dev", "开发环境", "#2ecc71");
    store.add_host(
        "dev",
        "vm1",
        HostConfig {
            label: "本地虚拟机".to_string(),
            host: "192.168.56.101".to_string(),
            port: 22,
            user: "lfl".to_string(),
            auth: AuthMethod::Keyring,
            key_path: String::new(),
            charset: "utf-8".to_string(),
        },
    );

    store.save_to(&path).unwrap();
    let loaded = ConnectionStore::load_from(&path).unwrap();

    let hosts = loaded.hosts_in_group("dev");
    assert_eq!(hosts.len(), 1);
    assert_eq!(hosts[0].1.label, "本地虚拟机");
}

#[test]
fn test_remove_host() {
    let mut store = ConnectionStore::default();
    store.add_group("test", "测试", "#333");
    store.add_host(
        "test",
        "h1",
        HostConfig {
            label: "H1".to_string(),
            host: "1.2.3.4".to_string(),
            port: 22,
            user: "root".to_string(),
            auth: AuthMethod::Agent,
            key_path: String::new(),
            charset: "utf-8".to_string(),
        },
    );
    assert_eq!(store.hosts_in_group("test").len(), 1);
    store.remove_host("test", "h1");
    assert_eq!(store.hosts_in_group("test").len(), 0);
}

#[test]
fn test_connection_config_from_host() {
    let host = HostConfig {
        label: "Web".to_string(),
        host: "10.0.0.1".to_string(),
        port: 2222,
        user: "deploy".to_string(),
        auth: AuthMethod::Key,
        key_path: "/home/lfl/.ssh/id_ed25519".to_string(),
        charset: "utf-8".to_string(),
    };
    let cfg = ConnectionConfig::from(&host);
    assert_eq!(cfg.host, "10.0.0.1");
    assert_eq!(cfg.port, 2222);
    assert_eq!(cfg.user, "deploy");
}
