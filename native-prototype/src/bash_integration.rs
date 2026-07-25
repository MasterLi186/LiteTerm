use base64::Engine;
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use tempfile::TempDir;

use crate::smart_completion::CompletionSessionKey;

pub const MAX_OSC_FRAME: usize = 256;

pub fn is_bash_path(path: &str) -> bool {
    Path::new(path)
        .file_name()
        .is_some_and(|name| name == "bash")
}

pub fn widget_sequence(session: &CompletionSessionKey) -> String {
    let numeric = session
        .token()
        .get(..8)
        .and_then(|prefix| u32::from_str_radix(prefix, 16).ok())
        .unwrap_or(777);
    format!("\x1b[777;{numeric}~")
}

pub fn readline_literal(sequence: &str) -> String {
    sequence.replace('\x1b', "\\e")
}

fn shell_single_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\"'\"'"))
}

pub fn build_bash_rc(
    session: &CompletionSessionKey,
    candidate_path: &Path,
    sequence: &str,
) -> String {
    let candidate = shell_single_quote(&candidate_path.to_string_lossy());
    let key = readline_literal(sequence);
    let token = session.token();
    let generation = session.generation;

    format!(
        r#"[[ -r "$HOME/.bashrc" ]] && source "$HOME/.bashrc"

if ! declare -p __liteterm_candidate >/dev/null 2>&1; then
    readonly __liteterm_candidate={candidate}
fi

__liteterm_fill() {{
    READLINE_LINE=$(<"$__liteterm_candidate")
    READLINE_POINT=${{#READLINE_LINE}}
}}

__liteterm_install_bindings() {{
    builtin bind -m emacs-standard -x '"{key}":__liteterm_fill'
    builtin bind -m vi-insert -x '"{key}":__liteterm_fill'
    builtin bind -m vi-command -x '"{key}":__liteterm_fill'
}}

if [[ -z ${{__liteterm_history_sent+x}} ]]; then
    __liteterm_history_sent=0
fi

if ! declare -p __liteterm_prompt_marker >/dev/null 2>&1; then
    readonly __liteterm_prompt_marker='\[\e]777;LiteTerm;{token};{generation};P\a\]'
fi

__liteterm_ensure_prompt_marker() {{
    case "${{PS1-}}" in
        *"$__liteterm_prompt_marker") ;;
        *) PS1="${{PS1-}}${{__liteterm_prompt_marker}}" ;;
    esac
}}

__liteterm_prompt_hook() {{
    __liteterm_install_bindings
    if [[ ${{__liteterm_history_sent-0}} != 1 ]]; then
        if command -v base64 >/dev/null 2>&1 && command -v tr >/dev/null 2>&1; then
            local __liteterm_history_path=${{HISTFILE:-"$HOME/.bash_history"}}
            local __liteterm_history_payload
            __liteterm_history_payload=$(printf '%s' "$__liteterm_history_path" | base64 | tr '+/' '-_' | tr -d '=\n')
            printf '\e]777;LiteTerm;{token};{generation};H;%s\a' "$__liteterm_history_payload"
            __liteterm_history_sent=1
        fi
    fi
    __liteterm_ensure_prompt_marker
}}

__liteterm_prompt_declaration=$(declare -p PROMPT_COMMAND 2>/dev/null || :)
if [[ $__liteterm_prompt_declaration == "declare -a "* ]]; then
    __liteterm_has_prompt_hook=0
    for __liteterm_prompt_entry in "${{PROMPT_COMMAND[@]}}"; do
        if [[ $__liteterm_prompt_entry == __liteterm_prompt_hook ]]; then
            __liteterm_has_prompt_hook=1
            break
        fi
    done
    if [[ $__liteterm_has_prompt_hook != 1 ]]; then
        PROMPT_COMMAND+=(__liteterm_prompt_hook)
    fi
else
    case ";${{PROMPT_COMMAND-}};" in
        *";__liteterm_prompt_hook;"*) ;;
        *)
            if [[ -n ${{PROMPT_COMMAND-}} ]]; then
                PROMPT_COMMAND="${{PROMPT_COMMAND%;}};__liteterm_prompt_hook"
            else
                PROMPT_COMMAND=__liteterm_prompt_hook
            fi
            ;;
    esac
fi
"#
    )
}

