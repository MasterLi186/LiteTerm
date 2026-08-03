use super::*;

impl Sidebar {
    pub fn render_dialogs(&mut self, ctx: &egui::Context) {
        self.dialog_new_connection(ctx);
        self.dialog_key_manager(ctx);
    }

    // ── 新建连接 ──
    fn dialog_new_connection(&mut self, ctx: &egui::Context) {
        if !self.show_new_connection {
            return;
        }
        let mut open = true;
        let mut save_and_connect = false;
        let mut save_only = false;

        egui::Window::new("新建 SSH 连接")
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .resizable(false)
            .default_width(380.0)
            .show(ctx, |ui| {
                ui.add_space(4.0);

                egui::Grid::new("new_conn_grid")
                    .num_columns(2)
                    .spacing([8.0, 6.0])
                    .show(ui, |ui| {
                        ui.label("标签:");
                        ui.add(
                            egui::TextEdit::singleline(&mut self.new_conn.label)
                                .desired_width(260.0),
                        );
                        ui.end_row();

                        ui.label("主机:");
                        ui.add(
                            egui::TextEdit::singleline(&mut self.new_conn.host)
                                .desired_width(260.0)
                                .hint_text("192.168.1.1"),
                        );
                        ui.end_row();

                        ui.label("端口:");
                        ui.add(
                            egui::TextEdit::singleline(&mut self.new_conn.port).desired_width(80.0),
                        );
                        ui.end_row();

                        ui.label("用户名:");
                        ui.add(
                            egui::TextEdit::singleline(&mut self.new_conn.user)
                                .desired_width(260.0),
                        );
                        ui.end_row();

                        ui.label("认证:");
                        ui.horizontal(|ui| {
                            ui.selectable_value(&mut self.new_conn.auth_idx, 0, "密钥");
                            ui.selectable_value(&mut self.new_conn.auth_idx, 1, "密码");
                        });
                        ui.end_row();

                        if self.new_conn.auth_idx == 0 {
                            ui.label("密钥路径:");
                            ui.horizontal(|ui| {
                                ui.add(
                                    egui::TextEdit::singleline(&mut self.new_conn.key_path)
                                        .desired_width(190.0),
                                );
                                if ui.button("浏览…").clicked() {
                                    let mut dialog =
                                        rfd::FileDialog::new().set_title("选择 SSH 私钥文件");
                                    if let Some(ssh_dir) =
                                        dirs::home_dir().map(|home| home.join(".ssh"))
                                    {
                                        dialog = dialog.set_directory(ssh_dir);
                                    }
                                    if let Some(path) = dialog.pick_file() {
                                        self.new_conn.key_path =
                                            path.to_string_lossy().into_owned();
                                    }
                                }
                            });
                            ui.end_row();
                        } else {
                            ui.label("密码:");
                            ui.add(
                                egui::TextEdit::singleline(&mut self.new_conn.password)
                                    .password(true)
                                    .desired_width(260.0),
                            );
                            ui.end_row();
                        }

                        ui.label("分组:");
                        let store = ConnectionStore::load();
                        let groups: Vec<String> = store.groups.keys().cloned().collect();
                        egui::ComboBox::from_id_salt("group_combo")
                            .selected_text(&self.new_conn.group)
                            .show_ui(ui, |ui| {
                                for g in &groups {
                                    ui.selectable_value(&mut self.new_conn.group, g.clone(), g);
                                }
                                ui.selectable_value(
                                    &mut self.new_conn.group,
                                    "__new__".to_string(),
                                    "+ 新建分组",
                                );
                            });
                        ui.end_row();

                        if self.new_conn.group == "__new__" {
                            ui.label("新分组名:");
                            ui.add(
                                egui::TextEdit::singleline(&mut self.new_conn.new_group)
                                    .desired_width(260.0),
                            );
                            ui.end_row();
                        }
                    });

                if !self.new_conn.status.is_empty() {
                    ui.add_space(4.0);
                    ui.colored_label(
                        egui::Color32::from_rgb(0xf8, 0x51, 0x49),
                        &self.new_conn.status,
                    );
                }

                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button("保存并连接").clicked() {
                        save_and_connect = true;
                    }
                    if ui.button("仅保存").clicked() {
                        save_only = true;
                    }
                    if ui.button("取消").clicked() {
                        open = false;
                    }
                });
            });

        if save_and_connect || save_only {
            if let Err(e) = self.do_save_connection() {
                self.new_conn.status = e;
            } else {
                self.reload();
                if save_and_connect {
                    // Trigger connection
                    let port = self.new_conn.port.parse().unwrap_or(22);
                    let auth = if self.new_conn.auth_idx == 0 {
                        "key"
                    } else {
                        "password"
                    };
                    self.on_connect = Some(SshConnection {
                        label: self.new_conn.label.clone(),
                        host: self.new_conn.host.clone(),
                        port,
                        user: self.new_conn.user.clone(),
                        auth: auth.to_string(),
                        key_path: self.new_conn.key_path.clone(),
                        password: self.new_conn.password.clone(),
                        group: self.new_conn.group.clone(),
                        group_color: [0x58, 0xa6, 0xff],
                    });
                }
                open = false;
            }
        }
        if !open {
            self.show_new_connection = false;
        }
    }

    fn do_save_connection(&self) -> Result<(), String> {
        let f = &self.new_conn;
        if f.host.is_empty() {
            return Err("主机不能为空".into());
        }
        let port: u16 = f.port.parse().map_err(|_| "端口格式无效")?;
        let label = if f.label.is_empty() {
            f.host.clone()
        } else {
            f.label.clone()
        };
        let group_id = if f.group == "__new__" {
            if f.new_group.is_empty() {
                return Err("分组名不能为空".into());
            }
            f.new_group.clone()
        } else {
            f.group.clone()
        };

        let auth = if f.auth_idx == 0 {
            AuthMethod::Key
        } else {
            AuthMethod::Password
        };
        let host_id = format!("{}:{}", f.host, port);

        let mut store = ConnectionStore::load();
        if !store.groups.contains_key(&group_id) {
            store.add_group(&group_id, &group_id, "#58a6ff");
        }
        store.add_host(
            &group_id,
            &host_id,
            HostConfig {
                label,
                host: f.host.clone(),
                port,
                user: f.user.clone(),
                auth,
                key_path: f.key_path.clone(),
                charset: "UTF-8".to_string(),
                proxy_jump: String::new(),
            },
        );
        store.save()
    }

    // ── SSH 密钥管理 ──
    fn dialog_key_manager(&mut self, ctx: &egui::Context) {
        if !self.show_key_manager {
            return;
        }

        // Load keys on first show
        if !self.ssh_keys_loaded {
            self.ssh_keys = list_ssh_keys();
            self.ssh_keys_loaded = true;
            self.keygen_status.clear();
        }

        let mut open = true;
        egui::Window::new("SSH 密钥管理")
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .resizable(true)
            .default_width(500.0)
            .default_height(400.0)
            .show(ctx, |ui| {
                // Key list
                ui.heading("已有密钥");
                egui::ScrollArea::vertical()
                    .max_height(250.0)
                    .show(ui, |ui| {
                        for key in &self.ssh_keys {
                            ui.horizontal(|ui| {
                                let icon = if key.is_public { "🔓" } else { "🔑" };
                                ui.label(egui::RichText::new(icon).size(12.0));
                                ui.label(
                                    egui::RichText::new(&key.name)
                                        .size(11.0)
                                        .color(egui::Color32::from_rgb(0xc9, 0xd1, 0xd9)),
                                );
                                ui.label(
                                    egui::RichText::new(&key.key_type)
                                        .size(10.0)
                                        .color(egui::Color32::from_rgb(0x8b, 0x94, 0x9e)),
                                );
                                if !key.fingerprint.is_empty() {
                                    ui.label(
                                        egui::RichText::new(&key.fingerprint)
                                            .size(9.0)
                                            .color(egui::Color32::from_rgb(0x48, 0x4f, 0x58)),
                                    );
                                }
                                if key.is_public {
                                    if ui.small_button("复制").clicked() {
                                        if let Ok(content) = std::fs::read_to_string(&key.path) {
                                            if let Ok(mut cb) = arboard::Clipboard::new() {
                                                let _ = cb.set_text(&content);
                                            }
                                        }
                                    }
                                }
                            });
                        }
                    });

                ui.separator();
                ui.heading("生成新密钥");
                ui.horizontal(|ui| {
                    ui.label("类型:");
                    egui::ComboBox::from_id_salt("keygen_type")
                        .selected_text(&self.keygen_type)
                        .show_ui(ui, |ui| {
                            ui.selectable_value(
                                &mut self.keygen_type,
                                "ed25519".to_string(),
                                "ed25519 (推荐)",
                            );
                            ui.selectable_value(&mut self.keygen_type, "rsa".to_string(), "rsa");
                            ui.selectable_value(
                                &mut self.keygen_type,
                                "ecdsa".to_string(),
                                "ecdsa",
                            );
                        });
                    ui.label("备注:");
                    ui.add(
                        egui::TextEdit::singleline(&mut self.keygen_comment)
                            .desired_width(150.0)
                            .hint_text("user@host"),
                    );
                });
                ui.horizontal(|ui| {
                    if ui.button("生成密钥").clicked() {
                        match generate_ssh_key(&self.keygen_type, &self.keygen_comment) {
                            Ok(pub_key) => {
                                self.keygen_status =
                                    format!("✓ 已生成 id_{}\n{}", self.keygen_type, pub_key.trim());
                                self.ssh_keys = list_ssh_keys(); // reload
                            }
                            Err(e) => {
                                self.keygen_status = format!("✗ {}", e);
                            }
                        }
                    }
                });
                if !self.keygen_status.is_empty() {
                    ui.label(egui::RichText::new(&self.keygen_status).size(10.0));
                }
                ui.add_space(8.0);
                if ui.button("关闭").clicked() {
                    open = false;
                }
            });
        if !open {
            self.show_key_manager = false;
        }
    }

    // 导入/导出使用系统原生文件对话框，见 file_dialogs.rs。
}

