# Native Build Isolation and Alpha Color Fix Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add an isolated Native build command, prevent stale Native launches, and restore the intended translucent colors across the Native UI.

**Architecture:** `native-prototype/build.sh` owns all Native Cargo checks and artifact validation. The root `run-native.sh` compares the Native binary timestamp with Cargo manifests and source files, invokes that build script when needed, and only replaces a running Native process after a successful build. Native UI code keeps the RGBA values copied from `main`, but constructs them with egui's unmultiplied-alpha API.

**Tech Stack:** Bash, Cargo, Rust, shell integration tests

---

### Task 1: Cover Native build freshness behavior

**Files:**
- Modify: `tests/run_native_script_test.sh`
- Test: `tests/run_native_script_test.sh`

- [ ] **Step 1: Replace the single launcher fixture with isolated scenario fixtures**

Replace `tests/run_native_script_test.sh` with a fixture that creates one temporary repository per scenario. Use a fake builder which records its invocation and installs this executable:

```bash
#!/bin/bash
{
    printf 'cwd=%s\n' "$PWD"
    printf 'arg=%s\n' "$@"
} > "$RUN_NATIVE_TEST_OUTPUT"
```

The fixture must also create:

```text
native-prototype/Cargo.toml
native-prototype/Cargo.lock
native-prototype/src/main.rs
native-prototype/build.sh
test-bin/pgrep
```

- [ ] **Step 2: Add missing, fresh, and stale artifact assertions**

For each scenario, launch from `/` and set `RUN_NATIVE_TEST_OUTPUT` plus
`RUN_NATIVE_TEST_BUILD_MARKER`. Assert:

```bash
# missing
test -f "$repo/build-called"
grep -Fxq "cwd=$repo" "$repo/output"
grep -Fxq "arg=first" "$repo/output"
grep -Fxq "arg=two words" "$repo/output"

# fresh: create source first and executable second
test ! -e "$repo/build-called"

# stale source
touch -t 202001010000 "$repo/native-prototype/target/debug/liteterm-native"
touch -t 202101010000 "$repo/native-prototype/src/main.rs"
test -f "$repo/build-called"

# stale manifest
touch -t 202001010000 "$repo/native-prototype/target/debug/liteterm-native"
touch -t 202101010000 "$repo/native-prototype/Cargo.toml"
test -f "$repo/build-called"
```

- [ ] **Step 3: Run the launcher test and verify RED**

Run:

```bash
bash tests/run_native_script_test.sh
```

Expected: FAIL because the current `run-native.sh` exits when the binary is missing instead of invoking a builder.

### Task 2: Add the isolated Native build entry point

**Files:**
- Create: `native-prototype/build.sh`

- [ ] **Step 1: Implement the build pipeline**

Create `native-prototype/build.sh` with:

```bash
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
```

Run `chmod +x native-prototype/build.sh`. The script must not clean caches or invoke root build/launch scripts.

- [ ] **Step 2: Validate shell syntax and executable mode**

Run:

```bash
bash -n native-prototype/build.sh
test -x native-prototype/build.sh
```

Expected: both commands exit 0.

### Task 3: Rebuild automatically before Native launch

**Files:**
- Modify: `run-native.sh`
- Test: `tests/run_native_script_test.sh`

- [ ] **Step 1: Add a freshness predicate**

Use this launcher structure:

```bash
#!/bin/bash
set -e
cd "$(dirname "$0")"

BINARY="native-prototype/target/debug/liteterm-native"
BUILD_SCRIPT="native-prototype/build.sh"

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
    if [ ! -x "$BUILD_SCRIPT" ]; then
        echo "Native 构建脚本不存在或不可执行：$BUILD_SCRIPT" >&2
        exit 1
    fi
    echo "Native 构建产物缺失或已过期，开始构建..."
    "$BUILD_SCRIPT"
fi

if [ ! -x "$BINARY" ]; then
    echo "未找到 Native 构建产物：$BINARY" >&2
    exit 1
fi
```

- [ ] **Step 2: Preserve process replacement after the build gate**

Append this existing process replacement block after the build gate:

```bash
OLD_PID="$(pgrep -x 'liteterm-native' 2>/dev/null || true)"
if [ -n "$OLD_PID" ]; then
    echo "关闭旧 Native 进程: $OLD_PID"
    kill $OLD_PID 2>/dev/null || true
    sleep 0.5
fi

echo "启动 LiteTerm Native..."
exec "$BINARY" "$@"
```

