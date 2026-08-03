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
}
