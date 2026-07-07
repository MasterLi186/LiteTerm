use std::io::Write;
use std::sync::Mutex;

static LOG_LOCK: Mutex<()> = Mutex::new(());

/// Append a log line to `~/guishell.log`.
///
/// Format: `[epoch.ms] [CATEGORY] message`
///
/// 全局 Mutex 保证多线程写入不交错。
pub fn app_log(category: &str, message: &str) {
    let _guard = LOG_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let log_path = dirs::home_dir()
        .unwrap_or_default()
        .join("guishell.log");
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
    {
        let d = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default();
        let _ = writeln!(f, "[{}.{:03}] [{}] {}", d.as_secs(), d.subsec_millis(), category, message);
    }
}

/// Convenience macro — works like `app_log!("SSH", "连接 {}:{} 失败: {}", host, port, e)`.
#[macro_export]
macro_rules! app_log {
    ($cat:expr, $($arg:tt)*) => {
        $crate::log_util::app_log($cat, &format!($($arg)*))
    };
}
