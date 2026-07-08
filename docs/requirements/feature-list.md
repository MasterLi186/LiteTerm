# LiteTerm 功能需求文档

## 已实现功能

### 终端核心
- [x] 多标签管理(新建/关闭/切换/拖拽排序)
- [x] 水平/垂直分屏(递归分屏,可拖拽分割线)
- [x] 6 套终端主题(暗色默认/Monokai/Solarized/Dracula/One Dark/浅色)
- [x] 终端搜索(Ctrl+Shift+F)
- [x] 终端录屏(asciicast v2 格式)+ 回放
- [x] 日志录制(自动去 ANSI 转义)
- [x] Scrollback 10000 行 + 右键清空缓存释放内存
- [x] 文件路径点击打开(registerLinkProvider)
- [x] URL 点击打开(WebLinksAddon)

### SSH 连接
- [x] SSH 密码/密钥/Agent 认证
- [x] SSH 密钥管理(生成/导入/查看)
- [x] AES-256-GCM 加密密码存储(machine-bound)
- [x] 连接书签(分组/收藏/导入/导出)
- [x] ProxyJump 跳板机支持(系统 ssh -J)
- [x] 串口终端(serialport crate)
- [x] SSH 端口转发/隧道管理
- [x] 断开提示 + 手动重连(按 Enter / 点击按钮)
- [x] 断连诊断日志(EOF/READ-ERR + errno + host)
- [x] SSH keepalive(30s 服务端 + 15s 手动)
- [x] OSC7 cwd 跟踪(bash/zsh/fish)

### 文件传输
- [x] SFTP 双栏文件管理器(本地+远端)
- [x] 拖拽上传(SFTP 通道 + ZMODEM fallback)
- [x] SFTP 下载/上传进度浮窗(含实时速率)
- [x] ZMODEM 收发(rz/sz,堡垒机穿透)
- [x] 传输取消

### 系统监控
- [x] 远端 SSH 监控(CPU/内存/Swap/磁盘/网络/负载/运行时间)
- [x] 本地 sysinfo 监控(跨平台:Linux/macOS/Windows)
- [x] 同一主机多标签共享监控线程
- [x] 监控数据缓存(切标签零延迟)
- [x] 网络流量按接口显示

### 进程管理器
- [x] 远端进程列表(CPU/内存/用户/命令/启动时间)
- [x] 进程详情(进程树链 + 完整命令行 + 环境变量 + 可执行文件 + 工作目录)
- [x] 进程状态统计(总数/运行/休眠/僵尸/停止)
- [x] 进程异常告警(>10000 进程 / >100 僵尸)
- [x] 3 秒自动刷新

### 效率工具
- [x] WindTerm 式历史命令自动补全(bash 下,打字即弹出)
- [x] shell 类型检测(bash 启用补全,zsh/fish 跳过)
- [x] 批量命令执行
- [x] 命令收藏/历史
- [x] 底部命令输入栏
- [x] 快捷键自定义
- [x] adb shell 自动修正终端尺寸

### 其他
- [x] 关于对话框(版本号 + 系统信息)
- [x] NSIS 安装窗口中文化
- [x] 会话持久化(重启恢复标签 + 文件管理器状态)
- [x] 前端日志桥(appLog → ~/guishell.log)
- [x] IME 输入法兼容(WebKitGTK compositionend 去重)

## 待实现功能

### 优先级高
- [ ] AI 命令生成(自然语言→shell 命令)
- [ ] AI 终端输出解释(右键选中文本)
- [ ] 非活跃标签 scrollback 自动缩减(省内存)

### 优先级中
- [ ] 多会话同步输入(同时往多个终端发命令)
- [ ] 智能命令补全(OSC 133 + SQLite frecency)
- [ ] SSH 配置导入(~/.ssh/config)

### 暂不实现
- 移动端支持
- RDP/VNC 远程桌面
- X11 转发
- 插件系统
- Telnet/TFTP
