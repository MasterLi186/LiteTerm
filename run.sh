#!/bin/bash
set -e
cd "$(dirname "$0")"

BINARY="src-tauri/target/debug/guishell-tauri"

if [ ! -f "$BINARY" ] || [ ! -d "dist" ]; then
    echo "未找到构建产物，请先运行 ./build.sh"
    exit 1
fi

# 杀掉已有的 guishell-tauri 进程(含 WebKit 子进程)
OLD_PID=$(pgrep -f 'guishell-tauri' 2>/dev/null || true)
if [ -n "$OLD_PID" ]; then
    echo "关闭旧进程: $OLD_PID"
    kill $OLD_PID 2>/dev/null || true
    sleep 0.5
fi

# WebKitGTK 内存优化环境变量
export WEBKIT_DISABLE_DMABUF_RENDERER=1   # 禁用 DMA-BUF 硬件渲染，防止 dmabuf fd 泄漏
export WEBKIT_INSPECTOR_SERVER=127.0.0.1:9222  # 开启 WebInspector 远程调试

echo "启动 GuiShell..."
echo "  DMABUF 渲染: 已禁用"
echo "  WebInspector: http://127.0.0.1:9222"
exec "$BINARY" "$@" 2>/dev/null
