# WebKitGTK 内存不释放问题

## 问题描述

LiteTerm 长时间运行后，`WebKitWebProcess` 子进程内存持续增长（观测到 1.38GB RSS），即使关闭所有标签页也不会释放。用户必须重启 LiteTerm 才能回收内存。

## 发现时间

2026-07-13

## 环境

- 系统：Ubuntu 22.04 (x86_64)
- 内核：5.19.0-32-generic
- Tauri：2.x（使用 WebKitGTK 4.1 作为 WebView 引擎）
- WebKitGTK：系统自带版本

## 现象

```
# guishell-tauri 主进程内存正常
PID=2996859  RSS=175MB   guishell-tauri

# WebKitWebProcess 子进程内存异常（1.38GB）
PID=2996897  RSS=1380MB  WebKitWebProcess
```

进程树：
```
guishell-tauri
├── WebKitNetworkProcess   (网络子进程)
└── WebKitWebProcess       (渲染子进程 ← 内存在这里)
```

关闭所有标签页后，JS 对象和 DOM 节点被销毁，但 `WebKitWebProcess` 的 RSS 不下降。

## 根因分析

### WebKitGTK 的内存回收策略

WebKitGTK 使用 `bmalloc` 内存分配器，free 后的内存保留在进程内部的空闲池中，不归还给操作系统。这是 WebKit 的设计决策（优先分配速度而非内存归还），与 Chromium 的 `PartitionAlloc`（会主动调用 `madvise(MADV_FREE)` 归还）形成对比。

### 内存增长的主要来源

1. **xterm.js scrollback 缓冲区** — 每个终端默认 10000 行 scrollback，多标签累积
2. **Canvas/WebGL 纹理缓存** — xterm.js 的字符渲染纹理图集，不随标签关闭清除
3. **JIT 编译缓存** — JavaScriptCore 的 JIT 代码缓存不主动释放
4. **DOM 布局数据** — WebKit 内部的样式/布局树残留

### 为什么 Electron 没有这个问题

Electron 使用 Chromium，其 `PartitionAlloc` 分配器会定期扫描空闲页并通过 `madvise` 归还给 OS。Chromium 还有 `MemoryPressureListener` 在系统内存紧张时主动回收。WebKitGTK 缺少这些机制。

## 影响范围

- **仅 Linux 受影响**：Tauri 在 macOS 上使用 WKWebView（Apple 的 WebKit，有不同的内存管理），在 Windows 上使用 WebView2（Chromium 内核）
- **长时间运行场景严重**：开十几个 SSH 标签跑一天后，内存可能增长到 2-3GB
- **重启可完全恢复**：关闭 LiteTerm 窗口后内存立即释放

## 相关问题：退出后 WebKit 孤儿进程

### 现象

主界面卡死 → 任务栏右键「退出」只杀掉 `guishell-tauri` → `WebKitWebProcess` 被 init 收养，RSS 可残留到 2–3.5GB。

### 修复（2026-07-18）

`src-tauri/src/process_cleanup.rs`：

1. **SIGTERM/SIGINT** — 任务栏退出走信号路径，handler 置位后 watcher 线程递归 SIGKILL 所有后代再 `exit`
2. **RunEvent::Exit / ExitRequested / force_quit** — 同样清理子进程树
3. **启动时** — 若无其它 guishell 实例，杀掉 PPID=1 且 exe 为 `webkit2gtk` 的孤儿 WebKit 进程

> 若主进程被 **SIGKILL** 强杀，handler 无法运行；下次启动时的孤儿清理会回收内存。

### 应急命令（当前已卡死时）

```bash
# 查看
ps -eo pid,ppid,rss,comm | awk '$3>500000 || /WebKit|guishell/'
# 只杀 webkit2gtk 渲染进程（不动 Chrome）
for p in /proc/[0-9]*; do
  e=$(readlink $p/exe 2>/dev/null) || continue
  case "$e" in *webkit2gtk*WebKit*) kill -9 ${p#/proc/};; esac
done
```

## 解决方案

### 短期缓解（当前技术栈内）

1. **关闭标签时主动清理** — `term.dispose()` + `clearTextureAtlas()` + 清空 scrollback
2. **非活跃标签自动缩减 scrollback** — 后台标签从 10000 行缩到 1000 行，切回恢复（TODO 已记录）
3. **定期触发 GC** — 在 WebKitGTK 中调用 `window.gc()`（如果可用）
4. **考虑关闭 WebGL** — xterm.js 使用 canvas 2D 渲染替代 WebGL，减少纹理缓存
5. **退出时杀子进程树** — 见上文「退出后 WebKit 孤儿进程」

### 中期方案

5. **Tauri Chromium 后端** — Tauri 2 正在开发 Linux 上的 Chromium (CEF) 后端支持，切换后可根治。目前为实验性功能，需关注 Tauri 官方进展。

### 长期方案（备选）

6. **迁移到 Electron** — 使用 Chromium 内核，内存管理更优，但包体增大 ~150MB
7. **原生 UI 重写** — 使用 GTK4/Qt/Slint 等原生框架，彻底消除 WebView 内存问题，但开发成本最高

## 参考

- [WebKit bmalloc 源码](https://github.com/nicehash/nicehash.github.io) — WebKit 的内存分配器实现
- [Tauri Linux WebView 讨论](https://github.com/nicehash/nicehash.github.io) — Tauri 社区关于 WebKitGTK 内存的讨论
- [xterm.js 内存优化](https://github.com/xtermjs/xterm.js/wiki/Performance) — xterm.js 官方性能指南
