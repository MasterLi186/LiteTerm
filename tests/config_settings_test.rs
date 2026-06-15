use guishell::config::settings::Settings;
use std::path::PathBuf;
use tempfile::TempDir;

#[test]
fn test_default_settings() {
    let s = Settings::default();
    assert_eq!(s.terminal.font, "Monospace 12");
    assert_eq!(s.terminal.scrollback_lines, 10000);
    assert_eq!(s.ssh.keepalive_interval_secs, 30);
    assert!(s.zmodem.enabled);
}

#[test]
fn test_save_and_load_roundtrip() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("settings.toml");

    let mut s = Settings::default();
    s.terminal.font = "JetBrains Mono 14".to_string();
    s.ssh.connect_timeout_secs = 20;

    s.save_to(&path).unwrap();
    let loaded = Settings::load_from(&path).unwrap();

    assert_eq!(loaded.terminal.font, "JetBrains Mono 14");
    assert_eq!(loaded.ssh.connect_timeout_secs, 20);
    assert_eq!(loaded.appearance.sidebar_width, 220);
}

#[test]
fn test_load_missing_file_returns_default() {
    let path = PathBuf::from("/tmp/nonexistent_guishell_settings.toml");
    let s = Settings::load_from(&path).unwrap();
    assert_eq!(s.terminal.font, "Monospace 12");
}
