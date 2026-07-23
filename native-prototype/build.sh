#!/bin/bash
set -euo pipefail
cd "$(dirname "$0")"

echo "=== Native Rust 编译 ==="
cargo build

echo "=== Native Clippy 静态检查 ==="
cargo clippy --all-targets
echo "  ✓ Native Clippy 通过"

echo "=== Native 单元测试 ==="
cargo test
echo "  ✓ Native 测试通过"

BINARY="target/debug/liteterm-native"
if [ ! -x "$BINARY" ]; then
    echo "未找到 Native 构建产物：$BINARY" >&2
    exit 1
fi

ls -lh "$BINARY"
echo "=== Native 构建完成 ==="
