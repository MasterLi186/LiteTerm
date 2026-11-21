#!/bin/bash
set -eo pipefail
cd "$(dirname "$0")"

echo "=== 清理缓存 ==="
rm -rf dist node_modules/.vite node_modules/.cache

echo "=== TypeScript 类型检查 ==="
npx tsc --noEmit
echo "  ✓ 无类型错误"

echo "=== 编译前端 ==="
npx vite build 2>&1 | tail -5

echo "=== Rust 编译 ==="
(cd src-tauri && cargo build 2>&1 | tail -5)

echo "=== Rust clippy 静态检查 ==="
(cd src-tauri && cargo clippy 2>&1 | tail -10)
echo "  ✓ clippy 通过"

echo "=== Rust 单元测试 ==="
# 根目录 Cargo.toml 是旧 GTK4 版本,依赖系统 gtk4(macOS/Windows 通常没有),缺失时跳过
if pkg-config --exists gtk4 2>/dev/null; then
    cargo test 2>&1 | tail -10
else
    echo "  - 跳过根目录旧 GTK4 代码测试(未找到 gtk4)"
fi
(cd src-tauri && cargo test 2>&1 | tail -10)
echo "  ✓ 测试通过"

echo "=== 验证产物 ==="
ls -lh dist/assets/index-*.js dist/assets/index-*.css
ls -lh src-tauri/target/debug/guishell-tauri

echo "=== 构建完成(全部检查通过) ==="
