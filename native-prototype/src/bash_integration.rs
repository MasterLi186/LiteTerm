use base64::Engine;
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
#[cfg(unix)]
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use tempfile::TempDir;

use crate::smart_completion::CompletionSessionKey;

pub const MAX_SNAPSHOT_INPUT_BYTES: usize = 8 * 1024;
pub const MAX_OSC_FRAME: usize = 16 * 1024;
static LOCAL_CANDIDATE_TEMP_ID: AtomicU64 = AtomicU64::new(1);

pub(crate) fn validate_candidate_text(candidate: &str) -> Result<(), String> {
    if candidate.is_empty() || candidate.chars().any(char::is_control) {
        return Err("候选命令包含控制字符".into());
    }
    Ok(())
}

pub(crate) fn validate_candidate_bytes(bytes: &[u8]) -> Result<(), String> {
    let candidate = std::str::from_utf8(bytes).map_err(|_| "候选命令不是有效 UTF-8".to_string())?;
    validate_candidate_text(candidate)
}

pub fn is_bash_path(path: &str) -> bool {
    Path::new(path)
        .file_name()
        .is_some_and(|name| name == "bash")
}

pub fn is_safe_remote_bash_path(path: &str) -> bool {
    !path.is_empty()
        && Path::new(path).is_absolute()
        && !path
            .chars()
            .any(|character| character.is_control() || matches!(character, '\'' | '"'))
        && is_bash_path(path)
}

fn sequence_number(session: &CompletionSessionKey) -> u32 {
    session
        .token()
        .get(..8)
        .and_then(|prefix| u32::from_str_radix(prefix, 16).ok())
        .unwrap_or(777)
}

pub fn widget_sequence(session: &CompletionSessionKey) -> String {
    format!("\x1b[777;{}~", sequence_number(session))
}

