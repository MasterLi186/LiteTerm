#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

use liteterm_native_api as api;
use std::collections::{HashMap, HashSet};
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant};
use winit::{
    application::ApplicationHandler,
    event::{ElementState, Ime, Modifiers, MouseButton, WindowEvent},
    event_loop::{ActiveEventLoop, EventLoop, EventLoopProxy},
    keyboard::{Key, KeyCode, NamedKey, PhysicalKey},
    window::{CursorIcon, Window, WindowId},
};

mod adb_history;
mod app;
mod app_api_server;
mod app_completion;
mod app_input;
mod app_monitor;
mod app_resources;
mod app_settings;
mod app_zmodem;
mod atlas;
mod bash_integration;
mod batch_command;
mod command_bar;
#[cfg(test)]
mod completion_integration_tests;
mod completion_popup;
mod connections;
mod drag_upload;
mod file_browser;
mod font_support;
mod ime;
mod keyring;
mod monitor;
mod network_detail;
mod new_tab_selector;
mod process_manager;
mod recording;
mod remote_monitor;
mod renderer;
mod serial;
mod settings;
mod settings_panel;
mod sftp;
mod shortcuts;
mod sidebar;
mod smart_completion;
mod split;
mod ssh;
mod ssh_keys;
mod tab_bar;
mod tab_manager;
mod terminal;
mod terminal_links;
mod terminal_log;
mod terminal_search;
mod themes;
mod tunnel;
mod tunnel_manager;
mod workspace_session;
mod zmodem;

#[cfg(test)]
use app::*;
use app_api_server::HttpApiServer;
use app_completion::*;
use app_input::*;
use app_monitor::*;
use app_resources::*;
use app_settings::*;
use app_zmodem::*;
use renderer::{GpuState, PaneRenderRect, Renderer};
use sidebar::Sidebar;
use smart_completion::{AdbHistoryLoadRequest, CompletionSessionKey, HistoryLoadRequest};
use split::{LayoutSnapshot, PaneId, SplitDirection, SplitId};
use tab_manager::{Tab, TabManager, TabType};
use terminal::TerminalState;

enum UserEvent {
    Redraw,
    Api(api::MainThreadCall),
    SshReady {
        tab_id: String,
        pane_id: String,
        session: CompletionSessionKey,
        result: Result<crate::ssh::SshHandle, String>,
    },
    SerialPorts {
        generation: u64,
        result: Result<Vec<serial::SerialPortInfo>, String>,
    },
    SerialReady {
        tab_id: String,
        pane_id: String,
        generation: u64,
        result: Result<serial::SerialHandle, String>,
    },
    CompletionHistory {
        tab_id: String,
        pane_id: String,
        session: CompletionSessionKey,
        request: HistoryLoadRequest,
        path: std::path::PathBuf,
        result: Result<Vec<u8>, String>,
    },
    AdbCompletionHistory {
        tab_id: String,
        pane_id: String,
        session: CompletionSessionKey,
        request: AdbHistoryLoadRequest,
        result: Result<Vec<String>, String>,
    },
    TerminalIntegration {
        tab_id: String,
        pane_id: String,
        session: CompletionSessionKey,
        event: terminal::IntegrationEvent,
    },
    CompletionCandidateWritten {
        tab_id: String,
        pane_id: String,
        session: CompletionSessionKey,
        request_id: u64,
        result: Result<(), String>,
    },
    Zmodem {
        tab_id: String,
        pane_id: String,
        session: CompletionSessionKey,
        event: zmodem::runtime::RuntimeEvent,
    },
    Sftp(sftp::SftpWorkerEvent),
    Monitor(monitor::MonitorEvent),
    ProcessDetail {
        key: monitor::MonitorKey,
        generation: u64,
        requester: String,
        request_id: u64,
        result: Result<Box<monitor::ProcessDetail>, String>,
    },
    NetworkDetail {
        key: monitor::MonitorKey,
        generation: u64,
        requester: String,
        request_id: u64,
        result: Result<Box<network_detail::NetworkDetailSnapshot>, String>,
    },
    RecordingLoaded {
        path: std::path::PathBuf,
        result: Result<recording::Asciicast, String>,
    },
}

