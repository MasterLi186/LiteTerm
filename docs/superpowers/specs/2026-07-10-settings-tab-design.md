# 设置标签页设计文档

> 日期: 2026-07-10
> 状态: 已批准，待实现

## 1. 目标

将设置从模态弹窗改为独立标签页，对标 Tabby 设置界面。左侧分类导航，右侧详细配置。配色方案用全宽终端预览图列表展示，选中后实时预览到终端。

## 2. 布局

```
┌─ 标签栏 ──────────────────────────────────────────────────┐
│ 155bmc × │ 终端 1 × │ ⚙ 设置 × │                       + │
├───────────┬───────────────────────────────────────────────┤
│ 左侧导航   │ 右侧配置内容                                 │
│ (180px)   │                                               │
│           │  配色方案                                      │
│ ○ 终端    │  ┌─────────────────────────────────────────┐  │
│ ○ 外观    │  │ 当前: 暗色默认              [✎ 编辑]    │  │
│ ○ 快捷键  │  │ ┌─ 预览 ──────────────────────────────┐ │  │
│ ○ SSH     │  │ │ user@host:~$ ls                     │ │  │
│ ○ 传输    │  │ │ -rwxr-xr-x Documents Downloads      │ │  │
│ ○ ZMODEM  │  │ └────────────────────────────────────┘ │  │
│ ○ 关于    │  └─────────────────────────────────────────┘  │
│           │                                               │
│           │  [🔍 搜索配色方案...]                          │
│           │                                               │
│           │  3024 Day            ●●●●●●●●●●               │
│           │  ┌────────────────────────────────────────┐   │
│           │  │ john@doe-pc$ ls                        │   │
│           │  │ -rwxr-xr-x 1 root Documents            │   │
│           │  │ -rwxr-xr-x 1 root Downloads            │   │
│           │  └────────────────────────────────────────┘   │
│           │                                               │
│           │  3024 Night          ●●●●●●●●●●               │
│           │  ┌────────────────────────────────────────┐   │
│           │  │ john@doe-pc$ ls                        │   │
│           │  └────────────────────────────────────────┘   │
├───────────┴───────────────────────────────────────────────┤
```

## 3. 七个分类

### 3.1 终端

| 配置项 | 控件 | 数据源 |
|--------|------|--------|
| 配色方案 (191 套) | 全宽预览列表 + 搜索 + 8 色色点 | localStorage `guishell_terminal_theme` |
| 终端字体 | 下拉选择 | localStorage `guishell_terminal_font` |
| 字号 | 滑块 + 数字输入 (8-48) | localStorage `guishell_terminal_fontsize` |
| 使用系统字体 | 开关 | localStorage `guishell_use_system_font` |

配色列表每项结构（对标 Tabby）：
- 行头：配色名称（左）+ 8 个 ANSI 色圆点（右）
- 行体：全宽终端预览 div，用配色的 background/foreground/ANSI 色渲染固定文本
- 选中项：蓝色边框 + ✓ 标记
- 选中时实时应用到所有终端（`terminal-settings-changed` 事件）

预览文本固定为：
```
john@doe-pc$ ls        ← foreground + green(user) + blue(host)
-rwxr-xr-x 1 root Documents   ← foreground + green
-rwxr-xr-x 1 root Downloads   ← foreground + yellow(bg highlight)
-rwxr-xr-x 1 root Pictures    ← foreground + cyan
-rwxr-xr-x 1 root Music       ← foreground + magenta
```

### 3.2 外观

| 配置项 | 控件 | 数据源 |
|--------|------|--------|
| 侧边栏宽度 | 滑块 (100-400px) | settings.toml `appearance.sidebar_width` |
| 文件管理器高度 | 滑块 (100-500px) | settings.toml `appearance.file_browser_height` |
| 显示侧边栏 | 开关 | settings.toml `appearance.show_sidebar` |
| 显示文件管理器 | 开关 | settings.toml `appearance.show_file_browser` |

### 3.3 快捷键

复用现有 `ShortcutSettings` 组件的逻辑，嵌入设置标签页。

