use super::*;

impl Sidebar {
    pub(super) fn export_connections_with_dialog(&mut self) {
        let default_name = format!(
            "connections_backup_{}.toml",
            chrono::Local::now().format("%Y%m%d_%H%M%S")
        );
        let Some(path) = rfd::FileDialog::new()
            .set_title("导出 LiteTerm 连接配置")
            .set_file_name(default_name)
            .add_filter("LiteTerm 配置", &["toml"])
            .save_file()
        else {
            return;
        };
        let result = std::fs::copy(ConnectionStore::config_path(), &path)
            .map(|_| format!("已导出到：{}", path.display()))
            .map_err(|error| format!("导出失败：{error}"));
        show_result("导出配置", result);
    }

    pub(super) fn import_connections_with_dialog(&mut self) {
        let Some(path) = rfd::FileDialog::new()
            .set_title("导入 LiteTerm 连接配置")
            .add_filter("LiteTerm 配置", &["toml"])
            .pick_file()
        else {
            return;
        };
        let result = std::fs::read_to_string(&path)
            .map_err(|error| format!("读取失败：{error}"))
            .and_then(|content| {
                toml::from_str::<ConnectionStore>(&content)
                    .map_err(|error| format!("配置格式无效：{error}"))
            })
            .and_then(|store| {
                store
                    .save()
                    .map_err(|error| format!("保存配置失败：{error}"))
            });
        match result {
            Ok(()) => {
                self.connections = Self::load_connections();
                show_result("导入配置", Ok("导入成功，已重载连接列表".into()));
            }
            Err(error) => show_result("导入配置", Err(error)),
        }
    }
}

fn show_result(title: &str, result: Result<String, String>) {
    let (level, description) = match result {
        Ok(message) => (rfd::MessageLevel::Info, message),
        Err(error) => (rfd::MessageLevel::Error, error),
    };
    let _ = rfd::MessageDialog::new()
        .set_title(title)
        .set_description(description)
        .set_level(level)
        .set_buttons(rfd::MessageButtons::Ok)
        .show();
}
