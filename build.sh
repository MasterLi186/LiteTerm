#!/bin/bash
set -e
cd "$(dirname "$0")"

echo "=== 清理缓存 ==="
rm -rf dist node_modules/.vite node_modules/.cache

echo "=== 编译前端 ==="
npx tsc --noEmit 2>&1 | tail -3
npx vite build 2>&1 | tail -5

echo "=== 编译后端 ==="
cd src-tauri && cargo build 2>&1 | tail -3 && cd ..

echo "=== 验证 ==="
ls -lh dist/assets/index-*.js dist/assets/index-*.css
ls -lh src-tauri/target/debug/guishell-tauri

echo "=== 构建完成 ==="
