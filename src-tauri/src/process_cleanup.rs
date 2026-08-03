//! 进程树清理 — 解决 Linux 上 guishell 退出后 WebKitWebProcess 孤儿占用数 GB 内存的问题。
//!
//! 现象：主进程卡死 → 任务栏右键退出只干掉 guishell-tauri → WebKit 子进程被 init 收养并继续占内存。
//! 对策：退出/SIGTERM/force_quit 时递归 SIGKILL 所有后代进程。

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::atomic::{AtomicBool, Ordering};

static SIGNAL_EXIT: AtomicBool = AtomicBool::new(false);
static CLEANUP_DONE: AtomicBool = AtomicBool::new(false);

/// 安装 SIGTERM/SIGINT 处理：置位后由 watcher 线程执行清理并退出。
/// 必须在启动早期调用一次。
pub fn install_signal_handlers() {
    #[cfg(unix)]
    {
        // 用标准 signal 安装：handler 内只做 atomic store（async-signal-safe）
        unsafe {
            libc::signal(libc::SIGTERM, signal_handler as *const () as usize);
            libc::signal(libc::SIGINT, signal_handler as *const () as usize);
        }

        // watcher：信号到达后杀进程树并退出
        std::thread::Builder::new()
            .name("exit-cleanup".into())
            .spawn(|| {
                loop {
                    if SIGNAL_EXIT.load(Ordering::SeqCst) {
                        shutdown_cleanup("signal");
                        // 确保退出；若 cleanup 里已经 exit 则不会走到这里
                        std::process::exit(1);
                    }
                    std::thread::sleep(std::time::Duration::from_millis(50));
                }
            })
            .ok();
    }
}

#[cfg(unix)]
extern "C" fn signal_handler(_sig: i32) {
    SIGNAL_EXIT.store(true, Ordering::SeqCst);
}

/// 退出时调用：递归杀死本进程所有后代（含 WebKitWebProcess / NetworkProcess）。
/// 可重入；多次调用安全。
pub fn shutdown_cleanup(reason: &str) {
    if CLEANUP_DONE.swap(true, Ordering::SeqCst) {
        return;
    }
    crate::log_util::app_log(
        "关闭",
        &format!("shutdown_cleanup({reason}): 开始清理子进程树"),
    );

    let children = collect_descendants(std::process::id());
    crate::log_util::app_log(
        "关闭",
        &format!(
            "shutdown_cleanup: 发现 {} 个后代进程: {:?}",
            children.len(),
            children
        ),
    );

    #[cfg(unix)]
    {
        // 先 SIGTERM 给一次体面退出的机会，再 SIGKILL
        for &pid in &children {
            unsafe {
                libc::kill(pid as i32, libc::SIGTERM);
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(150));
        for &pid in &children {
            unsafe {
                libc::kill(pid as i32, libc::SIGKILL);
            }
        }
    }

    crate::log_util::app_log("关闭", "shutdown_cleanup: 完成");
}

/// 读取 /proc 构建 pid→ppid 映射，BFS 收集 `root` 的全部后代（不含 root 自身）。
fn collect_descendants(root: u32) -> Vec<u32> {
    let mut ppid_of: HashMap<u32, u32> = HashMap::new();
    let Ok(entries) = std::fs::read_dir("/proc") else {
        return Vec::new();
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        let Ok(pid) = name.parse::<u32>() else {
            continue;
        };
        let status_path = entry.path().join("status");
        let Ok(content) = std::fs::read_to_string(status_path) else {
            continue;
        };
        for line in content.lines() {
            if let Some(rest) = line.strip_prefix("PPid:") {
                if let Ok(ppid) = rest.trim().parse::<u32>() {
                    ppid_of.insert(pid, ppid);
                }
                break;
            }
        }
    }

    // 反向：parent → children
    let mut children_of: HashMap<u32, Vec<u32>> = HashMap::new();
    for (&pid, &ppid) in &ppid_of {
        children_of.entry(ppid).or_default().push(pid);
    }

    let mut out = Vec::new();
    let mut seen = HashSet::new();
    let mut q = VecDeque::new();
    q.push_back(root);
    seen.insert(root);
    while let Some(p) = q.pop_front() {
        if let Some(kids) = children_of.get(&p) {
            for &c in kids {
                if seen.insert(c) {
                    out.push(c);
                    q.push_back(c);
                }
            }
        }
    }
    // 先杀叶子：按发现的逆序（深的在后）反转后从深到浅
    out.reverse();
    out
}
