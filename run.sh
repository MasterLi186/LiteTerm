#!/bin/bash
set -e
cd "$(dirname "$0")"

BINARY="src-tauri/target/debug/guishell-tauri"

if [ ! -f "$BINARY" ] || [ ! -d "dist" ]; then
    echo "未找到构建产物，请先运行 ./build.sh"
    exit 1
fi

echo "启动 GuiShell..."
exec "$BINARY" "$@" 2>/dev/null
