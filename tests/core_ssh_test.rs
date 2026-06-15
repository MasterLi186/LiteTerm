use guishell::core::session::{SessionEvent, SessionState, SessionStatus};

#[test]
fn test_session_state_transitions() {
    let mut state = SessionState::new();
    assert!(matches!(state.status(), SessionStatus::Disconnected));

    state.set_connecting();
    assert!(matches!(state.status(), SessionStatus::Connecting));

    state.set_connected();
    assert!(matches!(state.status(), SessionStatus::Connected));

    state.set_disconnected("connection lost");
    assert!(matches!(state.status(), SessionStatus::Disconnected));
    assert_eq!(state.last_error(), Some("connection lost"));
}

#[test]
fn test_session_event_channel() {
    let (tx, rx) = std::sync::mpsc::channel::<SessionEvent>();
    tx.send(SessionEvent::Connected).unwrap();
    tx.send(SessionEvent::DataReceived(vec![72, 101, 108, 108, 111]))
        .unwrap();
    tx.send(SessionEvent::Disconnected("bye".to_string()))
        .unwrap();

    assert!(matches!(rx.recv().unwrap(), SessionEvent::Connected));
    if let SessionEvent::DataReceived(data) = rx.recv().unwrap() {
        assert_eq!(&data, b"Hello");
    } else {
        panic!("expected DataReceived");
    }
    assert!(matches!(
        rx.recv().unwrap(),
        SessionEvent::Disconnected(_)
    ));
}
