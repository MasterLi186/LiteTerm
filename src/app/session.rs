use super::*;
use crate::workspace_session::{PersistedTab, WorkspaceSession};

impl App {
    pub(super) fn restore_workspace_session(&mut self) -> bool {
        let Some(session) = self.pending_workspace_session.take() else {
            return false;
        };
        if session.tabs.is_empty() {
            return false;
        }

        let requested_active = session.active_tab;
        let mut restored_active = None;
        for (saved_index, persisted) in session.tabs.into_iter().enumerate() {
            let restored_index = match persisted {
                PersistedTab::Local { shell_path, label } => {
                    self.restore_local_tab(&shell_path, &label)
                }
                PersistedTab::Ssh {
                    label,
                    host,
                    port,
                    user,
                    auth,
                    key_path,
                } => {
                    let mut connection = sidebar::SshConnection {
                        label,
                        host,
                        port,
                        user,
                        auth,
                        key_path,
                        password: String::new(),
                        group: String::new(),
                        group_color: [0, 0, 0],
                    };
                    if connection.auth == "keyring" || connection.auth == "password" {
                        let entry = crate::keyring::KeyringEntry::new(
                            &connection.user,
                            &connection.host,
                            connection.port,
                        );
                        if let Ok(Some(password)) = entry.retrieve_password() {
                            connection.password = password;
                            connection.auth = "password".into();
                        }
                    }
                    self.new_ssh_tab(&connection);
                    Some(self.tab_manager.active_idx)
                }
                serial @ PersistedTab::Serial { .. } => {
                    let Some(spec) = serial.serial_spec() else {
                        continue;
                    };
                    if spec.validate().is_err() {
                        continue;
                    }
                    self.new_serial_tab(spec);
                    Some(self.tab_manager.active_idx)
                }
            };
            if saved_index == requested_active {
                restored_active = restored_index;
            }
        }

        if self.tab_manager.is_empty() {
            return false;
        }
        self.tab_manager.active_idx = restored_active.unwrap_or(0);
        self.refresh_pane_layout();
        true
    }

    fn restore_local_tab(&mut self, shell: &str, label: &str) -> Option<usize> {
        self.prepare_for_active_tab_change();
        let (cols, rows) = self.grid_size();
        let (tab_id, terminal) = match self.tab_manager.try_new_local(shell, cols, rows) {
            Ok(created) => created,
            Err(error) => {
                log::warn!("恢复本地终端 {shell} 失败：{error}");
                return None;
            }
        };
        let index = self.tab_manager.find_by_id(&tab_id)?;
        let session = self.tab_manager.tabs[index].completion.session().clone();
        let history_path = self.tab_manager.tabs[index]
            .completion
            .history_path()
            .map(std::path::PathBuf::from);
        if !label.trim().is_empty() {
            self.tab_manager.rename(&tab_id, label);
        }
        self.start_read_loop(tab_id.clone(), tab_id.clone(), session.clone(), terminal);
        if let Some(path) = history_path {
            self.request_local_history(tab_id.clone(), tab_id, session, path);
        }
        Some(index)
    }

    pub(super) fn save_workspace_session(&mut self) {
        if self.workspace_session_saved {
            return;
        }
        self.workspace_session_saved = true;
        let session = WorkspaceSession::capture(&self.tab_manager);
        if let Err(error) = session.save() {
            log::warn!("保存标签页会话失败：{error}");
        }
    }

    /// 记录一次正常（非最大化/全屏）状态下的窗口尺寸（逻辑像素）。
    pub(super) fn remember_normal_window_size(&mut self, size: winit::dpi::PhysicalSize<u32>) {
        if size.width == 0 || size.height == 0 {
            return;
        }
        let Some(window) = self.window.as_ref() else {
            return;
        };
        let scale = window.scale_factor();
        if !(scale.is_finite() && scale > 0.0) {
            return;
        }
        let width = size.width as f32 / scale as f32;
        let height = size.height as f32 / scale as f32;
        if width.is_finite() && height.is_finite() && width > 0.0 && height > 0.0 {
            self.window_normal_size = Some((width, height));
        }
    }

    /// 退出时记录窗口尺寸与最大化状态，下次启动按记录恢复。
    pub(super) fn save_window_state(&mut self) {
        if self.window_state_saved {
            return;
        }
        self.window_state_saved = true;
        let Some(window) = self.window.as_ref() else {
            return;
        };
        let maximized = window.is_maximized();
        let fullscreen = window.fullscreen().is_some();
        let inner = window.inner_size();
        let scale = window.scale_factor();
        let current_logical = (inner.width > 0 && inner.height > 0 && scale > 0.0)
            .then(|| {
                (
                    inner.width as f32 / scale as f32,
                    inner.height as f32 / scale as f32,
                )
            })
            .filter(|(width, height)| width.is_finite() && height.is_finite());

        let mut state = self.window_state;
        state.maximized = maximized;
        if !maximized && !fullscreen {
            if let Some((width, height)) = current_logical {
                state.width = width;
                state.height = height;
            }
        } else if let Some((width, height)) = self.window_normal_size {
            // 最大化/全屏时保留最大化前的正常尺寸；若记录值与当前窗口尺寸一致，
            // 说明它被最大化时的 Resized 事件污染过，退回上次保存值。
            let polluted = current_logical.is_some_and(|(current_width, current_height)| {
                (current_width - width).abs() <= 2.0 && (current_height - height).abs() <= 2.0
            });
            if !polluted {
                state.width = width;
                state.height = height;
            }
        }
        state.sanitize();
        if let Err(error) = state.save() {
            log::warn!("保存窗口状态失败：{error}");
        }
        self.window_state = state;
    }
}