pub struct LocalBashRuntime {
    temp_dir: TempDir,
    session: CompletionSessionKey,
    rc_path: PathBuf,
    candidate_path: PathBuf,
    widget_sequence: String,
}

impl LocalBashRuntime {
    pub fn create(session: CompletionSessionKey) -> io::Result<Self> {
        let temp_dir = tempfile::Builder::new()
            .prefix("liteterm-bash-")
            .tempdir()?;
        fs::set_permissions(temp_dir.path(), fs::Permissions::from_mode(0o700))?;

        let rc_path = temp_dir.path().join("bashrc");
        let candidate_path = temp_dir.path().join("candidate");
        let widget_sequence = widget_sequence(&session);
        let rc = build_bash_rc(&session, &candidate_path, &widget_sequence);
        write_new_private_file(&rc_path, rc.as_bytes())?;
        write_new_private_file(&candidate_path, b"")?;

        Ok(Self {
            temp_dir,
            session,
            rc_path,
            candidate_path,
            widget_sequence,
        })
    }

    pub fn temp_dir(&self) -> &Path {
        self.temp_dir.path()
    }

    pub fn session(&self) -> &CompletionSessionKey {
        &self.session
    }

    pub fn rc_path(&self) -> &Path {
        &self.rc_path
    }

    pub fn candidate_path(&self) -> &Path {
        &self.candidate_path
    }

    pub fn widget_sequence(&self) -> &str {
        &self.widget_sequence
    }

    pub fn write_candidate(&self, candidate: &str) -> io::Result<()> {
        if candidate.is_empty() || candidate.chars().any(char::is_control) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "候选命令不能为空或包含控制字符",
            ));
        }

        let next_path = self.candidate_path.with_extension("next");
        let result = (|| {
            write_new_private_file(&next_path, candidate.as_bytes())?;
            fs::rename(&next_path, &self.candidate_path)
        })();
        if result.is_err() {
            let _ = fs::remove_file(&next_path);
        }
        result
    }
}

