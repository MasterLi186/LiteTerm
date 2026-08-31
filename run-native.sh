#!/usr/bin/env bash
set -euo pipefail
cd -- "$(dirname -- "$0")"

BINARY="native-prototype/target/debug/liteterm-native"

native_needs_build() {
    if [ ! -x "$BINARY" ]; then
        return 0
    fi

    for input in native-prototype/Cargo.toml native-prototype/Cargo.lock; do
        if [ -e "$input" ] && [ "$input" -nt "$BINARY" ]; then
            return 0
        fi
    done

    if [ ! -d native-prototype/src ] || [ ! -r native-prototype/src ] || [ ! -x native-prototype/src ]; then
        echo "Native 源码目录缺失或不可遍历：native-prototype/src" >&2
        return 0
    fi

    if [ ! -d native-prototype/assets/icons ]; then
        echo "Native 图标目录缺失：native-prototype/assets/icons" >&2
        return 0
    fi

    if [ ! -f native-prototype/assets/icons/256x256.png ]; then
        echo "Native 窗口图标缺失：native-prototype/assets/icons/256x256.png" >&2
        return 0
    fi

    local newer_input
    if ! newer_input="$(find native-prototype/src native-prototype/assets/icons -newer "$BINARY" -print -quit)"; then
        echo "无法检查 Native 源码或图标目录的更新时间" >&2
        return 0
    fi
    if [ -n "$newer_input" ]; then
        return 0
    fi

    return 1
}

if native_needs_build; then
    echo "Native 构建产物缺失或已过期，开始构建..."
    if ! (cd -- native-prototype && cargo build --locked --bin liteterm-native); then
        echo "Native 构建失败" >&2
        exit 1
    fi
fi

if [ ! -x "$BINARY" ]; then
    echo "未找到 Native 构建产物：$BINARY" >&2
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
