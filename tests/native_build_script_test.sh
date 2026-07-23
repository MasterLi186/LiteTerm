#!/bin/bash
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
SOURCE_SCRIPT="$ROOT/native-prototype/build.sh"

if [ ! -x "$SOURCE_SCRIPT" ]; then
    echo "native-prototype/build.sh 不存在或不可执行" >&2
    exit 1
fi

TEST_ROOT="$(mktemp -d)"
trap 'rm -rf "$TEST_ROOT"' EXIT
mkdir -p "$TEST_ROOT/native-prototype" "$TEST_ROOT/test-bin"
cp "$SOURCE_SCRIPT" "$TEST_ROOT/native-prototype/build.sh"

cat > "$TEST_ROOT/test-bin/cargo" <<'EOF'
#!/bin/bash
set -e
printf 'cwd=%s command=%s\n' "$PWD" "$*" >> "$NATIVE_BUILD_TEST_LOG"
if [ "$1" = "build" ]; then
    mkdir -p target/debug
    : > target/debug/liteterm-native
    chmod +x target/debug/liteterm-native
fi
EOF
chmod +x "$TEST_ROOT/test-bin/cargo"

(
    cd /
    PATH="$TEST_ROOT/test-bin:$PATH" \
        NATIVE_BUILD_TEST_LOG="$TEST_ROOT/cargo.log" \
        "$TEST_ROOT/native-prototype/build.sh"
)

grep -Fxq "cwd=$TEST_ROOT/native-prototype command=build" "$TEST_ROOT/cargo.log"
grep -Fxq "cwd=$TEST_ROOT/native-prototype command=clippy --all-targets" \
    "$TEST_ROOT/cargo.log"
grep -Fxq "cwd=$TEST_ROOT/native-prototype command=test" "$TEST_ROOT/cargo.log"
test -x "$TEST_ROOT/native-prototype/target/debug/liteterm-native"