impl std::fmt::Debug for UserEvent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            UserEvent::Redraw => write!(f, "Redraw"),
            UserEvent::Api(_) => f.write_str("Api"),
            UserEvent::SshReady { tab_id, result, .. } => {
                let status = if result.is_ok() { "Ok" } else { "Err" };
                write!(f, "SshReady({}, {})", tab_id, status)
            }
            UserEvent::SerialPorts { generation, result } => f
                .debug_struct("SerialPorts")
                .field("generation", generation)
                .field("result", &if result.is_ok() { "Ok" } else { "Err" })
                .finish(),
            UserEvent::SerialReady { tab_id, result, .. } => f
                .debug_struct("SerialReady")
                .field("tab_id", tab_id)
                .field("result", &if result.is_ok() { "Ok" } else { "Err" })
                .finish(),
            UserEvent::CompletionHistory { tab_id, result, .. } => {
                let status = if result.is_ok() { "Ok" } else { "Err" };
                write!(f, "CompletionHistory({}, {})", tab_id, status)
            }
            UserEvent::AdbCompletionHistory { tab_id, result, .. } => {
                let status = if result.is_ok() { "Ok" } else { "Err" };
                write!(f, "AdbCompletionHistory({}, {})", tab_id, status)
            }
            UserEvent::TerminalIntegration { tab_id, event, .. } => {
                let kind = match event {
                    terminal::IntegrationEvent::HistoryPath { .. } => "HistoryPath",
                };
                write!(f, "TerminalIntegration({}, {})", tab_id, kind)
            }
            UserEvent::CompletionCandidateWritten { tab_id, result, .. } => {
                let status = if result.is_ok() { "Ok" } else { "Err" };
                write!(f, "CompletionCandidateWritten({}, {})", tab_id, status)
            }
            UserEvent::Zmodem {
                tab_id,
                pane_id,
                event,
                ..
            } => f
                .debug_struct("Zmodem")
                .field("tab_id", tab_id)
                .field("pane_id", pane_id)
                .field("transfer_id", &event.identity.transfer_id)
                .field("generation", &event.identity.generation)
                .field("kind", &zmodem_runtime_event_kind_name(&event.kind))
                .finish(),
            UserEvent::Sftp(event) => write!(f, "Sftp({event:?})"),
            UserEvent::Monitor(event) => write!(f, "Monitor({event:?})"),
            UserEvent::ProcessDetail {
                key,
                generation,
                requester,
                request_id,
                result,
            } => f
                .debug_struct("ProcessDetail")
                .field("key", key)
                .field("generation", generation)
                .field("requester", requester)
                .field("request_id", request_id)
                .field("result", &if result.is_ok() { "Ok" } else { "Err" })
                .finish(),
            UserEvent::NetworkDetail {
                key,
                generation,
                requester,
                request_id,
                result,
            } => f
                .debug_struct("NetworkDetail")
                .field("key", key)
                .field("generation", generation)
                .field("requester", requester)
                .field("request_id", request_id)
                .field("result", &if result.is_ok() { "Ok" } else { "Err" })
                .finish(),
            UserEvent::RecordingLoaded { path, result } => f
                .debug_struct("RecordingLoaded")
                .field("path", path)
                .field("result", &if result.is_ok() { "Ok" } else { "Err" })
                .finish(),
        }
    }
}

fn crash_log_path() -> std::path::PathBuf {
    dirs::config_dir()
        .unwrap_or_default()
        .join("guishell")
        .join("crash.log")
}

fn setup_crash_handler() {
    // Rust panic handler → 写入 crash.log
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let path = crash_log_path();
        let _ = std::fs::create_dir_all(path.parent().unwrap());
        let timestamp = chrono_lite();
        let mut msg = format!("[{}] PANIC: {}\n", timestamp, info);
        // 尝试获取 backtrace
        msg.push_str(&format!(
            "Backtrace:\n{:?}\n",
            std::backtrace::Backtrace::force_capture()
        ));
        let _ = std::fs::write(&path, &msg);
        eprintln!("=== CRASH ===\n{}\n写入: {:?}", msg, path);
        default_hook(info);
    }));

    // Unix signal handler → SIGSEGV/SIGBUS/SIGABRT
    #[cfg(unix)]
    unsafe {
        use std::os::raw::c_int;
        extern "C" fn signal_handler(sig: c_int) {
            let path = crash_log_path();
            let _ = std::fs::create_dir_all(path.parent().unwrap());
            let sig_name = match sig {
                11 => "SIGSEGV (段错误)",
                7 => "SIGBUS (总线错误)",
                6 => "SIGABRT (异常终止)",
                _ => "未知信号",
            };
            let msg = format!(
                "[crash] Signal {} ({})\nBacktrace:\n{:?}\n",
                sig,
                sig_name,
                std::backtrace::Backtrace::force_capture()
            );
            let _ = std::fs::write(&path, &msg);
            std::process::exit(128 + sig);
        }
        libc::signal(libc::SIGSEGV, signal_handler as *const () as usize);
        libc::signal(libc::SIGBUS, signal_handler as *const () as usize);
        libc::signal(libc::SIGABRT, signal_handler as *const () as usize);
    }
}

fn chrono_lite() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let hours = (secs % 86400) / 3600 + 8; // UTC+8 简化
    let mins = (secs % 3600) / 60;
    let s = secs % 60;
    format!("{:02}:{:02}:{:02}", hours % 24, mins, s)
}

fn main() {
    setup_crash_handler();

    // 启动时检查上次 crash
    let crash_path = crash_log_path();
    if crash_path.exists() {
        if let Ok(content) = std::fs::read_to_string(&crash_path) {
            eprintln!("=== 上次闪退记录 ===\n{}\n===================", content);
        }
        // 重命名为 .old 保留历史
        let old = crash_path.with_extension("log.old");
        let _ = std::fs::rename(&crash_path, &old);
    }

    env_logger::init();
    let event_loop = EventLoop::<UserEvent>::with_user_event().build().unwrap();
    let proxy = event_loop.create_proxy();
    let mut app = app::App::new(proxy);
    event_loop.run_app(&mut app).unwrap();
}

#[cfg(test)]
#[path = "main_tests/api_main_tests.rs"]
mod api_main_tests;

#[cfg(test)]
#[path = "main_tests/layout_tests.rs"]
mod layout_tests;

#[cfg(test)]
#[path = "main_tests/zmodem_main_tests.rs"]
mod zmodem_main_tests;