fn list_ssh_keys() -> Vec<SshKeyInfo> {
    let ssh_dir = match dirs::home_dir() {
        Some(h) => h.join(".ssh"),
        None => return Vec::new(),
    };
    if !ssh_dir.exists() {
        return Vec::new();
    }

    let mut keys = Vec::new();
    let entries = match std::fs::read_dir(&ssh_dir) {
        Ok(e) => e,
        Err(_) => return Vec::new(),
    };

    for entry in entries.flatten() {
        let path = entry.path();
        let name = path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();
        let is_public = name.ends_with(".pub");
        let is_private = name.starts_with("id_") && !is_public;
        if !is_public && !is_private {
            continue;
        }

        let key_type = if name.contains("ed25519") {
            "ed25519"
        } else if name.contains("ecdsa") {
            "ecdsa"
        } else if name.contains("rsa") {
            "rsa"
        } else if name.contains("dsa") {
            "dsa"
        } else {
            "unknown"
        }
        .to_string();

        let fingerprint = if is_public {
            get_fingerprint(&path)
        } else {
            let pub_path = path.with_extension("pub");
            if pub_path.exists() {
                get_fingerprint(&pub_path)
            } else {
                String::new()
            }
        };

        keys.push(SshKeyInfo {
            name,
            path: path.to_string_lossy().to_string(),
            key_type,
            is_public,
            fingerprint,
        });
    }
    keys.sort_by(|a, b| a.name.cmp(&b.name));
    keys
}

