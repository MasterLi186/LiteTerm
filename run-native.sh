#!/bin/bash
set -e
cd "$(dirname "$0")"

BINARY="native-prototype/target/debug/liteterm-native"

if [ ! -x "$BINARY" ]; then
    echo "未找到 Native 构建产物：$BINARY" >&2
    echo "请先构建 liteterm-native。" >&2
    exit 1
fi

OLD_PID="$(pgrep -x 'liteterm-native' 2>/dev/null || true)"
if [ -n "$OLD_PID" ]; then
    echo "关闭旧 Native 进程: $OLD_PID"
    kill $OLD_PID 2>/dev/null || true
    sleep 0.5
fi

echo "启动 LiteTerm Native..."
exec "$BINARY" "$@"
