pub mod commands;
pub mod config;
pub mod core;
pub mod log_util;
pub mod plugin;
pub mod state;

use std::collections::HashMap;
use std::sync::Mutex;

use tauri::Manager;

use config::connections::ConnectionStore;
use config::settings::Settings;
use state::AppState;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // Load persisted configuration
    let settings = Settings::load().unwrap_or_default();
    let connections_path = Settings::config_dir().join("connections.toml");
    let connections =
        ConnectionStore::load_from(&connections_path).unwrap_or_default();

    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_dialog::init())
        .manage(AppState {
            sessions: Mutex::new(HashMap::new()),
            local_terminals: Mutex::new(HashMap::new()),
            connections: Mutex::new(connections),
            settings: Mutex::new(settings),
            sftp_sessions: Mutex::new(HashMap::new()),
            tunnels: Mutex::new(HashMap::new()),
            recordings: Mutex::new(HashMap::new()),
            transfer_cancel: Mutex::new(HashMap::new()),
        })
        .invoke_handler(tauri::generate_handler![
            commands::terminal::open_local_terminal,
            commands::terminal::terminal_write,
            commands::terminal::terminal_resize,
            commands::terminal::close_terminal,
            commands::terminal::list_shells,
            commands::terminal::open_shell_terminal,
            commands::serial::list_serial_ports,
            commands::serial::open_serial_terminal,
            commands::ssh::ssh_connect,
            commands::ssh::ssh_supported_algs,
            commands::connection::load_connections,
            commands::connection::save_connection,
            commands::connection::delete_connection,
            commands::keyring::store_password,
            commands::keyring::retrieve_password,
            commands::monitor::start_monitor,
            commands::monitor::start_local_monitor,
            commands::sftp::list_local_dir,
            commands::sftp::start_sftp_session,
            commands::sftp::sftp_list_dir,
            commands::sftp::sftp_download,
            commands::sftp::sftp_upload,
            commands::sftp::sftp_delete,
            commands::sftp::sftp_rename,
            commands::sftp::save_file,
            commands::sftp::local_delete,
            commands::sftp::local_rename,
            commands::sftp::cancel_transfer,
            commands::sftp::remove_sftp_session,
            commands::sftp::sftp_exec,
            commands::sftp::read_local_file,
            commands::process::get_process_list,
            commands::process::get_process_detail,
            commands::config_io::export_config,
            commands::config_io::import_config,
            commands::config_io::read_text_file,
            commands::ssh_keys::list_ssh_keys,
            commands::ssh_keys::generate_ssh_key,
            commands::ssh_keys::read_ssh_public_key,
            commands::tunnel::create_tunnel,
            commands::tunnel::list_tunnels,
            commands::tunnel::close_tunnel,
            commands::recording::start_recording,
            commands::recording::stop_recording,
            commands::recording::record_event,
            commands::recording::is_recording,
            commands::zmodem::zmodem_send,
        ])
        .setup(|app| {
            // Inject JS to suppress the native webview right-click menu.
            // This runs before any React code and catches the event at the
            // window level, preventing WebKit from showing its default menu.
            let window = app.get_webview_window("main").unwrap();
            window.eval(
                "document.addEventListener('contextmenu',function(e){e.preventDefault();},true);"
            ).ok();
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
