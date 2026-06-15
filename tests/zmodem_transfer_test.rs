use guishell::core::zmodem::receive::ZmodemReceiver;
use guishell::core::zmodem::send::ZmodemSender;

#[test]
fn test_receiver_initial_state() {
    let receiver = ZmodemReceiver::new("/tmp");
    assert!(!receiver.is_complete());
    assert_eq!(receiver.files_received(), 0);
}

#[test]
fn test_receiver_generates_zrinit() {
    let receiver = ZmodemReceiver::new("/tmp");
    let init = receiver.make_zrinit();
    assert!(init.starts_with(b"**\x18B"));
}

#[test]
fn test_sender_initial_state() {
    let sender = ZmodemSender::new();
    assert!(!sender.is_complete());
}

#[test]
fn test_sender_generates_zrqinit() {
    let sender = ZmodemSender::new();
    let init = sender.make_zrqinit();
    assert!(init.starts_with(b"**\x18B"));
}

#[test]
fn test_receiver_parse_zfile_extracts_filename() {
    let receiver = ZmodemReceiver::new("/tmp");
    let subpacket = b"testfile.txt\x0012345 1718300000\x00";
    let (name, size) = receiver.parse_zfile_subpacket(subpacket).unwrap();
    assert_eq!(name, "testfile.txt");
    assert_eq!(size, 12345);
}

#[test]
fn test_receiver_parse_zfile_no_size() {
    let receiver = ZmodemReceiver::new("/tmp");
    let subpacket = b"myfile.bin\x00\x00";
    let (name, size) = receiver.parse_zfile_subpacket(subpacket).unwrap();
    assert_eq!(name, "myfile.bin");
    assert_eq!(size, 0);
}
