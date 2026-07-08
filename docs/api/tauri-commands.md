# LiteTerm Tauri 命令 API 文档

所有 Tauri 命令通过 `invoke('command_name', { params })` 从前端调用。

## 终端管理

| 命令 | 参数 | 返回 | 说明 |
|------|------|------|------|
| `open_local_terminal` | — | `String` (id) | 打开本地终端 |
| `open_shell_terminal` | `shellPath: String` | `String` (id) | 指定 shell 打开终端 |
| `terminal_write` | `id: String, data: Vec<u8>` | `()` | 写入终端 |
| `terminal_resize` | `id: String, cols: u32, rows: u32` | `()` | 调整终端大小 |
| `close_terminal` | `id: String` | `()` | 关闭终端(含 SFTP/监控清理) |
| `list_shells` | — | `Vec<ShellInfo>` | 列出可用 shell |
| `open_file_path` | `id: String, path: String` | `()` | Ctrl+Click 打开文件 |
| `get_default_shell` | — | `String` | 获取默认 shell |
| `get_system_info` | — | `JSON` | 系统信息(关于对话框) |
| `frontend_log` | `category: String, message: String` | `()` | 前端日志写入文件 |

## SSH

| 命令 | 参数 | 返回 | 说明 |
|------|------|------|------|
| `ssh_connect` | `host, port, user, password?, authMethod, keyPath?, label, proxyJump?, cols?, rows?` | `String` (id) | SSH 连接 |
| `ssh_supported_algs` | — | `JSON` | 支持的加密算法 |

## SFTP

| 命令 | 参数 | 返回 | 说明 |
|------|------|------|------|
| `start_sftp_session` | `sessionId, host, port, user, password?, authMethod, keyPath?` | `()` | 建立 SFTP 会话 |
| `sftp_list_dir` | `sessionId: String, path: String` | `Vec<FileEntry>` | 列目录 |
| `sftp_download` | `sessionId, remotePath, localPath` | `()` | 下载文件 |
| `sftp_upload` | `sessionId, localPath, remotePath` | `()` | 上传文件 |
| `sftp_delete` | `sessionId, path` | `()` | 删除远端文件 |
| `sftp_rename` | `sessionId, oldPath, newPath` | `()` | 重命名 |
| `sftp_exec` | `sessionId: String, command: String` | `String` | 执行远端命令 |
| `drag_upload` | `sessionId, files: Vec<String>, fallbackDir?` | `()` | 拖拽上传 |
| `list_local_dir` | `path: String` | `Vec<FileEntry>` | 列本地目录 |
| `local_delete` | `path: String` | `()` | 删除本地文件 |
| `local_rename` | `oldPath, newPath` | `()` | 本地重命名 |
| `save_file` | `path: String, data: Vec<u8>` | `()` | 保存文件 |
| `cancel_transfer` | `transferKey: String` | `()` | 取消传输 |
| `read_local_file` | `path: String` | `String` | 读本地文件 |

## 监控

| 命令 | 参数 | 返回 | 说明 |
|------|------|------|------|
| `start_monitor` | `sessionId, monitorKey?, host, port, user, password?, authMethod, keyPath?` | `()` | 启动 SSH 远端监控 |
| `start_local_monitor` | — | `()` | 启动本地 sysinfo 监控 |

## 连接管理

| 命令 | 参数 | 返回 | 说明 |
|------|------|------|------|
| `load_connections` | — | `ConnectionStore` | 加载连接书签 |
| `save_connection` | `groupId, groupLabel, groupColor, hostId, config` | `()` | 保存连接 |
| `delete_connection` | `groupId, hostId` | `()` | 删除连接 |

## 密码存储

| 命令 | 参数 | 返回 | 说明 |
|------|------|------|------|
| `store_password` | `user, host, port, password` | `()` | 存密码(AES-256-GCM) |
| `retrieve_password` | `user, host, port` | `Option<String>` | 取密码 |

## 其他

| 命令 | 参数 | 返回 | 说明 |
|------|------|------|------|
| `export_config` | `path: String` | `()` | 导出配置 |
| `import_config` | `path: String` | `()` | 导入配置 |
| `read_text_file` | `path: String` | `String` | 读文本文件(支持 ~) |
| `create_tunnel` | `...` | `String` (id) | 创建 SSH 隧道 |
| `list_tunnels` | — | `Vec<TunnelInfo>` | 列隧道 |
| `close_tunnel` | `id: String` | `()` | 关闭隧道 |
| `start_recording` | `terminalId, filePath` | `()` | 开始录屏 |
| `stop_recording` | `terminalId` | `()` | 停止录屏 |
| `record_event` | `terminalId, data` | `()` | 录屏事件 |
| `list_ssh_keys` | — | `Vec<SshKeyInfo>` | 列 SSH 密钥 |
| `generate_ssh_key` | `...` | `()` | 生成 SSH 密钥 |
| `list_serial_ports` | — | `Vec<SerialPortInfo>` | 列串口 |
| `open_serial_terminal` | `device, baudRate` | `String` (id) | 打开串口终端 |

## 前端事件(emit)

| 事件 | Payload | 方向 | 说明 |
|------|---------|------|------|
| `terminal-output` | `{ id, data: number[] }` | 后端→前端 | 终端输出 |
| `terminal-closed` | `{ id }` | 后端→前端 | 终端关闭/断开 |
| `monitor-data` | `MonitorPayload` | 后端→前端 | 监控数据(每 2 秒) |
| `transfer-progress` | `{ filename, bytes_transferred, total_bytes, direction }` | 后端→前端 | 传输进度 |
