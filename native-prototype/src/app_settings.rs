use super::*;

#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub(super) struct SettingsApplyPlan {
    pub(super) theme_changed: bool,
    pub(super) font_family_changed: bool,
    pub(super) font_size_changed: bool,
    pub(super) zmodem_changed: bool,
}

pub(super) fn plan_settings_apply(
    before: &settings::Settings,
    after: &settings::Settings,
) -> SettingsApplyPlan {
    SettingsApplyPlan {
        theme_changed: before.terminal.color_scheme != after.terminal.color_scheme,
        font_family_changed: before.terminal.font != after.terminal.font,
        font_size_changed: before.terminal.font_size != after.terminal.font_size,
        zmodem_changed: before.zmodem.enabled != after.zmodem.enabled
            || before.zmodem.auto_detect != after.zmodem.auto_detect
            || before.zmodem.download_dir != after.zmodem.download_dir
            || before.zmodem.timeout_secs != after.zmodem.timeout_secs,
    }
}

pub(super) fn zmodem_runtime_settings(
    settings: &settings::Settings,
) -> Result<zmodem::runtime::RuntimeSettings, String> {
    Ok(zmodem::runtime::RuntimeSettings {
        enabled: settings.zmodem.enabled,
        auto_detect: settings.zmodem.auto_detect,
        receive_directory: settings::resolve_zmodem_download_dir(&settings.zmodem.download_dir)?,
        transfer_timeout: Some(Duration::from_secs(settings.zmodem.timeout_secs.into())),
    })
}

pub(super) fn persist_and_publish_zmodem_settings<F>(
    source: &zmodem::runtime::RuntimeSettingsSource,
    settings: &settings::Settings,
    zmodem_changed: bool,
    save: F,
) -> Result<(), String>
where
    F: FnOnce(&settings::Settings) -> std::io::Result<()>,
{
    let runtime_settings = zmodem_changed
        .then(|| zmodem_runtime_settings(settings))
        .transpose()?;
    save(settings).map_err(|error| format!("保存设置失败：{error}"))?;
    if let Some(runtime_settings) = runtime_settings {
        source.update(runtime_settings);
    }
    Ok(())
}