fn write_new_private_file(path: &Path, contents: &[u8]) -> io::Result<()> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)?;
    file.set_permissions(fs::Permissions::from_mode(0o600))?;
    file.write_all(contents)?;
    file.sync_all()
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MarkerKind {
    Prompt,
    HistoryPath(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MarkerBoundary {
    pub end_offset: usize,
    pub kind: MarkerKind,
}

pub struct MarkerDecoder {
    session: CompletionSessionKey,
    ground_escape: bool,
    in_osc: bool,
    osc_escape: bool,
    overflow: bool,
    frame: Vec<u8>,
}

impl MarkerDecoder {
    pub fn new(session: CompletionSessionKey) -> Self {
        Self {
            session,
            ground_escape: false,
            in_osc: false,
            osc_escape: false,
            overflow: false,
            frame: Vec::with_capacity(MAX_OSC_FRAME),
        }
    }

    pub fn scan(&mut self, chunk: &[u8]) -> Vec<MarkerBoundary> {
        let mut markers = Vec::new();

        for (offset, &byte) in chunk.iter().enumerate() {
            if !self.in_osc {
                if self.ground_escape {
                    self.ground_escape = byte == b'\x1b';
                    if byte == b']' {
                        self.in_osc = true;
                        self.ground_escape = false;
                        self.osc_escape = false;
                        self.overflow = false;
                        self.frame.clear();
                    }
                } else if byte == b'\x1b' {
                    self.ground_escape = true;
                }
                continue;
            }

            if self.osc_escape {
                self.osc_escape = false;
                if byte == b'\\' {
                    self.finish_frame(offset + 1, &mut markers);
                    continue;
                }

                self.push_frame_byte(b'\x1b');
                if byte == b'\x07' {
                    self.finish_frame(offset + 1, &mut markers);
                } else if byte == b'\x1b' {
                    self.osc_escape = true;
                } else {
                    self.push_frame_byte(byte);
                }
            } else if byte == b'\x07' {
                self.finish_frame(offset + 1, &mut markers);
            } else if byte == b'\x1b' {
                self.osc_escape = true;
            } else {
                self.push_frame_byte(byte);
            }
        }

        markers
    }

    fn push_frame_byte(&mut self, byte: u8) {
        if self.overflow {
            return;
        }
        if self.frame.len() == MAX_OSC_FRAME {
            self.overflow = true;
            self.frame.clear();
        } else {
            self.frame.push(byte);
        }
    }

    fn finish_frame(&mut self, end_offset: usize, markers: &mut Vec<MarkerBoundary>) {
        if !self.overflow {
            if let Some(kind) = parse_marker(&self.frame, &self.session) {
                markers.push(MarkerBoundary { end_offset, kind });
            }
        }

        self.ground_escape = false;
        self.in_osc = false;
        self.osc_escape = false;
        self.overflow = false;
        self.frame.clear();
    }
}

fn parse_marker(frame: &[u8], session: &CompletionSessionKey) -> Option<MarkerKind> {
    let text = std::str::from_utf8(frame).ok()?;
    if text.chars().any(char::is_control) {
        return None;
    }

    let fields = text.split(';').collect::<Vec<_>>();
    if fields.get(0) != Some(&"777")
        || fields.get(1) != Some(&"LiteTerm")
        || fields.get(2) != Some(&session.token())
        || fields.get(3)?.parse::<u64>().ok()? != session.generation
    {
        return None;
    }

    match fields.as_slice() {
        [_, _, _, _, "P"] => Some(MarkerKind::Prompt),
        [_, _, _, _, "H", payload] => {
            let decoded = base64::engine::general_purpose::URL_SAFE_NO_PAD
                .decode(payload)
                .ok()?;
            if base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(&decoded) != *payload {
                return None;
            }
            let path = std::str::from_utf8(&decoded).ok()?;
            if path.is_empty() || !path.starts_with('/') || path.chars().any(char::is_control) {
                return None;
            }
            Some(MarkerKind::HistoryPath(path.to_owned()))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::engine::general_purpose::URL_SAFE_NO_PAD;
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
    use std::process::{Command, Output};

    const TOKEN: &str = "0123456789abcdef0123456789abcdef";
    const GENERATION: u64 = 42;

    fn session() -> CompletionSessionKey {
        CompletionSessionKey::new_for_test(GENERATION, TOKEN)
    }

    fn prompt_body() -> String {
        format!("777;LiteTerm;{TOKEN};{GENERATION};P")
    }

    fn history_body(path: &[u8]) -> String {
        format!(
            "777;LiteTerm;{TOKEN};{GENERATION};H;{}",
            URL_SAFE_NO_PAD.encode(path)
        )
    }

    fn run_bash(runtime: &LocalBashRuntime, home: &Path, script: &str) -> Output {
        Command::new("bash")
            .args(["--noprofile", "--norc", "-c", script, "liteterm-test"])
            .arg(runtime.rc_path())
            .env("HOME", home)
            .env("LC_ALL", "C")
            .output()
            .unwrap()
    }

    #[test]
    fn bash_path_requires_bash_basename() {
        assert!(is_bash_path("bash"));
        assert!(is_bash_path("/bin/bash"));
        assert!(is_bash_path("/usr/local/bin/bash"));
        assert!(!is_bash_path("/bin/sh"));
        assert!(is_bash_path("/tmp/bash/"));
        assert!(!is_bash_path("bash.exe"));
    }

    #[test]
    fn widget_sequence_is_stable_and_token_specific() {
        let first = CompletionSessionKey::new_for_test(1, "01234567aaaaaaaa");
        let same_prefix = CompletionSessionKey::new_for_test(99, "01234567bbbbbbbb");
        let different = CompletionSessionKey::new_for_test(1, "89abcdefaaaaaaaa");
        let malformed = CompletionSessionKey::new_for_test(1, "not-hex");

        assert_eq!(widget_sequence(&first), "\x1b[777;19088743~");
        assert_eq!(widget_sequence(&same_prefix), widget_sequence(&first));
        assert_ne!(widget_sequence(&different), widget_sequence(&first));
        assert_eq!(widget_sequence(&malformed), "\x1b[777;777~");
    }

    #[test]
    fn readline_literal_only_escapes_escape_bytes() {
        assert_eq!(readline_literal("\x1b[777;42~\\\"'$"), "\\e[777;42~\\\"'$");
    }

    #[test]
    fn bash_rc_sources_user_rc_preserves_prompt_command_and_installs_widget() {
        let session = CompletionSessionKey::new_for_test(42, TOKEN);
        let sequence = widget_sequence(&session);
        let rc = build_bash_rc(&session, std::path::Path::new("/tmp/candidate"), &sequence);

        assert!(rc.contains("source \"$HOME/.bashrc\""));
        assert!(rc.contains("declare -p PROMPT_COMMAND"));
        assert!(rc.contains("PROMPT_COMMAND+=("));
        assert!(rc.contains("PROMPT_COMMAND=\"${PROMPT_COMMAND%;};__liteterm_prompt_hook\""));
        for keymap in ["emacs-standard", "vi-insert", "vi-command"] {
            assert!(rc.contains(&format!("bind -m {keymap} -x")));
        }
        assert!(rc.contains("READLINE_LINE=$(<\"$__liteterm_candidate\")"));
        assert!(rc.contains("READLINE_POINT=${#READLINE_LINE}"));
        assert!(!rc.contains("eval"));
        assert!(!rc.contains("accept-line"));
        assert!(rc.contains(&format!("777;LiteTerm;{TOKEN};42;P")));
        assert!(rc.contains("${HISTFILE:-\"$HOME/.bash_history\"}"));
        assert!(rc.contains("tr '+/' '-_'"));
        assert!(rc.contains("tr -d '=\\n'"));
    }

    #[test]
    fn bash_rc_keeps_prompt_marker_as_readline_nonprinting_literal() {
        let session = CompletionSessionKey::new_for_test(42, TOKEN);
        let rc = build_bash_rc(
            &session,
            std::path::Path::new("/tmp/candidate"),
            &widget_sequence(&session),
        );
        let marker =
            format!("readonly __liteterm_prompt_marker='\\[\\e]777;LiteTerm;{TOKEN};42;P\\a\\]'");

        assert!(rc.contains(&marker));
        assert!(!rc.contains("readonly __liteterm_prompt_marker=$'\\e"));
        assert!(rc.contains("*) PS1=\"${PS1-}${__liteterm_prompt_marker}\" ;;"));
    }

    #[test]
    fn bash_rc_guards_history_marker_on_both_encoder_commands() {
        let session = CompletionSessionKey::new_for_test(42, TOKEN);
        let rc = build_bash_rc(
            &session,
            std::path::Path::new("/tmp/candidate"),
            &widget_sequence(&session),
        );
        let guard = "if command -v base64 >/dev/null 2>&1 && command -v tr >/dev/null 2>&1; then";
        let guarded_start = rc.find(guard).expect("history encoder guard");
        let guarded_end = rc[guarded_start..]
            .find("\n        fi")
            .map(|offset| guarded_start + offset)
            .expect("history encoder guard end");
        let guarded_block = &rc[guarded_start..guarded_end];

        assert!(guarded_block.contains(";H;%s\\a'"));
        assert!(guarded_block.contains("__liteterm_history_sent=1"));
    }

    #[test]
    fn generated_widget_is_bound_in_real_bash_vi_insert_keymap() {
        let runtime = LocalBashRuntime::create(session()).unwrap();
        let home = tempfile::tempdir().unwrap();
        fs::write(home.path().join(".bashrc"), "").unwrap();

        let output = run_bash(
            &runtime,
            home.path(),
            r#"
source "$1"
__liteterm_install_bindings
bind -m vi-insert -X
"#,
        );
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        let expected = format!(
            "\"{}\": \"__liteterm_fill\"",
            readline_literal(runtime.widget_sequence())
        );

        assert!(output.status.success(), "stderr: {stderr}");
        assert!(!stderr.contains("invalid keymap"), "stderr: {stderr}");
        assert!(stdout.contains(&expected), "stdout: {stdout}");
    }

    #[test]
    fn generated_widget_uses_builtin_bind_when_user_rc_shadows_bind() {
        let runtime = LocalBashRuntime::create(session()).unwrap();
        let home = tempfile::tempdir().unwrap();
        fs::write(home.path().join(".bashrc"), "bind() { return 99; }\n").unwrap();

        let output = run_bash(
            &runtime,
            home.path(),
            r#"
source "$1"
__liteterm_install_bindings
builtin bind -m vi-insert -X
"#,
        );
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        let expected = format!(
            "\"{}\": \"__liteterm_fill\"",
            readline_literal(runtime.widget_sequence())
        );

        assert!(output.status.success(), "stderr: {stderr}");
        assert!(stdout.contains(&expected), "stdout: {stdout}");
    }

    #[test]
    fn prompt_hook_runs_after_user_array_hooks_and_restores_marker_once() {
        let runtime = LocalBashRuntime::create(session()).unwrap();
        let home = tempfile::tempdir().unwrap();
        fs::write(
            home.path().join(".bashrc"),
            "PROMPT_COMMAND=('PS1=dynamic')\n",
        )
        .unwrap();

        let output = run_bash(
            &runtime,
            home.path(),
            r#"
source "$1"
for __test_prompt in 1 2; do
    for __test_command in "${PROMPT_COMMAND[@]}"; do
        eval "$__test_command"
    done
done
printf '%s' "$PS1"
"#,
        );
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        let marker = format!("\\[\\e]777;LiteTerm;{TOKEN};{GENERATION};P\\a\\]");

        assert!(output.status.success(), "stderr: {stderr}");
        assert!(stdout.ends_with(&marker), "stdout: {stdout:?}");
        assert_eq!(stdout.matches(&marker).count(), 1, "stdout: {stdout:?}");
    }

    #[test]
    fn sourcing_generated_rc_twice_keeps_readonly_values_and_history_state() {
        let runtime = LocalBashRuntime::create(session()).unwrap();
        let home = tempfile::tempdir().unwrap();
        fs::write(home.path().join(".bashrc"), "").unwrap();

        let output = run_bash(
            &runtime,
            home.path(),
            r#"
source "$1"
__liteterm_history_sent=1
source "$1"
printf '%s' "$__liteterm_history_sent"
"#,
        );
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);

        assert!(output.status.success(), "stderr: {stderr}");
        assert!(!stderr.contains("readonly variable"), "stderr: {stderr}");
        assert_eq!(stdout, "1");
    }

    #[test]
    fn local_runtime_creates_private_files_and_atomically_updates_candidate() {
        let runtime = LocalBashRuntime::create(session()).unwrap();

        assert_eq!(
            fs::metadata(runtime.temp_dir())
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
        assert_eq!(
            fs::metadata(runtime.rc_path())
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
        assert_eq!(
            fs::metadata(runtime.candidate_path())
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
        assert!(fs::read_to_string(runtime.rc_path())
            .unwrap()
            .contains("READLINE_LINE=$(<\"$__liteterm_candidate\")"));
        assert_eq!(runtime.session(), &session());
        assert_eq!(runtime.widget_sequence(), widget_sequence(&session()));

        runtime.write_candidate("git status").unwrap();
        assert_eq!(
            fs::read_to_string(runtime.candidate_path()).unwrap(),
            "git status"
        );
        assert!(!runtime.candidate_path().with_extension("next").exists());
    }

    #[test]
    fn invalid_candidate_does_not_replace_existing_candidate() {
        let runtime = LocalBashRuntime::create(session()).unwrap();
        runtime.write_candidate("cargo test").unwrap();

        for invalid in ["", "line\nbreak", "tab\tvalue", "\u{7f}"] {
            assert!(runtime.write_candidate(invalid).is_err());
            assert_eq!(
                fs::read_to_string(runtime.candidate_path()).unwrap(),
                "cargo test"
            );
            assert!(!runtime.candidate_path().with_extension("next").exists());
        }
    }

    fn bel_frame(body: impl AsRef<[u8]>) -> Vec<u8> {
        let mut frame = b"\x1b]".to_vec();
        frame.extend_from_slice(body.as_ref());
        frame.push(b'\x07');
        frame
    }

    fn st_frame(body: impl AsRef<[u8]>) -> Vec<u8> {
        let mut frame = b"\x1b]".to_vec();
        frame.extend_from_slice(body.as_ref());
        frame.extend_from_slice(b"\x1b\\");
        frame
    }

    #[test]
    fn bel_marker_reports_chunk_exclusive_end_offset() {
        let mut decoder = MarkerDecoder::new(session());
        let frame = bel_frame(prompt_body());
        let mut chunk = b"ordinary-before".to_vec();
        chunk.extend_from_slice(&frame);
        let expected_end = chunk.len();
        chunk.extend_from_slice(b"ordinary-after");

        assert_eq!(
            decoder.scan(&chunk),
            vec![MarkerBoundary {
                end_offset: expected_end,
                kind: MarkerKind::Prompt,
            }]
        );
    }

    #[test]
    fn st_split_across_chunks_decodes_absolute_history_path() {
        let mut decoder = MarkerDecoder::new(session());
        let path = b"/home/test/.bash_history";
        let frame = st_frame(history_body(path));
        let split = frame.len() - 1;
        let mut first = b"before".to_vec();
        first.extend_from_slice(&frame[..split]);

        assert!(decoder.scan(&first).is_empty());
        assert_eq!(
            decoder.scan(&[frame[split], b'a', b'f', b't', b'e', b'r']),
            vec![MarkerBoundary {
                end_offset: 1,
                kind: MarkerKind::HistoryPath("/home/test/.bash_history".to_owned()),
            }]
        );
    }

    #[test]
    fn invalid_and_oversized_frames_are_ignored_then_decoder_recovers() {
        let mut decoder = MarkerDecoder::new(session());
        let wrong_token = bel_frame(format!(
            "777;LiteTerm;ffffffffffffffffffffffffffffffff;{GENERATION};P"
        ));
        let wrong_generation = bel_frame(format!("777;LiteTerm;{TOKEN};{};P", GENERATION + 1));
        let oversized = bel_frame(vec![b'x'; MAX_OSC_FRAME + 1]);
        let valid = bel_frame(prompt_body());
        let mut chunk = Vec::new();
        chunk.extend_from_slice(&wrong_token);
        chunk.extend_from_slice(&wrong_generation);
        chunk.extend_from_slice(&oversized);
        let valid_start = chunk.len();
        chunk.extend_from_slice(&valid);

        assert_eq!(
            decoder.scan(&chunk),
            vec![MarkerBoundary {
                end_offset: valid_start + valid.len(),
                kind: MarkerKind::Prompt,
            }]
        );
    }

    #[test]
    fn multiple_markers_in_one_chunk_have_individual_offsets() {
        let mut decoder = MarkerDecoder::new(session());
        let prompt = bel_frame(prompt_body());
        let history = st_frame(history_body(b"/tmp/history"));
        let mut chunk = b"x".to_vec();
        chunk.extend_from_slice(&prompt);
        let prompt_end = chunk.len();
        chunk.extend_from_slice(b"middle");
        chunk.extend_from_slice(&history);
        let history_end = chunk.len();
        chunk.push(b'y');

        assert_eq!(
            decoder.scan(&chunk),
            vec![
                MarkerBoundary {
                    end_offset: prompt_end,
                    kind: MarkerKind::Prompt,
                },
                MarkerBoundary {
                    end_offset: history_end,
                    kind: MarkerKind::HistoryPath("/tmp/history".to_owned()),
                },
            ]
        );
    }

    #[test]
    fn marker_survives_every_byte_boundary_between_chunks() {
        for frame in [
            bel_frame(prompt_body()),
            st_frame(history_body(b"/var/tmp/bash-history")),
        ] {
            for split in 0..=frame.len() {
                let mut decoder = MarkerDecoder::new(session());
                let first = decoder.scan(&frame[..split]);
                let second = decoder.scan(&frame[split..]);
                let markers = first
                    .into_iter()
                    .chain(second)
                    .collect::<Vec<MarkerBoundary>>();
                let expected_kind = if frame[frame.len() - 1] == b'\x07' {
                    MarkerKind::Prompt
                } else {
                    MarkerKind::HistoryPath("/var/tmp/bash-history".to_owned())
                };
                let expected_offset = if split == frame.len() {
                    frame.len()
                } else {
                    frame.len() - split
                };

                assert_eq!(
                    markers,
                    vec![MarkerBoundary {
                        end_offset: expected_offset,
                        kind: expected_kind,
                    }],
                    "split at {split} of {}",
                    frame.len()
                );
            }
        }
    }

    #[test]
    fn malformed_frames_and_unsafe_paths_are_rejected() {
        let malformed_utf8 = bel_frame(
            [
                format!("777;LiteTerm;{TOKEN};{GENERATION};P").as_bytes(),
                &[0xff],
            ]
            .concat(),
        );
        let malformed_base64 = bel_frame(format!("777;LiteTerm;{TOKEN};{GENERATION};H;%%%"));
        let malformed_path_utf8 = bel_frame(format!(
            "777;LiteTerm;{TOKEN};{GENERATION};H;{}",
            URL_SAFE_NO_PAD.encode([0xff])
        ));
        let relative_path = bel_frame(history_body(b"relative/history"));
        let empty_path = bel_frame(history_body(b""));
        let control_path = bel_frame(history_body(b"/tmp/\nsecret"));
        let prompt_extra = bel_frame(format!("777;LiteTerm;{TOKEN};{GENERATION};P;extra"));
        let history_extra = bel_frame(format!(
            "777;LiteTerm;{TOKEN};{GENERATION};H;{};extra",
            URL_SAFE_NO_PAD.encode(b"/tmp/history")
        ));
        let control_frame = bel_frame(format!(
            "777;LiteTerm;{TOKEN};{GENERATION};H;\n{}",
            URL_SAFE_NO_PAD.encode(b"/tmp/history")
        ));
        let valid = bel_frame(prompt_body());
        let mut chunk = Vec::new();
        for invalid in [
            malformed_utf8,
            malformed_base64,
            malformed_path_utf8,
            relative_path,
            empty_path,
            control_path,
            prompt_extra,
            history_extra,
            control_frame,
        ] {
            chunk.extend_from_slice(&invalid);
        }
        let valid_start = chunk.len();
        chunk.extend_from_slice(&valid);

        assert_eq!(
            MarkerDecoder::new(session()).scan(&chunk),
            vec![MarkerBoundary {
                end_offset: valid_start + valid.len(),
                kind: MarkerKind::Prompt,
            }]
        );
    }

    #[test]
    fn csi_other_osc_and_plain_text_do_not_produce_markers() {
        let mut decoder = MarkerDecoder::new(session());
        let mut input = b"plain\x1b[31mred\x1b[0m".to_vec();
        input.extend_from_slice(b"\x1b]0;window title\x07");
        input.extend_from_slice(b"\x1b]776;LiteTerm;ignored\x1b\\");

        assert!(decoder.scan(&input).is_empty());
    }

    #[test]
    fn non_st_escape_is_retained_as_frame_data_and_rejected() {
        let mut decoder = MarkerDecoder::new(session());
        let mut invalid = b"\x1b]777;LiteTerm;".to_vec();
        invalid.extend_from_slice(TOKEN.as_bytes());
        invalid.extend_from_slice(format!(";{GENERATION};").as_bytes());
        invalid.extend_from_slice(b"\x1bXP\x07");
        let valid = bel_frame(prompt_body());
        let valid_start = invalid.len();
        invalid.extend_from_slice(&valid);

        assert_eq!(
            decoder.scan(&invalid),
            vec![MarkerBoundary {
                end_offset: valid_start + valid.len(),
                kind: MarkerKind::Prompt,
            }]
        );
    }

    #[test]
    fn oversized_frame_storage_stays_bounded_until_bel_then_recovers() {
        let mut decoder = MarkerDecoder::new(session());
        let mut exact = b"\x1b]".to_vec();
        exact.extend(std::iter::repeat_n(b'x', MAX_OSC_FRAME));
        assert!(decoder.scan(&exact).is_empty());
        assert_eq!(decoder.frame.len(), MAX_OSC_FRAME);
        assert!(!decoder.overflow);

        assert!(decoder.scan(b"x").is_empty());
        assert!(decoder.overflow);
        assert!(decoder.frame.is_empty());

        assert!(decoder.scan(&vec![b'x'; MAX_OSC_FRAME * 4]).is_empty());
        assert!(decoder.overflow);
        assert!(decoder.frame.is_empty());

        let valid = bel_frame(prompt_body());
        let mut reset_and_valid = b"\x07".to_vec();
        reset_and_valid.extend_from_slice(&valid);
        assert_eq!(
            decoder.scan(&reset_and_valid),
            vec![MarkerBoundary {
                end_offset: 1 + valid.len(),
                kind: MarkerKind::Prompt,
            }]
        );
    }

    #[test]
    fn oversized_frame_resets_on_split_st_then_recovers() {
        let mut decoder = MarkerDecoder::new(session());
        let mut oversized = b"\x1b]".to_vec();
        oversized.extend(std::iter::repeat_n(b'x', MAX_OSC_FRAME + 1));
        oversized.push(b'\x1b');
        assert!(decoder.scan(&oversized).is_empty());
        assert!(decoder.overflow);

        assert!(decoder.scan(b"\\").is_empty());
        assert!(!decoder.in_osc);
        assert!(!decoder.overflow);

        let valid = bel_frame(prompt_body());
        assert_eq!(
            decoder.scan(&valid),
            vec![MarkerBoundary {
                end_offset: valid.len(),
                kind: MarkerKind::Prompt,
            }]
        );
    }
}