| 配置项 | 控件 | 数据源 |
|--------|------|--------|
| 9 个快捷键绑定 | 点击录入式输入框 | localStorage `guishell_shortcuts` |
| 恢复默认 | 按钮 | — |

### 3.4 SSH

| 配置项 | 控件 | 数据源 |
|--------|------|--------|
| 连接超时 (秒) | 数字输入 | settings.toml `ssh.connect_timeout_secs` |
| Keepalive 间隔 (秒) | 数字输入 | settings.toml `ssh.keepalive_interval_secs` |
| 默认字符集 | 下拉 (UTF-8/GBK/GB2312) | settings.toml `ssh.default_charset` |

### 3.5 传输

| 配置项 | 控件 | 数据源 |
|--------|------|--------|
| 默认下载目录 | 路径输入 + 浏览按钮 | settings.toml `transfer.default_download_dir` |
| 断点续传阈值 (MB) | 数字输入 | settings.toml `transfer.resume_threshold_mb` |
| 最大重试次数 | 数字输入 | settings.toml `transfer.max_retries` |
| 并发传输数 | 数字输入 (1-8) | settings.toml `transfer.concurrent_transfers` |

### 3.6 ZMODEM

| 配置项 | 控件 | 数据源 |
|--------|------|--------|
| 启用 ZMODEM | 开关 | settings.toml `zmodem.enabled` |
| 自动检测 | 开关 | settings.toml `zmodem.auto_detect` |
| 下载目录 | 路径输入 + 浏览按钮 | settings.toml `zmodem.download_dir` |
| 超时时间 (秒) | 数字输入 | settings.toml `zmodem.timeout_secs` |

### 3.7 关于

- 应用版本号（从 Tauri `get_system_info` 获取）
- 操作系统 / 架构 / 主机名 / 用户名
- 开源协议
- GitHub 链接

## 4. 数据流

### localStorage 配置（终端/快捷键）

立即生效，无需保存按钮。修改后触发 `terminal-settings-changed` 事件，所有终端实时刷新。与当前行为一致。

### settings.toml 配置（外观/SSH/传输/ZMODEM）

需要新增两个 Tauri 命令：

```rust
#[tauri::command]
pub async fn get_settings(state: State<'_, AppState>) -> Result<serde_json::Value, String>

#[tauri::command]
pub async fn update_settings(state: State<'_, AppState>, patch: serde_json::Value) -> Result<(), String>
```

`update_settings` 接收部分更新（patch），合并到内存中的 Settings，然后写入 `settings.toml`。前端修改任何 settings.toml 字段时自动调用。

## 5. 文件结构

| 文件 | 职责 |
|------|------|
| 新增 `src/components/settings/SettingsTab.tsx` | 设置标签页主组件（左侧导航 + 右侧内容路由） |
| 新增 `src/components/settings/TerminalSection.tsx` | 终端分类（配色 + 字体） |
| 新增 `src/components/settings/AppearanceSection.tsx` | 外观分类 |
| 新增 `src/components/settings/ShortcutsSection.tsx` | 快捷键分类（包装 ShortcutSettings 逻辑） |
| 新增 `src/components/settings/SshSection.tsx` | SSH 分类 |
| 新增 `src/components/settings/TransferSection.tsx` | 传输分类 |
| 新增 `src/components/settings/ZmodemSection.tsx` | ZMODEM 分类 |
| 新增 `src/components/settings/AboutSection.tsx` | 关于分类 |
| 新增 `src/components/settings/ThemePreview.tsx` | 配色预览卡片组件 |
| 修改 `src/types/index.ts` | Tab.type 加入 `'settings'` |
| 修改 `src/App.tsx` | 齿轮按钮改为打开设置标签 + 渲染 SettingsTab |
| 修改 `src-tauri/src/commands/terminal.rs` | 新增 get_settings / update_settings 命令 |
| 修改 `src-tauri/src/lib.rs` | 注册新命令 |

## 6. 非目标

- 不做整体暗色/亮色主题切换（保持独立 TODO）
- 不做设置导入/导出
- 不改 settings.toml 的 struct 结构
- 不做设置搜索（分类导航已足够）
