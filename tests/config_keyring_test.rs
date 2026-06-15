use guishell::config::keyring::KeyringEntry;

#[test]
fn test_keyring_entry_label_format() {
    let entry = KeyringEntry::new("root", "192.168.1.10", 22);
    assert_eq!(entry.label(), "guishell:ssh://root@192.168.1.10:22");
}

#[test]
fn test_keyring_entry_label_nonstandard_port() {
    let entry = KeyringEntry::new("deploy", "10.0.0.5", 2222);
    assert_eq!(entry.label(), "guishell:ssh://deploy@10.0.0.5:2222");
}
