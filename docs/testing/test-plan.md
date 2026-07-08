# LiteTerm 测试文档

## 自动化测试

### Rust 单元测试
```bash
# 旧核心库(GTK4 时代的解析器)
cargo test

# Tauri 后端
cd src-tauri && cargo test
```

现有测试:
- `tests/zmodem_frame_test.rs` — ZMODEM 帧编解码
- `tests/monitor_parse_test.rs` — 监控数据解析(/proc/stat, /proc/meminfo 等)

### TypeScript 类型检查
```bash
npx tsc --noEmit
```

### Rust 静态分析
```bash
cd src-tauri && cargo clippy
```

### 完整质量门(build.sh)
```bash
./build.sh
# 依次执行: tsc → vite build → cargo build → clippy → cargo test → 验证产物
```

## 手动测试清单

### 终端基础
- [ ] 新建本地终端,输入命令,有输出
- [ ] 新建 SSH 终端,连接成功,有 prompt
- [ ] 分屏(水平/垂直),两个面板独立工作
- [ ] 标签拖拽排序
- [ ] 终端搜索(Ctrl+Shift+F)
- [ ] 终端主题切换(右键菜单)
- [ ] 清屏 / 清空缓存
- [ ] 终端录屏 → 回放
- [ ] 窗口最大化/还原 → 终端自动 resize

### SSH 连接
- [ ] 密码认证连接
- [ ] 密钥认证连接
- [ ] 跳板机(ProxyJump)连接
- [ ] 保存连接书签 → 关闭 → 左侧点击 → 不弹密码框
- [ ] 远端 `exit` → 显示"连接已断开" → 按 Enter 重连
- [ ] 标签圆点:连接绿/断开红
- [ ] 导出配置 → 导入配置

### 文件传输
- [ ] SFTP 文件管理器:浏览远端目录
- [ ] 拖拽文件到终端 → SFTP 上传(直连)
- [ ] 拖拽文件到终端 → ZMODEM 上传(堡垒机)
- [ ] SFTP 下载文件
- [ ] 传输进度浮窗 → 取消传输

### 系统监控
- [ ] SSH 连接后侧边栏显示 CPU/内存/磁盘/网络
- [ ] 本地终端侧边栏显示本机 sysinfo 监控
- [ ] 多标签连同一主机 → 监控共享(不重复)
- [ ] 切标签 → 监控零延迟(从缓存)
- [ ] 进程管理器:进程列表 + 详情 + 进程树

### 输入法兼容(Linux WebKitGTK)
- [ ] 中文输入法打中文:不堆积
- [ ] 中文输入法打英文(空格上屏):不加倍
- [ ] 直接英文输入:正常

### 跨平台
- [ ] Linux: 全功能可用
- [ ] Windows: SSH/密码存储/终端正常,NSIS 安装中文
- [ ] macOS: SSH/密码存储/终端正常

## CI 质量门

### GitHub Actions (`.github/workflows/build.yml`)
触发条件:
- `v*` tag → 正式 Release(三平台)
- `dev-*` tag → Artifact only(三平台,不创建 Release)

每次构建执行:
1. TypeScript 类型检查(`npx tsc --noEmit`)
2. Rust clippy(`cargo clippy -- -D warnings || true`)
3. Rust 测试(`cargo test || true`)
4. Tauri 构建(`tauri-action`)
5. 版本号从 tag 自动提取

## 已知限制

- 158BitNet AI 集成暂未上线(ARM only,x86 编译不过)
- Windows 上 `sysinfo` 负载均衡返回 0(Windows 无此概念)
- WebKitWebProcess 长时间运行内存会缓慢增长(xterm scrollback)
- `ps -H`(进程树)在 >10000 进程的服务器上会超时,已禁用
