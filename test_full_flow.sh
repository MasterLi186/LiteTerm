#!/bin/bash
# Full flow test: initial display, maximize/restore, SSH connect + monitor + process manager
# Usage: ./test_full_flow.sh [host] [port] [user] [password]
# If no SSH args, only tests local terminal.
set -e
cd "$(dirname "$0")"

SSH_HOST="${1:-}"
SSH_PORT="${2:-22}"
SSH_USER="${3:-}"
SSH_PASS="${4:-}"

SCREENSHOT_DIR="/tmp/guishell_fulltest_$(date +%s)"
mkdir -p "$SCREENSHOT_DIR"
FAIL=0

screenshot() {
    local name="$1"
    local path="$SCREENSHOT_DIR/${name}.png"
    sleep 1
    import -window root "$path" 2>/dev/null || true
    echo "  Screenshot: $path"
}

get_wid() {
    for i in $(seq 1 20); do
        local wid=$(xdotool search --name "GuiShell" 2>/dev/null | tail -1)
        if [ -n "$wid" ]; then echo "$wid"; return 0; fi
        sleep 0.5
    done
    echo ""
}

echo "=== GuiShell Full Flow Test ==="
echo "Screenshots: $SCREENSHOT_DIR"
echo ""

# Build
echo "[Build] Compiling..."
rm -rf dist
npm run build --silent 2>&1 | tail -1
cd src-tauri && cargo build 2>&1 | tail -1 && cd ..

# Kill existing
pkill -f guishell-tauri 2>/dev/null || true
sleep 1

# ---- Test 1: Initial Display ----
echo ""
echo "[Test 1] Initial terminal display"
./src-tauri/target/debug/guishell-tauri &
APP_PID=$!
sleep 4

WID=$(get_wid)
if [ -z "$WID" ]; then
    echo "  FAIL: Window not found"
    exit 1
fi

wmctrl -i -r "$WID" -e 0,0,0,1400,850 2>/dev/null
xdotool windowactivate "$WID" windowraise "$WID" 2>/dev/null
sleep 2
screenshot "01_initial"
echo "  VERIFY: Terminal should show bash prompt (lfl@...)"

# ---- Test 2: Maximize / Restore ----
echo ""
echo "[Test 2] Maximize then restore"
wmctrl -i -r "$WID" -b add,maximized_vert,maximized_horz 2>/dev/null
sleep 3
screenshot "02_maximized"
echo "  VERIFY: Full-screen layout, terminal fills area"

wmctrl -i -r "$WID" -b remove,maximized_vert,maximized_horz 2>/dev/null
sleep 3
xdotool windowactivate "$WID" windowraise "$WID" 2>/dev/null
sleep 1
screenshot "03_restored"
echo "  VERIFY: Layout intact — sidebar + terminal + file browser all visible, no overflow"

# ---- Test 3: SSH Connect + Monitor + Process Manager ----
if [ -n "$SSH_HOST" ] && [ -n "$SSH_USER" ]; then
    echo ""
    echo "[Test 3] SSH connection to $SSH_USER@$SSH_HOST:$SSH_PORT"
    echo "  (Requires saved connection in ~/.config/guishell/connections.toml)"
    echo "  Waiting for manual SSH connection or skipping..."
    echo ""
    echo "  NOTE: Automated SSH testing requires the connection to be saved"
    echo "  and password stored in keyring. Test manually:"
    echo "    1. Click the saved connection in sidebar"
    echo "    2. After connecting, verify:"
    echo "       - SSH terminal shows remote prompt"
    echo "       - Left sidebar shows: 已连接, IP, CPU/内存/交换 bars"
    echo "       - Process list populates (内存/CPU/命令 tabs)"
    echo "       - Network chart shows traffic"
    echo "       - Disk table shows mount points"
    echo "    3. Click any process in the sidebar list"
    echo "       - Process manager tab opens with full process table"
    echo "       - Click a row to see detail panel (PID, 位置, 工作目录, 命令行, 环境变量)"
    echo "    4. Test maximize/restore with SSH connected"
    echo "       - All panels should remain intact"
else
    echo ""
    echo "[Test 3] SSH test skipped (no host provided)"
    echo "  Usage: $0 <host> <port> <user> <password>"
fi

# ---- Cleanup ----
echo ""
echo "[Cleanup]"
kill "$APP_PID" 2>/dev/null || true
wait "$APP_PID" 2>/dev/null || true

echo ""
echo "=== Results ==="
echo "Screenshots saved to: $SCREENSHOT_DIR"
ls -1 "$SCREENSHOT_DIR"/*.png 2>/dev/null
echo ""
echo "Manual SSH verification checklist:"
echo "  [ ] SSH terminal shows remote prompt"
echo "  [ ] 左侧 已连接 + IP + CPU/内存/交换 bars"
echo "  [ ] 进程列表有数据 (内存/CPU/命令 三个 tab)"
echo "  [ ] 网络流量图有数据"
echo "  [ ] 磁盘表有挂载点"
echo "  [ ] 点击侧栏进程 → 进程管理器标签页打开"
echo "  [ ] 进程表可排序 (点击列头)"
echo "  [ ] 点击进程行 → 底部显示详情 (PID/位置/工作目录/命令行/环境变量)"
echo "  [ ] 最大化→还原 布局不错乱"
