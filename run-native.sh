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

    if find native-prototype/src -type f -newer "$BINARY" -print -quit |
        grep -q .; then
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
