use super::*;
use alacritty_terminal::index::Point;
use alacritty_terminal::vte::ansi::StdSyncHandler;
use base64::Engine;
use std::io;
use std::time::Duration;

const TOKEN: &str = "0123456789abcdef0123456789abcdef";
const GENERATION: u64 = 42;

type TestProcessor = alacritty_terminal::vte::ansi::Processor<StdSyncHandler>;

fn completion_session() -> CompletionSessionKey {
    CompletionSessionKey::new_for_test(GENERATION, TOKEN)
}

fn prompt_marker() -> Vec<u8> {
    format!("\x1b]777;LiteTerm;{TOKEN};{GENERATION};P\x07").into_bytes()
}

fn history_marker(path_payload: &str) -> Vec<u8> {
    format!("\x1b]777;LiteTerm;{TOKEN};{GENERATION};H;{path_payload}\x07").into_bytes()
}

fn input_snapshot_marker(session: &CompletionSessionKey, line: &str, point: usize) -> Vec<u8> {
    let payload = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(line.as_bytes());
    format!(
        "\x1b]777;LiteTerm;{};{};I;{point};{payload}\x07",
        session.token(),
        session.generation
    )
    .into_bytes()
}

fn tracked_terminal(cols: u16, rows: u16) -> TerminalState {
    let mut terminal = TerminalState::new();
    terminal.init_term(cols, rows);
    terminal.prompt_tracking = Some(PromptTracking {
        session: completion_session(),
        decoder: MarkerDecoder::new(completion_session()),
        active: false,
        anchor: None,
        snapshot_base: None,
        snapshot_requested_at: None,
        outstanding_snapshot_responses: 0,
        stale_snapshot_responses: 0,
    });
    terminal
}

fn selection_fixture(output: &str) -> TerminalState {
    let mut terminal = TerminalState::new();
    terminal.init_term(24, 4);
    let mut parser = TestProcessor::new();
    terminal.process_pty_output(&mut parser, output.as_bytes());
    terminal
}
include!("tests/part_01.rs");
include!("tests/part_02.rs");
include!("tests/part_03.rs");
include!("tests/part_04.rs");
include!("tests/part_05.rs");
