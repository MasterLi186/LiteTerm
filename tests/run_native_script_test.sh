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
mkdir -p "$TEST_ROOT/test-bin"

cat > "$TEST_ROOT/test-bin/pgrep" <<'EOF'
#!/bin/sh
exit 1
EOF
chmod +x "$TEST_ROOT/test-bin/pgrep"

install_binary() {
    local repo="$1"
    mkdir -p "$repo/native-prototype/target/debug"
    cat > "$repo/native-prototype/target/debug/liteterm-native" <<'EOF'
#!/bin/bash
{
    printf 'cwd=%s\n' "$PWD"
    printf 'arg=%s\n' "$@"
} > "$RUN_NATIVE_TEST_OUTPUT"
EOF
    chmod +x "$repo/native-prototype/target/debug/liteterm-native"
}

create_repo() {
    local name="$1"
    REPO="$TEST_ROOT/$name"
    mkdir -p "$REPO/native-prototype/src"
    cp "$SOURCE_SCRIPT" "$REPO/run-native.sh"
    : > "$REPO/native-prototype/Cargo.toml"
    : > "$REPO/native-prototype/Cargo.lock"
    : > "$REPO/native-prototype/src/main.rs"

    cat > "$REPO/native-prototype/build.sh" <<'EOF'
#!/bin/bash
set -e
: > "$RUN_NATIVE_TEST_BUILD_MARKER"
mkdir -p "native-prototype/target/debug"
cat > "native-prototype/target/debug/liteterm-native" <<'BINARY'
#!/bin/bash
{
    printf 'cwd=%s\n' "$PWD"
    printf 'arg=%s\n' "$@"
} > "$RUN_NATIVE_TEST_OUTPUT"
BINARY
chmod +x "native-prototype/target/debug/liteterm-native"
EOF
    chmod +x "$REPO/native-prototype/build.sh"
}

run_repo() {
    local repo="$1"
    shift
    (
        cd /
        PATH="$TEST_ROOT/test-bin:$PATH" \
            RUN_NATIVE_TEST_OUTPUT="$repo/output" \
            RUN_NATIVE_TEST_BUILD_MARKER="$repo/build-called" \
            "$repo/run-native.sh" "$@"
    )
}

create_repo "missing"
run_repo "$REPO" first "two words"
test -f "$REPO/build-called"
grep -Fxq "cwd=$REPO" "$REPO/output"
grep -Fxq "arg=first" "$REPO/output"
grep -Fxq "arg=two words" "$REPO/output"

create_repo "fresh"
install_binary "$REPO"
run_repo "$REPO"
test ! -e "$REPO/build-called"

create_repo "stale-source"
install_binary "$REPO"
touch -t 202001010000 "$REPO/native-prototype/target/debug/liteterm-native"
touch -t 202101010000 "$REPO/native-prototype/src/main.rs"
run_repo "$REPO"
test -f "$REPO/build-called"

create_repo "stale-manifest"
install_binary "$REPO"
touch -t 202001010000 "$REPO/native-prototype/target/debug/liteterm-native"
touch -t 202101010000 "$REPO/native-prototype/Cargo.toml"
run_repo "$REPO"
test -f "$REPO/build-called"