This ensures a failed build cannot close the currently running Native process.

- [ ] **Step 3: Run shell tests and verify GREEN**

Run:

```bash
bash -n run-native.sh tests/run_native_script_test.sh
bash tests/run_native_script_test.sh
```

Expected: all commands exit 0.

### Task 4: Fix unmultiplied alpha colors across Native

**Files:**
- Modify: `native-prototype/src/sidebar.rs`
- Modify: `native-prototype/src/tab_bar.rs`
- Test: `tests/native_color_api_test.sh`

- [ ] **Step 1: Add a failing source-level regression test**

Create `tests/native_color_api_test.sh`:

```bash
#!/bin/bash
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"

if rg -n 'Color32::from_rgba_premultiplied' \
    "$ROOT/native-prototype/src" --glob '*.rs'; then
    echo "Native UI 仍在使用预乘 Alpha API" >&2
    exit 1
fi
```

Make it executable.

- [ ] **Step 2: Run the color regression test and verify RED**

Run:

```bash
bash tests/native_color_api_test.sh
```

Expected: FAIL and list existing calls in `sidebar.rs` and `tab_bar.rs`.

- [ ] **Step 3: Replace the incorrect constructors**

Change every existing Native UI call of:

```rust
egui::Color32::from_rgba_premultiplied(r, g, b, a)
```

to:

```rust
egui::Color32::from_rgba_unmultiplied(r, g, b, a)
```

Keep all `r`, `g`, `b`, and `a` values unchanged so they continue to match the CSS RGBA values on `main`.

- [ ] **Step 4: Run the color regression test and verify GREEN**

Run:

```bash
bash tests/native_color_api_test.sh
```

Expected: exit 0 with no matching premultiplied-alpha calls.

### Task 5: Verify isolation and build the real Native application

**Files:**
- Verify unchanged: `build.sh`
- Verify unchanged: `run.sh`
- Verify artifact: `native-prototype/target/debug/liteterm-native`

- [ ] **Step 1: Verify old GuiShell scripts are byte-identical**

Run:

```bash
sha256sum build.sh run.sh
```

Expected:

```text
69e913332c3b14111a62f5843cf35dd6751f0f566dc941efc1218364350a2a5e  build.sh
f7d1b7b264548f8e1b968c781c0fa38529a92b24e2bd894b6dcea3b080b1ff6e  run.sh
```

- [ ] **Step 2: Run the real isolated build**

Run:

```bash
./native-prototype/build.sh
```

Expected: Cargo build, normal Clippy, and all Native tests pass; existing warnings may be printed.

- [ ] **Step 3: Verify the rebuilt binary includes file-manager UI**

Run:

```bash
strings native-prototype/target/debug/liteterm-native |
  rg '隐藏文件管理器|显示文件管理器|SFTP worker'
```

Expected: at least one file-manager/SFTP string is present.

- [ ] **Step 4: Smoke-test the launcher without replacing GuiShell**

Run:

```bash
timeout 8s ./run-native.sh
```

Expected: the Native process starts; exit 124 is acceptable because `timeout` intentionally stops the GUI. The old `run.sh` process is not targeted.

### Task 6: Commit only the requested implementation

**Files:**
- Add: `native-prototype/build.sh`
- Modify: `run-native.sh`
- Modify: `tests/run_native_script_test.sh`
- Modify: `native-prototype/src/sidebar.rs`
- Modify: `native-prototype/src/tab_bar.rs`
- Add: `tests/native_color_api_test.sh`

- [ ] **Step 1: Review the scoped diff**

Run:

```bash
git diff -- native-prototype/build.sh run-native.sh tests/run_native_script_test.sh \
  native-prototype/src/sidebar.rs native-prototype/src/tab_bar.rs \
  tests/native_color_api_test.sh
git status --short
```

Confirm unrelated Native/Tauri worktree changes remain unstaged.

- [ ] **Step 2: Commit the implementation**

Run:

```bash
git add native-prototype/build.sh run-native.sh tests/run_native_script_test.sh
git add native-prototype/src/sidebar.rs native-prototype/src/tab_bar.rs
git add tests/native_color_api_test.sh
git commit -m "fix: 隔离 Native 构建并修正半透明配色"
```

Expected: the commit contains exactly the six implementation files.
