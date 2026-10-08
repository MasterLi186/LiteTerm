export type UiLocale = 'zh-CN' | 'en';

const english: Record<string, string> = {
  '连接到': 'Connect to', '请输入密码': 'Enter password', '取消': 'Cancel', '连接': 'Connect',
  '命令输入:': 'Command:', '输入命令后回车发送到终端': 'Enter a command and press Enter to send it to the terminal',
  '命令历史': 'Command history', '收藏命令': 'Favorite commands', '命令历史 (': 'Command history (',
  '确认清空所有命令历史？': 'Clear all command history?', '清空': 'Clear', '暂无历史记录': 'No command history yet',
  '点击填入输入框': 'Click to fill the input', '执行': 'Run', '复制': 'Copy', '删除': 'Delete',
  '取消收藏': 'Remove favorite', '收藏': 'Add to favorites', '收藏命令 (': 'Favorite commands (',
  '暂无收藏，在历史记录中点击星标收藏': 'No favorites yet. Star a command in History to add it here.',
  '连接管理': 'Connections', '导入配置': 'Import configuration', '导出配置': 'Export configuration',
  'SSH 密钥管理': 'SSH key manager', '新建连接': 'New connection', '暂无连接': 'No connections yet',
  '新建会话': 'New session', '编辑属性': 'Edit properties', '关闭': 'Close', '新建标签页': 'New tab',
  '批量命令': 'Batch commands', 'SSH 隧道': 'SSH tunnels', '设置': 'Settings', '调试日志': 'Debug log',
  '重命名': 'Rename', '复制标签页': 'Duplicate tab', '重新连接': 'Reconnect', '关闭其他': 'Close others',
  '关闭标签页': 'Close tab', '关闭标签': 'Close tab', '新建本地终端': 'New local terminal', '重命名标签页': 'Rename tab',
  '确定': 'Confirm', '暂无终端。点击 + 新建一个。': 'No terminal yet. Click + to create one.',
  '释放文件上传到远程服务器': 'Drop files here to upload them to the remote server', '按 Enter 重新连接': 'Press Enter to reconnect',
  '取消重连': 'Cancel reconnect', '文件传输 (': 'File transfers (', '取消传输': 'Cancel transfer',
  '↑↓ 传输中 (': '↑↓ Transferring (', '暂无打开的 SSH 连接': 'No open SSH connections', '目标终端 (': 'Target terminals (',
  '命令': 'Command', '输入要执行的命令...': 'Enter a command to run...', '已发送到': 'Sent to', '个终端': 'terminals',
  '发送失败:': 'Failed to send:', '名称': 'Name', '我的服务器': 'My server', '主机地址': 'Host address',
  '端口': 'Port', '用户名': 'Username', '认证方式': 'Authentication', '密码': 'Password', '密钥文件': 'Key file',
  '密钥密码 (可选)': 'Key passphrase (optional)', '分组': 'Group', '跳板机 (ProxyJump)': 'Jump host (ProxyJump)',
  'user@bastion:22 (可选)': 'user@bastion:22 (optional)', '只保存': 'Save only', '(留空则不修改)': '(Leave blank to keep unchanged)',
  '编辑连接': 'Edit connection', '保存并连接': 'Save and connect', 'TOML 文件': 'TOML files', '串口:': 'Serial:',
  '搜索 (Ctrl+F)': 'Search (Ctrl+F)', '搜索文件名...': 'Search file names...', '搜索': 'Search', '项': 'items',
  '刷新': 'Refresh', '上级目录': 'Parent directory', '文件名': 'File name', '大小': 'Size', '类型': 'Type',
  '修改时间': 'Modified', '加载中...': 'Loading...', '文件': 'Files', '文件夹': 'Folders', '已连接': 'Connected',
  '未连接': 'Disconnected', '本地文件': 'Local files', '远程文件': 'Remote files',
  '未连接远程主机 — 连接SSH后显示远程文件': 'Not connected to a remote host — connect via SSH to view remote files',
  '命令收藏 -- 点击直接发送到终端': 'Favorite commands — click to send directly to the terminal', '保存': 'Save',
  '暂无收藏命令，点击"添加"创建': 'No favorite commands yet. Click “Add” to create one.',
  '隐藏隐藏文件': 'Hide hidden files', '显示隐藏文件': 'Show hidden files', '隐藏.': 'Hide .', '显示.': 'Show .',
  '网卡:': 'Network interface:', '刷新:': 'Refresh:', '条连接': 'connections', '状态': 'Status',
  '本地地址': 'Local address', '远程地址': 'Remote address', '进程': 'Processes', '没有活跃连接': 'No active connections',
  '上没有活跃连接': ' has no active connections', 'Shell 环境': 'Shell environment', '未检测到可用 Shell': 'No available shell detected',
  'SSH 连接': 'SSH connections', '暂无保存的连接': 'No saved connections', '+ 新建 SSH 连接': '+ New SSH connection',
  '串口设备': 'Serial devices', '扫描中...': 'Scanning...', '未检测到串口设备': 'No serial devices detected',
  '用户': 'User', '内存': 'Memory', '启动时间': 'Start time', '可执行文件': 'Executable', '工作目录': 'Working directory',
  '进程树': 'Process tree', '完整命令行': 'Full command line', '环境变量 (': 'Environment variables (',
  '进程列表 -': 'Processes -', '共': 'Total', '运行': 'Running', '休眠': 'Sleeping', '僵尸': 'Zombie', '停止': 'Stopped',
  '⚠ 进程数异常(': '⚠ Unusually high process count (', '),可能存在进程泄漏': '); a process may be leaking',
  '⚠ 僵尸进程过多(': '⚠ Too many zombie processes (', '),建议排查父进程': '); check the parent process',
  '名称 | 命令行': 'Name | Command line', '请输入标签名称': 'Enter a label', '请输入命令内容': 'Enter a command',
  '添加快捷命令': 'Add shortcut', '编辑': 'Edit', '新增': 'Add new', '管理': 'Manage', '管理快捷命令': 'Manage shortcuts',
  '暂无快捷命令': 'No shortcuts yet', '内置': 'Built-in', '+ 添加': '+ Add', '确认删除快捷命令「': 'Delete shortcut “',
  '标签名称(必填,最多20字)': 'Label (required, up to 20 characters)', '命令内容(必填)': 'Command (required)',
  '加载失败:': 'Failed to load:', '重播': 'Replay', '系统设置': 'System settings', '使用系统字体': 'Use system font',
  '勾选后使用操作系统默认等宽字体': 'Use the operating system’s default monospace font', '终端字体': 'Terminal font',
  '字号 (8-48)': 'Font size (8–48)', '终端配色 (': 'Terminal themes (', '套': 'themes', '套)': 'themes)',
  '搜索配色方案...': 'Search themes...', '预览': 'Preview', '快捷键设置': 'Keyboard shortcuts',
  '请按下新的快捷键...': 'Press the new shortcut...', '按下快捷键...': 'Press a shortcut...', '确认': 'Confirm',
  '修改': 'Change', '恢复默认': 'Restore defaults', '连接后显示监控': 'Monitoring appears after connecting', '采集中...': 'Collecting...',
  '负载:': 'Load:', '网络: ↑': 'Network: ↑', '正在采集系统信息...': 'Collecting system information...', '运行时间': 'Uptime',
  '系统负载': 'System load', '资源': 'Resources', '交换': 'Swap', '网络': 'Network', '点击查看网络详情': 'Click to view network details',
  '磁盘': 'Disk', '挂载点': 'Mount point', '可用/总量': 'Available / total', '公钥 (': 'Public key (', '未找到公钥': 'Public key not found',
  '查看': 'View', '私钥 (': 'Private key (', '未找到私钥': 'Private key not found', '无指纹': 'No fingerprint', '已复制': 'Copied',
  '备注 (可选)': 'Note (optional)', '生成中...': 'Generating...', '生成新密钥': 'Generate new key', '录制中': 'Recording',
  '点击停止': 'Click to stop', '录屏中': 'Recording', '搜索...': 'Search...', '上一个 (Shift+Enter)': 'Previous (Shift+Enter)',
  '上一个': 'Previous', '下一个 (Enter)': 'Next (Enter)', '下一个': 'Next', '关闭 (Esc)': 'Close (Esc)',
  'SSH 隧道管理': 'SSH tunnel manager', '活跃隧道 (': 'Active tunnels (', '暂无活跃隧道': 'No active tunnels', '新建隧道': 'New tunnel',
  'SSH 主机': 'SSH host', '选择连接...': 'Select a connection...', '本地端口': 'Local port', '远程主机': 'Remote host',
  '远程端口': 'Remote port', '创建中...': 'Creating...', '创建隧道': 'Create tunnel', '关于': 'About', '关于 LiteTerm': 'About LiteTerm',
  '轻量级跨平台 SSH 客户端': 'A lightweight, cross-platform SSH client', '版本': 'Version', '操作系统': 'Operating system',
  '主机名': 'Hostname', '基于 Tauri 2 + React + xterm.js 构建': 'Built with Tauri 2, React, and xterm.js',
  '外观': 'Appearance', '显示侧边栏': 'Show sidebar', '侧边栏宽度 (': 'Sidebar width (', '显示文件管理器': 'Show file manager',
  '文件管理器高度 (': 'File manager height (', '快捷键': 'Shortcuts', '恢复默认快捷键': 'Restore default shortcuts',
  '连接超时 (秒)': 'Connection timeout (seconds)', 'Keepalive 间隔 (秒)': 'Keepalive interval (seconds)', '默认字符集': 'Default character encoding',
  '终端': 'Terminal', '当前配色方案:': 'Current theme:', '传输': 'Transfers', '默认下载目录': 'Default download directory',
  '断点续传阈值 (MB)': 'Resume threshold (MB)', '最大重试次数': 'Maximum retries', '并发传输数 (1-8)': 'Concurrent transfers (1–8)',
  '启用 ZMODEM': 'Enable ZMODEM', '自动检测': 'Detect automatically', '下载目录': 'Download directory', '超时时间 (秒)': 'Timeout (seconds)',
  '取消全选': 'Deselect all', '全选': 'Select all', '查看磁盘': 'View disk',
  '查看内存': 'View memory', '查看进程': 'View processes', '网络连接': 'Network connections', '系统日志': 'System log',
  '暂停': 'Pause', '播放': 'Play', '完成': 'Done', '保存下载文件': 'Save downloaded file', '所有文件': 'All files',
  '选择日志保存位置': 'Choose where to save the log', '日志文件': 'Log files', '选择录屏保存位置': 'Choose where to save the recording',
  '录屏文件': 'Recording files', '选择录屏文件': 'Choose a recording file', '清屏': 'Clear terminal', '清空缓存': 'Clear cache',
  '粘贴': 'Paste', '水平分屏': 'Split horizontally', '垂直分屏': 'Split vertically',
  '搜索 (Ctrl+Shift+F)': 'Search (Ctrl+Shift+F)', '终端主题': 'Terminal theme', '⏹ 停止录制日志': '⏹ Stop log recording',
  '⏺ 开始录制日志': '⏺ Start log recording', '⏹ 停止录屏': '⏹ Stop recording', '⏺ 开始录屏': '⏺ Start recording',
  '▶ 回放录屏': '▶ Play recording', '关闭面板': 'Close panel', '等待传输...': 'Waiting for transfer...', '✓ 完成': '✓ Done',
  '新建标签': 'New tab', '下一标签': 'Next tab', '上一标签': 'Previous tab', '1 秒': '1 second', '2 秒': '2 seconds',
  '3 秒': '3 seconds', '5 秒': '5 seconds', '10 秒': '10 seconds', '30 秒': '30 seconds', '点击连接': 'Click to connect',
  '下载到本地 (': 'Download to local (', '上传到远程 (': 'Upload to remote (', '语言': 'Language',
  '本机': 'This device', '连接已断开': 'Connection closed', '加载连接列表失败:': 'Failed to load connection list:',
  '加载连接失败:': 'Failed to load connections:', '加载进程详情失败:': 'Failed to load process details:', '重命名失败:': 'Rename failed:',
  '无法读取远程目录，请手动输入路径': 'Unable to read the remote directory. Enter the path manually.', '发送中...': 'Sending...',
  '进程 -': 'Processes -', '进程 - 本机': 'Processes - This device', '网络 -': 'Network -', '网络 - 本机': 'Network - This device',
  '录屏回放': 'Recording playback', '回放:': 'Playback:', '重连失败': 'Reconnect failed', '重连失败:': 'Reconnect failed:',
  '打开终端失败:': 'Failed to open terminal:', '打开串口失败:': 'Failed to open serial port:', '分屏失败:': 'Failed to split the terminal:',
  '上传失败:': 'Upload failed:', '删除失败:': 'Delete failed:', '保存失败:': 'Save failed:', '导出配置失败:': 'Failed to export configuration:',
  '导入配置失败:': 'Failed to import configuration:', '复制连接失败:': 'Failed to copy connection:', '加载隧道列表失败:': 'Failed to load tunnels:',
  '创建隧道失败:': 'Failed to create tunnel:', '关闭隧道失败:': 'Failed to close tunnel:', '加载密钥失败:': 'Failed to load keys:',
  '生成密钥失败:': 'Failed to generate key:', '读取公钥失败:': 'Failed to read public key:', '复制公钥失败:': 'Failed to copy public key:',
  '连接失败:': 'Connection failed:', '▼ 隐藏文件管理器': '▼ Hide file manager', '▲ 显示文件管理器': '▲ Show file manager',
};

