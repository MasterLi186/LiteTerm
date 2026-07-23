#!/bin/bash
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
SOURCE_SCRIPT="$ROOT/run-native.sh"

if [ ! -x "$SOURCE_SCRIPT" ]; then
    echo "run-native.sh 不存在或不可执行" >&2
    exit 1
fi

TEST_ROOT="$(mktemp -d)"
trap 'rm -rf "$TEST_ROOT"' EXIT
mkdir -p "$TEST_ROOT/native-prototype/target/debug" "$TEST_ROOT/test-bin"
cp "$SOURCE_SCRIPT" "$TEST_ROOT/run-native.sh"

cat > "$TEST_ROOT/test-bin/pgrep" <<'EOF'
#!/bin/sh
exit 1
EOF
chmod +x "$TEST_ROOT/test-bin/pgrep"

cat > "$TEST_ROOT/native-prototype/target/debug/liteterm-native" <<'EOF'
#!/bin/bash
{
    printf 'cwd=%s\n' "$PWD"
    printf 'arg=%s\n' "$@"
} > "$RUN_NATIVE_TEST_OUTPUT"
EOF
chmod +x "$TEST_ROOT/native-prototype/target/debug/liteterm-native"

OUTPUT="$TEST_ROOT/output"
(
    cd /
    PATH="$TEST_ROOT/test-bin:$PATH" \
        RUN_NATIVE_TEST_OUTPUT="$OUTPUT" \
        "$TEST_ROOT/run-native.sh" first "two words"
)

grep -Fxq "cwd=$TEST_ROOT" "$OUTPUT"
grep -Fxq "arg=first" "$OUTPUT"
grep -Fxq "arg=two words" "$OUTPUT"

rm "$TEST_ROOT/native-prototype/target/debug/liteterm-native"
if PATH="$TEST_ROOT/test-bin:$PATH" "$TEST_ROOT/run-native.sh" \
    >"$TEST_ROOT/missing.out" 2>&1; then
    echo "缺少二进制时脚本意外成功" >&2
    exit 1
fi
grep -Fq "未找到 Native 构建产物" "$TEST_ROOT/missing.out"
