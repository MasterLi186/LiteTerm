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

struct BashPtyGuard {
    child: Option<Box<dyn portable_pty::Child + Send + Sync>>,
    writer: Option<Box<dyn std::io::Write + Send>>,
    master: Option<Box<dyn portable_pty::MasterPty + Send>>,
    reader_thread: Option<std::thread::JoinHandle<()>>,
    reader_done: Option<std::sync::mpsc::Receiver<()>>,
}

impl BashPtyGuard {
    fn new(child: Box<dyn portable_pty::Child + Send + Sync>) -> Self {
        Self {
            child: Some(child),
            writer: None,
            master: None,
            reader_thread: None,
            reader_done: None,
        }
    }

    fn attach_reader_and_writer(
        &mut self,
        writer: Box<dyn std::io::Write + Send>,
        master: Box<dyn portable_pty::MasterPty + Send>,
        reader_thread: std::thread::JoinHandle<()>,
        reader_done: std::sync::mpsc::Receiver<()>,
    ) {
        self.writer = Some(writer);
        self.master = Some(master);
        self.reader_thread = Some(reader_thread);
        self.reader_done = Some(reader_done);
    }

    fn child_exited(
        child: &mut dyn portable_pty::Child,
        timeout: std::time::Duration,
    ) -> Result<bool, String> {
        let deadline = std::time::Instant::now() + timeout;
        loop {
            match child.try_wait() {
                Ok(Some(_)) => return Ok(true),
                Ok(None) if std::time::Instant::now() < deadline => {
                    std::thread::sleep(std::time::Duration::from_millis(10));
                }
                Ok(None) => return Ok(false),
                Err(error) => return Err(format!("等待 Bash 子进程失败: {error}")),
            }
        }
    }

    fn cleanup(&mut self, graceful_timeout: std::time::Duration) -> Result<(), String> {
        self.writer = None;
        self.master = None;
        let mut errors = Vec::new();
        if let Some(mut child) = self.child.take() {
            match Self::child_exited(child.as_mut(), graceful_timeout) {
                Ok(true) => {}
                Ok(false) | Err(_) => {
                    if let Err(error) = child.kill() {
                        errors.push(format!("终止 Bash 子进程失败: {error}"));
                    }
                    match Self::child_exited(child.as_mut(), std::time::Duration::from_secs(1)) {
                        Ok(true) => {}
                        Ok(false) => errors.push("Bash 子进程未在期限内退出".into()),
                        Err(error) => errors.push(error),
                    }
                }
            }
        }

        let reader_finished = match self.reader_done.take() {
            Some(done) => done.recv_timeout(std::time::Duration::from_secs(1)).is_ok(),
            None => true,
        };
        if reader_finished {
            if let Some(reader_thread) = self.reader_thread.take() {
                if reader_thread.join().is_err() {
                    errors.push("Bash PTY reader 线程 panic".into());
                }
            }
        } else {
            self.reader_thread = None;
            errors.push("Bash PTY reader 未在期限内退出".into());
        }

        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors.join("; "))
        }
    }

    fn finish(&mut self) -> Result<(), String> {
        let write_result = self
            .writer
            .as_mut()
            .ok_or_else(|| "Bash PTY writer 未初始化".to_string())
            .and_then(|writer| {
                writer
                    .write_all(b"exit\r")
                    .and_then(|_| writer.flush())
                    .map_err(|error| format!("发送 Bash exit 失败: {error}"))
            });
        let graceful_timeout = if write_result.is_ok() {
            std::time::Duration::from_secs(2)
        } else {
            std::time::Duration::ZERO
        };
        let cleanup_result = self.cleanup(graceful_timeout);
        write_result.and(cleanup_result)
    }
}

impl Drop for BashPtyGuard {
    fn drop(&mut self) {
        let _ = self.cleanup(std::time::Duration::ZERO);
    }
}
include!("tests/part_01.rs");
include!("tests/part_02.rs");
include!("tests/part_03.rs");
