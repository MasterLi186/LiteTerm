# Native Run Script Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a root-level `run-native.sh` that launches the native LiteTerm binary without changing the existing `run.sh` GuiShell launcher.

**Architecture:** The new Bash script resolves paths relative to its own location, validates the native binary, stops only exact-name `liteterm-native` processes, and replaces itself with the binary while forwarding arguments. An isolated Shell test copies the launcher into a temporary fake repository so behavior can be verified without starting the real GUI.

**Tech Stack:** Bash, standard Linux process tools, POSIX temporary test fixtures.

---

## File Structure

- Create `run-native.sh`: native binary launcher.
- Create `tests/run_native_script_test.sh`: isolated launcher behavior test.
- Preserve `run.sh` byte-for-byte.

### Task 1: Add the Native Launcher with a Failing Test

**Files:**
- Create: `tests/run_native_script_test.sh`
- Create: `run-native.sh`
- Verify unchanged: `run.sh`

- [ ] **Step 1: Record the old launcher checksum**

Run:

```bash
sha256sum run.sh
```

Expected:

```text
f7d1b7b264548f8e1b968c781c0fa38529a92b24e2bd894b6dcea3b080b1ff6e  run.sh
```

- [ ] **Step 2: Write the failing isolated launcher test**

Create `tests/run_native_script_test.sh`:

```bash
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
```

- [ ] **Step 3: Run the test and verify RED**

Run:

```bash
bash tests/run_native_script_test.sh
```

Expected: exit 1 with `run-native.sh 不存在或不可执行`.

- [ ] **Step 4: Implement the launcher**

Create `run-native.sh`:

```bash
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
```

Make both new scripts executable:

```bash
chmod +x run-native.sh tests/run_native_script_test.sh
```

- [ ] **Step 5: Verify GREEN and shell syntax**

Run:

```bash
bash tests/run_native_script_test.sh
bash -n run-native.sh tests/run_native_script_test.sh
test -x run-native.sh
test -x tests/run_native_script_test.sh
```

Expected: every command exits 0.

- [ ] **Step 6: Prove the legacy launcher was preserved**

Run:

```bash
sha256sum run.sh
git diff --exit-code -- run.sh
git diff --check -- run-native.sh tests/run_native_script_test.sh
```

Expected: the checksum remains `f7d1b7b264548f8e1b968c781c0fa38529a92b24e2bd894b6dcea3b080b1ff6e`, `run.sh` has no diff, and the new files have no whitespace errors.

- [ ] **Step 7: Commit only the two new scripts**

```bash
git add run-native.sh tests/run_native_script_test.sh
git commit -m "feat: 添加 Native 独立启动脚本"
```
