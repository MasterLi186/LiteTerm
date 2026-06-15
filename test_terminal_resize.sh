#!/bin/bash
# Terminal resize test: validates initial display and maximize/restore behavior
# Prerequisites: wmctrl, xdotool, scrot or import (ImageMagick)
set -e
cd "$(dirname "$0")"

BINARY="src-tauri/target/debug/guishell-tauri"
SCREENSHOT_DIR="/tmp/guishell_test_$(date +%s)"
mkdir -p "$SCREENSHOT_DIR"
PASS=true

screenshot() {
    local name="$1"
    local path="$SCREENSHOT_DIR/${name}.png"
    sleep 1
    import -window root "$path" 2>/dev/null
    echo "$path"
}

get_wid() {
    local wid=""
    for i in $(seq 1 20); do
        wid=$(xdotool search --name "GuiShell" 2>/dev/null | tail -1)
        if [ -n "$wid" ]; then echo "$wid"; return 0; fi
        sleep 0.5
    done
    echo ""
}

focus_window() {
    local wid="$1"
    wmctrl -i -r "$wid" -b remove,hidden 2>/dev/null
    xdotool windowactivate "$wid" windowraise "$wid" windowfocus "$wid" 2>/dev/null
    sleep 0.5
}

# Build first
echo "=== Building ==="
npm run build --silent 2>&1 | tail -1
cd src-tauri && cargo build 2>&1 | tail -3 && cd ..

# Kill any existing instance
pkill -f guishell-tauri 2>/dev/null || true
sleep 1

echo ""
echo "=== Test: Terminal Initial Display + Maximize/Restore ==="
echo "Screenshots will be saved to: $SCREENSHOT_DIR"
echo ""

# Step 1: Launch app
echo "[Step 1] Launching GuiShell..."
"$BINARY" &
APP_PID=$!
sleep 2

WID=$(get_wid)
if [ -z "$WID" ]; then
    echo "FAIL: Could not find GuiShell window"
    kill "$APP_PID" 2>/dev/null
    exit 1
fi
echo "  Window found: $WID"

# Position window
focus_window "$WID"
wmctrl -i -r "$WID" -e 0,50,50,1200,800 2>/dev/null
sleep 3

# Step 2: Screenshot initial state
echo "[Step 2] Capturing initial state..."
S1=$(screenshot "01_initial")
echo "  Saved: $S1"

# Step 3: Type 'ls' + Enter via xdotool
echo "[Step 3] Typing 'ls' + Enter..."
focus_window "$WID"
xdotool type --delay 100 "ls"
sleep 0.3
xdotool key Return
sleep 2

S2=$(screenshot "02_after_ls")
echo "  Saved: $S2"

# Step 4: Maximize
echo "[Step 4] Maximizing window..."
wmctrl -i -r "$WID" -b add,maximized_vert,maximized_horz 2>/dev/null
sleep 3

S3=$(screenshot "03_maximized")
echo "  Saved: $S3"

# Step 5: Type 'ls' + Enter in maximized state
echo "[Step 5] Typing 'ls' + Enter (maximized)..."
focus_window "$WID"
xdotool type --delay 100 "ls"
sleep 0.3
xdotool key Return
sleep 2

S4=$(screenshot "04_maximized_after_ls")
echo "  Saved: $S4"

# Step 6: Restore (un-maximize)
echo "[Step 6] Restoring window..."
wmctrl -i -r "$WID" -b remove,maximized_vert,maximized_horz 2>/dev/null
sleep 3

S5=$(screenshot "05_restored")
echo "  Saved: $S5"

# Step 7: Type 'echo OK' + Enter to verify terminal still works after restore
echo "[Step 7] Typing 'echo RESIZE_TEST_OK' + Enter (after restore)..."
focus_window "$WID"
xdotool type --delay 100 "echo RESIZE_TEST_OK"
sleep 0.3
xdotool key Return
sleep 2

S6=$(screenshot "06_restored_after_echo")
echo "  Saved: $S6"

# Cleanup
echo ""
echo "[Cleanup] Closing GuiShell..."
kill "$APP_PID" 2>/dev/null
wait "$APP_PID" 2>/dev/null

echo ""
echo "=== Screenshots ==="
ls -la "$SCREENSHOT_DIR/"
echo ""
echo "Review screenshots manually:"
echo "  Initial (should show terminal prompt):  $S1"
echo "  After ls (should show file listing):    $S2"
echo "  Maximized (layout intact):              $S3"
echo "  Maximized after ls (wider listing):     $S4"
echo "  Restored (sidebar+terminal intact):     $S5"
echo "  Restored after echo (RESIZE_TEST_OK):   $S6"
echo ""
echo "All screenshots saved to $SCREENSHOT_DIR"
