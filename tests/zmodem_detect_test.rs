use guishell::core::zmodem::detect::{DetectResult, ZmodemDetector};

#[test]
fn test_no_zmodem_passthrough() {
    let mut detector = ZmodemDetector::new();
    let input = b"normal terminal output here\r\n";
    let result = detector.feed(input);
    match result {
        DetectResult::Normal(data) => assert_eq!(&data, input),
        _ => panic!("expected Normal"),
    }
}

#[test]
fn test_detect_sz_initiation() {
    let mut detector = ZmodemDetector::new();
    // Build a valid ZRQINIT frame using the frame module
    use guishell::core::zmodem::frame::{encode_zhex_header, FrameType, ZmodemFrame};
    let frame = ZmodemFrame {
        frame_type: FrameType::ZRQINIT,
        flags: [0; 4],
    };
    let encoded = encode_zhex_header(&frame);

    let mut input = Vec::new();
    input.extend_from_slice(b"some text before");
    input.extend_from_slice(&encoded);

    let result = detector.feed(&input);
    assert!(matches!(result, DetectResult::ZmodemStart { .. }));
}

#[test]
fn test_partial_match_buffered() {
    let mut detector = ZmodemDetector::new();
    let result1 = detector.feed(b"text**");
    match result1 {
        DetectResult::Normal(data) => assert_eq!(&data, b"text"),
        _ => panic!("expected Normal with 'text'"),
    }
    let result2 = detector.feed(b"not zmodem");
    assert!(matches!(result2, DetectResult::Normal(_)));
}
