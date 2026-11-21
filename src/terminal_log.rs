use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

#[derive(Clone, Default)]
pub struct TerminalLogRegistry {
    inner: Arc<Mutex<HashMap<String, ActiveLog>>>,
}

struct ActiveLog {
    path: PathBuf,
    writer: BufWriter<File>,
    ansi: AnsiStripper,
}

impl TerminalLogRegistry {
    pub fn start(&self, pane_id: &str, path: &Path) -> Result<(), String> {
        if pane_id.is_empty() {
            return Err("日志目标无效".into());
        }
        let mut logs = self
            .inner
            .lock()
            .map_err(|_| "日志状态锁已损坏".to_string())?;
        if logs.contains_key(pane_id) {
            return Err("当前终端已在录制日志".into());
        }
        let file = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(path)
            .map_err(|error| format!("无法创建日志文件：{error}"))?;
        logs.insert(
            pane_id.to_string(),
            ActiveLog {
                path: path.to_path_buf(),
                writer: BufWriter::new(file),
                ansi: AnsiStripper::default(),
            },
        );
        Ok(())
    }

    pub fn record_output(&self, pane_id: &str, bytes: &[u8]) {
        if bytes.is_empty() {
            return;
        }
        let Ok(mut logs) = self.inner.lock() else {
            return;
        };
        let Some(log) = logs.get_mut(pane_id) else {
            return;
        };
        let clean = log.ansi.strip(bytes);
        if !clean.is_empty() {
            let _ = log.writer.write_all(&clean);
            if clean.contains(&b'\n') {
                let _ = log.writer.flush();
            }
        }
    }

    pub fn stop(&self, pane_id: &str) -> Result<PathBuf, String> {
        let mut log = self
            .inner
            .lock()
            .map_err(|_| "日志状态锁已损坏".to_string())?
            .remove(pane_id)
            .ok_or_else(|| "当前终端没有进行中的日志录制".to_string())?;
        log.writer
            .flush()
            .map_err(|error| format!("保存日志失败：{error}"))?;
        Ok(log.path)
    }

    pub fn is_logging(&self, pane_id: &str) -> bool {
        self.inner
            .lock()
            .is_ok_and(|logs| logs.contains_key(pane_id))
    }

    pub fn stop_all(&self) {
        if let Ok(mut logs) = self.inner.lock() {
            for (_, mut log) in logs.drain() {
                let _ = log.writer.flush();
            }
        }
    }
}

#[derive(Clone, Copy, Default)]
enum EscapeState {
    #[default]
    Ground,
    Escape,
    Csi,
    Osc,
    OscEscape,
    String,
    StringEscape,
}

#[derive(Default)]
struct AnsiStripper {
    state: EscapeState,
}

impl AnsiStripper {
    fn strip(&mut self, input: &[u8]) -> Vec<u8> {
        let mut output = Vec::with_capacity(input.len());
        for &byte in input {
            self.state = match self.state {
                EscapeState::Ground if byte == 0x1b => EscapeState::Escape,
                EscapeState::Ground => {
                    output.push(byte);
                    EscapeState::Ground
                }
                EscapeState::Escape => match byte {
                    b'[' => EscapeState::Csi,
                    b']' => EscapeState::Osc,
                    b'P' | b'X' | b'^' | b'_' => EscapeState::String,
                    0x1b => EscapeState::Escape,
                    _ => EscapeState::Ground,
                },
                EscapeState::Csi if (0x40..=0x7e).contains(&byte) => EscapeState::Ground,
                EscapeState::Csi => EscapeState::Csi,
                EscapeState::Osc if byte == 0x07 => EscapeState::Ground,
                EscapeState::Osc if byte == 0x1b => EscapeState::OscEscape,
                EscapeState::Osc => EscapeState::Osc,
                EscapeState::OscEscape if byte == b'\\' => EscapeState::Ground,
                EscapeState::OscEscape if byte == 0x1b => EscapeState::OscEscape,
                EscapeState::OscEscape => EscapeState::Osc,
                EscapeState::String if byte == 0x1b => EscapeState::StringEscape,
                EscapeState::String => EscapeState::String,
                EscapeState::StringEscape if byte == b'\\' => EscapeState::Ground,
                EscapeState::StringEscape if byte == 0x1b => EscapeState::StringEscape,
                EscapeState::StringEscape => EscapeState::String,
            };
        }
        output
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_fragmented_csi_and_osc_without_damaging_utf8() {
        let mut stripper = AnsiStripper::default();
        let mut output = stripper.strip(b"\x1b[31");
        output.extend(stripper.strip("m中文\x1b]0;title".as_bytes()));
        output.extend(stripper.strip(b"\x07 ok\x1b[0m\n"));
        assert_eq!(String::from_utf8(output).unwrap(), "中文 ok\n");
    }

    #[test]
    fn registry_streams_plain_text_and_flushes_on_stop() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("terminal.txt");
        let registry = TerminalLogRegistry::default();
        registry.start("pane", &path).unwrap();
        registry.record_output("pane", b"\x1b[32mhello\x1b[0m\n");
        assert_eq!(registry.stop("pane").unwrap(), path);
        assert_eq!(std::fs::read_to_string(path).unwrap(), "hello\n");
    }
}
