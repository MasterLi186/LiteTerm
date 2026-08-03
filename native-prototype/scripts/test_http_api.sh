#!/usr/bin/env bash
set -euo pipefail

CONFIG_DIR="${XDG_CONFIG_HOME:-$HOME/.config}/guishell"
TOKEN_FILE="$CONFIG_DIR/native-api-token"
PORT_FILE="$CONFIG_DIR/native-api-port"
REQUEST_TIMEOUT=8
TAB_ID=""

fail() {
  printf 'Native HTTP API test: %s\n' "$*" >&2
  exit 1
}

for command in curl jq; do
  command -v "$command" >/dev/null 2>&1 || fail "缺少依赖：$command"
done

[[ -r "$TOKEN_FILE" ]] ||
  fail "找不到 $TOKEN_FILE；请先手动启动 Native 应用"
[[ -r "$PORT_FILE" ]] ||
  fail "找不到 $PORT_FILE；请先手动启动 Native 应用"

TOKEN=$(tr -d '\r\n' < "$TOKEN_FILE")
[[ "$TOKEN" =~ ^[[:xdigit:]]{64}$ ]] || fail "Native token 格式无效"

PORT=$(jq -er '
  .port
  | select(type == "number" and floor == . and . >= 1 and . <= 65535)
' "$PORT_FILE") || fail "Native port discovery JSON 无效"

BASE="http://127.0.0.1:$PORT/api/v1"

cleanup() {
  if [[ -n "$TAB_ID" ]]; then
    curl --silent --show-error --max-time "$REQUEST_TIMEOUT" \
      -X DELETE \
      -H "Authorization: Bearer $TOKEN" \
      "$BASE/tabs/$TAB_ID" >/dev/null ||
      printf 'Native HTTP API test: 无法清理脚本创建的标签 %s\n' "$TAB_ID" >&2
  fi
}
trap cleanup EXIT

AUTH_HEADER="Authorization: Bearer $TOKEN"

unauthorized_status=$(
  curl --silent --show-error --max-time "$REQUEST_TIMEOUT" \
    -o /dev/null -w '%{http_code}' "$BASE/tabs"
)
[[ "$unauthorized_status" == "401" ]] ||
  fail "无认证请求预期返回 401，实际为 $unauthorized_status"
printf 'ok - Bearer 认证\n'

tabs=$(
  curl --silent --show-error --fail-with-body --max-time "$REQUEST_TIMEOUT" \
    -H "$AUTH_HEADER" "$BASE/tabs"
)
jq -e 'type == "array"' <<<"$tabs" >/dev/null ||
  fail "GET /tabs 未返回数组"
printf 'ok - 列出标签\n'

opened=$(
  curl --silent --show-error --fail-with-body --max-time "$REQUEST_TIMEOUT" \
    -X POST -H "$AUTH_HEADER" "$BASE/tabs/local"
)
TAB_ID=$(jq -er '.id | select(type == "string" and length > 0)' <<<"$opened") ||
  fail "POST /tabs/local 未返回标签 ID"
printf 'ok - 打开本地终端 %s\n' "$TAB_ID"

initial=$(
  curl --silent --show-error --fail-with-body --max-time "$REQUEST_TIMEOUT" \
    -H "$AUTH_HEADER" "$BASE/tabs/$TAB_ID/read?limit=262144&raw=false"
)
CURSOR=$(jq -er '.cursor | select(type == "number")' <<<"$initial") ||
  fail "初次读取未返回 cursor"
STREAM_ID=$(jq -er '.stream_id | select(type == "number")' <<<"$initial") ||
  fail "初次读取未返回 stream_id"
PANE_ID=$(jq -er '.pane_id | select(type == "string" and length > 0)' <<<"$initial") ||
  fail "初次读取未返回 pane_id"
PANE_ID_ENCODED=$(jq -rn --arg value "$PANE_ID" '$value | @uri')

MARKER="NATIVE_HTTP_API_OK_$$"
command_text="printf '%s\\n' '$MARKER'"
command_text+=$'\n'
payload=$(jq -nc --arg data "$command_text" '{data:$data}')
written=$(
  curl --silent --show-error --fail-with-body --max-time "$REQUEST_TIMEOUT" \
    -X POST \
    -H "$AUTH_HEADER" \
    -H 'Content-Type: application/json' \
    -d "$payload" \
    "$BASE/tabs/$TAB_ID/write?pane_id=$PANE_ID_ENCODED"
)
jq -e '.ok == true' <<<"$written" >/dev/null ||
  fail "POST /write 未返回 ok"
printf 'ok - 写入终端\n'

found=false
for _ in {1..20}; do
  output=$(
    curl --silent --show-error --fail-with-body --max-time "$REQUEST_TIMEOUT" \
      -H "$AUTH_HEADER" \
      "$BASE/tabs/$TAB_ID/read?pane_id=$PANE_ID_ENCODED&cursor=$CURSOR&stream_id=$STREAM_ID&limit=262144&raw=false"
  )
  CURSOR=$(jq -er '.cursor | select(type == "number")' <<<"$output") ||
    fail "增量读取未返回 cursor"
  STREAM_ID=$(jq -er '.stream_id | select(type == "number")' <<<"$output") ||
    fail "增量读取未返回 stream_id"
  if jq -e --arg marker "$MARKER" '.data | contains($marker)' <<<"$output" >/dev/null; then
    found=true
    break
  fi
  sleep 0.1
done
[[ "$found" == "true" ]] || fail "未在终端输出中找到测试标记"
printf 'ok - 增量读取终端输出\n'

closed=$(
  curl --silent --show-error --fail-with-body --max-time "$REQUEST_TIMEOUT" \
    -X DELETE -H "$AUTH_HEADER" "$BASE/tabs/$TAB_ID"
)
jq -e '.ok == true' <<<"$closed" >/dev/null ||
  fail "DELETE /tabs/:id 未返回 ok"
TAB_ID=""
printf 'ok - 关闭脚本创建的标签\n'
printf 'Native HTTP API smoke test passed\n'