fn get_fingerprint(pub_key_path: &std::path::Path) -> String {
    let output = match std::process::Command::new("ssh-keygen")
        .args(["-lf", &pub_key_path.to_string_lossy()])
        .output()
    {
        Ok(o) => o,
        Err(_) => return String::new(),
    };
    if output.status.success() {
        let line = String::from_utf8_lossy(&output.stdout);
        let parts: Vec<&str> = line.trim().splitn(3, ' ').collect();
        if parts.len() >= 2 {
            return parts[1].to_string();
        }
    }
    String::new()
}

fn generate_ssh_key(key_type: &str, comment: &str) -> Result<String, String> {
    let ssh_dir = dirs::home_dir().ok_or("无法获取用户目录")?.join(".ssh");
    std::fs::create_dir_all(&ssh_dir).map_err(|e| format!("创建 .ssh 失败: {}", e))?;

    let key_name = format!("id_{}", key_type);
    let key_path = ssh_dir.join(&key_name);
    if key_path.exists() {
        return Err(format!("密钥 {} 已存在", key_name));
    }

    let comment = if comment.is_empty() {
        "generated-by-liteterm"
    } else {
        comment
    };
    let output = std::process::Command::new("ssh-keygen")
        .args([
            "-t",
            key_type,
            "-C",
            comment,
            "-f",
            &key_path.to_string_lossy(),
            "-N",
            "",
        ])
        .output()
        .map_err(|e| format!("执行 ssh-keygen 失败: {}", e))?;

    if !output.status.success() {
        return Err(format!(
            "ssh-keygen 失败: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    std::fs::read_to_string(key_path.with_extension("pub"))
        .map_err(|e| format!("读取公钥失败: {}", e))
}