pub fn snapshot_sequence(session: &CompletionSessionKey) -> String {
    format!("\x1b[778;{}~", sequence_number(session))
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
    fill_sequence: &str,
    snapshot_sequence: &str,
) -> String {
    let candidate = shell_single_quote(&candidate_path.to_string_lossy());
    let fill_key = readline_literal(fill_sequence);
    let snapshot_key = readline_literal(snapshot_sequence);
    let token = session.token();
    let generation = session.generation;
    let max_snapshot_input_bytes = MAX_SNAPSHOT_INPUT_BYTES;

    format!(
        r#"[[ -r "$HOME/.bashrc" ]] && source "$HOME/.bashrc"

if ! declare -p __liteterm_candidate >/dev/null 2>&1; then
    readonly __liteterm_candidate={candidate}
fi

__liteterm_fill() {{
    READLINE_LINE=$(<"$__liteterm_candidate")
    READLINE_POINT=${{#READLINE_LINE}}
}}

__liteterm_snapshot() {{
    if command -v base64 >/dev/null 2>&1 && command -v tr >/dev/null 2>&1; then
        local __liteterm_input_bytes
        local __liteterm_input_point
        local __liteterm_input_prefix
        local __liteterm_input_payload
        __liteterm_input_bytes=$(LC_ALL=C; printf '%s' "${{#READLINE_LINE}}")
        if (( __liteterm_input_bytes > {max_snapshot_input_bytes} )); then
            return
        fi
        __liteterm_input_prefix=${{READLINE_LINE:0:READLINE_POINT}}
        __liteterm_input_point=$(LC_ALL=C; printf '%s' "${{#__liteterm_input_prefix}}")
        __liteterm_input_payload=$(printf '%s' "$READLINE_LINE" | base64 | tr '+/' '-_' | tr -d '=\n')
        printf '\e]777;LiteTerm;{token};{generation};I;%s;%s\a' "$__liteterm_input_point" "$__liteterm_input_payload"
    fi
}}

__liteterm_install_bindings() {{
    builtin bind -m emacs-standard -x '"{fill_key}":__liteterm_fill'
    builtin bind -m emacs-standard -x '"{snapshot_key}":__liteterm_snapshot'
    builtin bind -m vi-insert -x '"{fill_key}":__liteterm_fill'
    builtin bind -m vi-insert -x '"{snapshot_key}":__liteterm_snapshot'
    builtin bind -m vi-command -x '"{fill_key}":__liteterm_fill'
    builtin bind -m vi-command -x '"{snapshot_key}":__liteterm_snapshot'
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

#[derive(Clone, PartialEq, Eq)]
pub struct RemoteBashRuntime {
    pub session: CompletionSessionKey,
    pub bash_path: String,
    pub rc_path: String,
    pub candidate_path: String,
    pub widget_sequence: String,
    pub snapshot_sequence: String,
}

impl std::fmt::Debug for RemoteBashRuntime {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("RemoteBashRuntime")
            .field("session", &self.session)
            .field("bash_path", &self.bash_path)
            .field("rc_path", &"<redacted>")
            .field("candidate_path", &"<redacted>")
            .field("widget_sequence", &"<redacted>")
            .field("snapshot_sequence", &"<redacted>")
            .finish()
    }
}

impl RemoteBashRuntime {
    pub fn snapshot_sequence(&self) -> &str {
        &self.snapshot_sequence
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct RemoteBashPaths {
    pub rc: String,
    pub candidate: String,
}

impl std::fmt::Debug for RemoteBashPaths {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("RemoteBashPaths")
            .field("rc", &"<redacted>")
            .field("candidate", &"<redacted>")
            .finish()
    }
}

impl RemoteBashPaths {
    pub fn new(session: &CompletionSessionKey) -> Self {
        let stem = format!("liteterm-native-{}-{}", session.token(), session.generation);
        Self {
            rc: format!("/tmp/{stem}.rc"),
            candidate: format!("/tmp/{stem}.candidate"),
        }
    }

    pub fn launch_command(&self, bash: &str) -> String {
        format!(
            "umask 077; trap 'rm -f -- \"{}\" \"{}\"' EXIT HUP INT TERM; '{}' --rcfile '{}' -i",
            self.rc, self.candidate, bash, self.rc
        )
    }
}

pub struct LocalBashRuntime {
    temp_dir: TempDir,
    session: CompletionSessionKey,
    rc_path: PathBuf,
    candidate_path: PathBuf,
    widget_sequence: String,
    snapshot_sequence: String,
}

impl std::fmt::Debug for LocalBashRuntime {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("LocalBashRuntime")
            .field("temp_dir", &"<redacted>")
            .field("session", &self.session)
            .field("rc_path", &"<redacted>")
            .field("candidate_path", &"<redacted>")
            .field("widget_sequence", &"<redacted>")
            .field("snapshot_sequence", &"<redacted>")
            .finish()
    }
}

impl LocalBashRuntime {
    pub fn create(session: CompletionSessionKey) -> io::Result<Self> {
        let temp_dir = tempfile::Builder::new()
            .prefix("liteterm-bash-")
            .tempdir()?;
        #[cfg(unix)]
        fs::set_permissions(temp_dir.path(), fs::Permissions::from_mode(0o700))?;

        let rc_path = temp_dir.path().join("bashrc");
        let candidate_path = temp_dir.path().join("candidate");
        let widget_sequence = widget_sequence(&session);
        let snapshot_sequence = snapshot_sequence(&session);
        let rc = build_bash_rc(
            &session,
            &candidate_path,
            &widget_sequence,
            &snapshot_sequence,
        );
        write_new_private_file(&rc_path, rc.as_bytes())?;
        write_new_private_file(&candidate_path, b"")?;

        Ok(Self {
            temp_dir,
            session,
            rc_path,
            candidate_path,
            widget_sequence,
            snapshot_sequence,
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

    pub fn snapshot_sequence(&self) -> &str {
        &self.snapshot_sequence
    }

    pub fn write_candidate(&self, candidate: &str) -> Result<(), String> {
        write_local_candidate_atomic(&self.candidate_path, candidate.as_bytes())
    }
}

pub fn write_local_candidate_atomic(path: &Path, bytes: &[u8]) -> Result<(), String> {
    validate_candidate_bytes(bytes)?;
    let unique = LOCAL_CANDIDATE_TEMP_ID.fetch_add(1, Ordering::Relaxed);
    let temporary = local_candidate_temporary_path(path, unique)?;
    let result = (|| {
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        options.mode(0o600);
        let mut file = options
            .open(&temporary)
            .map_err(|error| error.to_string())?;
        file.write_all(bytes).map_err(|error| error.to_string())?;
        file.sync_all().map_err(|error| error.to_string())?;
        fs::rename(&temporary, path).map_err(|error| error.to_string())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

fn local_candidate_temporary_path(path: &Path, unique: u64) -> Result<PathBuf, String> {
    let parent = path
        .parent()
        .ok_or_else(|| "候选路径缺少父目录".to_string())?;
    let name = path
        .file_name()
        .ok_or_else(|| "候选文件名无效".to_string())?;
    let mut temporary_name = std::ffi::OsString::from(".");
    temporary_name.push(name);
    temporary_name.push(format!(".{}.{unique}.tmp", std::process::id()));
    Ok(parent.join(temporary_name))
}

fn write_new_private_file(path: &Path, contents: &[u8]) -> io::Result<()> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    options.mode(0o600);
    let mut file = options.open(path)?;
    #[cfg(unix)]
    file.set_permissions(fs::Permissions::from_mode(0o600))?;
    file.write_all(contents)?;
    file.sync_all()
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MarkerKind {
    Prompt,
    HistoryPath(String),
    InputSnapshot { line: String, point: usize },
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
        [_, _, _, _, "I", point, payload] => {
            let point = point.parse::<usize>().ok()?;
            let decoded = base64::engine::general_purpose::URL_SAFE_NO_PAD
                .decode(payload)
                .ok()?;
            if decoded.len() > MAX_SNAPSHOT_INPUT_BYTES
                || base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(&decoded) != *payload
            {
                return None;
            }
            let line = std::str::from_utf8(&decoded).ok()?;
            if point > line.len()
                || !line.is_char_boundary(point)
                || line.chars().any(char::is_control)
            {
                return None;
            }
            Some(MarkerKind::InputSnapshot {
                line: line.to_owned(),
                point,
            })
        }
        _ => None,
    }
}

#[cfg(all(test, unix))]
#[path = "bash_integration/tests.rs"]
mod tests;
