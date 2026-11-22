//! 快捷命令的发送目标。Windows 本地终端没有 free、top、ss 等 Linux 命令，
//! PowerShell / cmd 也只在收到回车（`\r`）时执行，这里按当前标签的终端类型适配。

use std::borrow::Cow;

/// 命令栏当前发送命令的目标终端
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CommandTarget {
    /// SSH、串口，以及 Linux/macOS 本地终端：内置命令原样发送，以 `\n` 结尾
    #[default]
    Unix,
    /// Windows 本地 Git Bash 等 MSYS 环境的 bash：有 df、ls，没有 free、top、ss
    GitBash,
    /// Windows 本地 PowerShell（Windows PowerShell 或 PowerShell 7）
    PowerShell,
    /// Windows 本地 cmd
    Cmd,
}

impl CommandTarget {
    /// 按本地终端的 shell 路径判断；非 Windows 平台一律按 Unix 处理。
    pub fn for_local_shell(shell_path: &str) -> Self {
        if !cfg!(windows) {
            return Self::Unix;
        }
        let stem = std::path::Path::new(shell_path)
            .file_stem()
            .map(|name| name.to_string_lossy().to_ascii_lowercase())
            .unwrap_or_default();
        match stem.as_str() {
            "bash" | "sh" | "zsh" => Self::GitBash,
            "pwsh" | "powershell" => Self::PowerShell,
            "cmd" => Self::Cmd,
            // wsl.exe 等进入的是 Linux 环境，原样发送
            _ => Self::Unix,
        }
    }

    /// 发送命令时追加的回车。PowerShell / cmd 收到 `\n` 只换行不执行，
    /// Windows 本地终端统一用 `\r`（Git Bash 两者都认）。
    pub fn line_ending(self) -> &'static str {
        match self {
            Self::Unix => "\n",
            Self::GitBash | Self::PowerShell | Self::Cmd => "\r",
        }
    }

    /// 内置快捷命令在该目标下实际发送的内容；`None` 表示该环境不支持，按钮不显示。
    /// 未列出的命令原样发送（Git Bash 自带 df、ls）。
    pub fn adapt_builtin(self, command: &str) -> Option<Cow<'_, str>> {
        use CommandTarget::{Cmd, GitBash, PowerShell, Unix};
        let adapted = match (self, command) {
            (Unix, _) => return Some(Cow::Borrowed(command)),
            (_, "docker ps") => return docker_available().then_some(Cow::Borrowed(command)),
            // Linux 上 showmount -e 查看本机 NFS 服务器的共享，Windows 一般不提供 NFS 服务
            (_, "showmount -e") => return None,
            (GitBash, "free -h") => "grep -E 'MemTotal|MemFree|Swap' /proc/meminfo",
            (GitBash, "top") => "tasklist",
            (GitBash, "ss -tlnp") => "netstat -ano | grep LISTENING",
            (PowerShell, "df -h") => "Get-PSDrive -PSProvider FileSystem",
            (PowerShell, "free -h") => {
                "Get-CimInstance Win32_OperatingSystem | Select-Object \
                 @{n='TotalGB';e={[math]::Round($_.TotalVisibleMemorySize/1MB,1)}}, \
                 @{n='FreeGB';e={[math]::Round($_.FreePhysicalMemory/1MB,1)}}"
            }
            (PowerShell, "top") => {
                "Get-Process | Sort-Object CPU -Descending | Select-Object -First 20"
            }
            (PowerShell, "ss -tlnp") => {
                "Get-NetTCPConnection -State Listen | Sort-Object LocalPort"
            }
            (PowerShell, "ls -la") => "Get-ChildItem -Force",
            // cmd 没有不需要管理员权限的磁盘用量命令（wmic 已被新版 Windows 移除），借用 PowerShell
            (Cmd, "df -h") => {
                "powershell -NoProfile -Command \"Get-PSDrive -PSProvider FileSystem\""
            }
            (Cmd, "free -h") => "systeminfo | findstr /C:\"物理内存\" /C:\"Physical Memory\"",
            (Cmd, "top") => "tasklist",
            (Cmd, "ss -tlnp") => "netstat -ano | findstr LISTENING",
            (Cmd, "ls -la") => "dir /a",
            _ => return Some(Cow::Borrowed(command)),
        };
        Some(Cow::Borrowed(adapted))
    }
}

/// docker 是否在 PATH 中（首次用到时检查一次）
fn docker_available() -> bool {
    static AVAILABLE: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *AVAILABLE.get_or_init(|| {
        #[cfg(windows)]
        {
            crate::terminal::find_windows_executable("docker.exe").is_some()
        }
        #[cfg(not(windows))]
        {
            true
        }
    })
}