const browserLocales = typeof navigator === 'undefined' ? [] : [...(navigator.languages ?? []), navigator.language];
let currentLocale: UiLocale = resolveUiLocale(...browserLocales);

export function resolveUiLocale(...locales: Array<string | null | undefined>): UiLocale {
  const preferred = locales.find(locale => locale?.trim());
  if (!preferred) return 'en';
  const normalized = preferred.trim().split('.')[0].split('@')[0].replace(/_/g, '-').toLowerCase();
  return normalized === 'c' || normalized === 'posix' || !normalized.startsWith('zh') ? 'en' : 'zh-CN';
}

export function setUiLocale(locale: UiLocale): void {
  currentLocale = locale;
  if (typeof document !== 'undefined') document.documentElement.lang = locale;
}

export function t(chinese: string): string {
  return currentLocale === 'zh-CN' ? chinese : (english[chinese] ?? chinese);
}

export function formatUptime(value: string): string {
  if (currentLocale === 'zh-CN' || value === 'N/A') return value;
  const days = value.match(/(\d+)\s*天/)?.[1];
  const hours = value.match(/(\d+)\s*(?:小时|时)/)?.[1];
  const minutes = value.match(/(\d+)\s*(?:分钟|分)/)?.[1];
  const parts = [days && `${days}d`, hours && `${hours}h`, minutes && `${minutes}m`].filter(Boolean);
  return parts.length ? parts.join(' ') : value;
}
