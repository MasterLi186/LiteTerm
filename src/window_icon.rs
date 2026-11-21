use winit::window::Icon;

const ICON_PNG: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/assets/icons/256x256.png"
));

/// Decode the embedded application icon for use by winit.
///
/// Icon failures are deliberately reported to the logger and converted to `None` so
/// that a malformed or unsupported asset can never prevent the window from opening.
pub(crate) fn load() -> Option<Icon> {
    let image = match image::load_from_memory(ICON_PNG) {
        Ok(image) => image.to_rgba8(),
        Err(error) => {
            log::warn!("无法解码 LiteTerm Native 窗口图标：{error}");
            return None;
        }
    };

    let (width, height) = image.dimensions();
    match Icon::from_rgba(image.into_raw(), width, height) {
        Ok(icon) => Some(icon),
        Err(error) => {
            log::warn!("无法创建 LiteTerm Native 窗口图标：{error}");
            None
        }
    }
}
