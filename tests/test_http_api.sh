#!/bin/bash
# LiteTerm HTTP API 集成测试
# 前置条件: LiteTerm 已启动并监听 127.0.0.1:19526

set -euo pipefail

BASE="http://127.0.0.1:19526/api/v1"
TOKEN_FILE="$HOME/.config/guishell/api_token"
PASS=0
FAIL=0
CREATED_TABS=()

# 颜色
GREEN='\033[0;32m'
RED='\033[0;31m'
YELLOW='\033[1;33m'
NC='\033[0m'

cleanup() {
    echo -e "\n${YELLOW}清理: 关闭测试创建的标签页...${NC}"
    for id in "${CREATED_TABS[@]}"; do
        curl -s -X DELETE -H "Authorization: Bearer $TOKEN" "$BASE/tabs/$id" > /dev/null 2>&1 || true
    done
    echo -e "\n========================================="
    echo -e "测试结果: ${GREEN}通过 $PASS${NC} / ${RED}失败 $FAIL${NC} / 总计 $((PASS + FAIL))"
    echo "========================================="
    if [ $FAIL -gt 0 ]; then exit 1; fi
}
trap cleanup EXIT

assert_eq() {
    local name="$1" expected="$2" actual="$3"
    if [ "$expected" = "$actual" ]; then
        echo -e "  ${GREEN}✓${NC} $name"
        ((PASS++))
    else
        echo -e "  ${RED}✗${NC} $name (期望: $expected, 实际: $actual)"
        ((FAIL++))
    fi
}

assert_contains() {
    local name="$1" needle="$2" haystack="$3"
    if echo "$haystack" | grep -q "$needle"; then
        echo -e "  ${GREEN}✓${NC} $name"
        ((PASS++))
    else
        echo -e "  ${RED}✗${NC} $name (输出不包含: $needle)"
        ((FAIL++))
    fi
}

assert_http_code() {
    local name="$1" expected="$2" actual="$3"
    if [ "$expected" = "$actual" ]; then
        echo -e "  ${GREEN}✓${NC} $name (HTTP $actual)"
        ((PASS++))
    else
        echo -e "  ${RED}✗${NC} $name (期望 HTTP $expected, 实际 HTTP $actual)"
        ((FAIL++))
    fi
}

# 前置检查
echo "========================================="
echo "LiteTerm HTTP API 集成测试"
echo "========================================="

if [ ! -f "$TOKEN_FILE" ]; then
    echo -e "${RED}错误: $TOKEN_FILE 不存在，LiteTerm 是否已启动？${NC}"
    exit 1
fi
TOKEN=$(cat "$TOKEN_FILE")

# 检查服务是否可达
if ! curl -s -o /dev/null -w "%{http_code}" -H "Authorization: Bearer $TOKEN" "$BASE/tabs" | grep -q "200"; then
    echo -e "${RED}错误: API 服务不可达，LiteTerm 是否已启动？${NC}"
    exit 1
fi
echo -e "${GREEN}API 服务可达${NC}\n"

# =========================================
echo "--- AUTH: 认证测试 ---"

CODE=$(curl -s -o /dev/null -w "%{http_code}" "$BASE/tabs")
assert_http_code "AUTH-01 无 token 返回 401" "401" "$CODE"

CODE=$(curl -s -o /dev/null -w "%{http_code}" -H "Authorization: Bearer wrong-token" "$BASE/tabs")
assert_http_code "AUTH-02 错误 token 返回 401" "401" "$CODE"

CODE=$(curl -s -o /dev/null -w "%{http_code}" -H "Authorization: Bearer " "$BASE/tabs")
assert_http_code "AUTH-03 空 token 返回 401" "401" "$CODE"

CODE=$(curl -s -o /dev/null -w "%{http_code}" -H "Authorization: Bearer $TOKEN" "$BASE/tabs")
assert_http_code "AUTH-04 正确 token 返回 200" "200" "$CODE"

# =========================================
echo -e "\n--- TAB: 标签页管理 ---"

# TAB-01 列出标签页
RESP=$(curl -s -H "Authorization: Bearer $TOKEN" "$BASE/tabs")
assert_contains "TAB-01 列出标签页返回数组" "\[" "$RESP"

# TAB-02 打开本地终端
RESP=$(curl -s -X POST -H "Authorization: Bearer $TOKEN" "$BASE/tabs/local")
ID=$(echo "$RESP" | jq -r .id 2>/dev/null || echo "")
assert_contains "TAB-02 打开本地终端返回 id" '"id"' "$RESP"
if [ -n "$ID" ] && [ "$ID" != "null" ]; then
    CREATED_TABS+=("$ID")
fi

# TAB-03 指定 shell 打开终端
RESP=$(curl -s -X POST -H "Authorization: Bearer $TOKEN" \
    -H "Content-Type: application/json" \
    -d '{"shell_path": "/bin/bash"}' \
    "$BASE/tabs/local")
ID2=$(echo "$RESP" | jq -r .id 2>/dev/null || echo "")
assert_contains "TAB-03 指定 shell 打开终端" '"id"' "$RESP"
if [ -n "$ID2" ] && [ "$ID2" != "null" ]; then
    CREATED_TABS+=("$ID2")
fi

# TAB-04 无效 shell
RESP=$(curl -s -o /dev/null -w "%{http_code}" -X POST -H "Authorization: Bearer $TOKEN" \
    -H "Content-Type: application/json" \
    -d '{"shell_path": "/nonexistent/shell"}' \
    "$BASE/tabs/local")
assert_eq "TAB-04 无效 shell 返回错误" "1" "$([ "$RESP" != "200" ] && echo 1 || echo 0)"

# TAB-07 切换焦点
if [ -n "$ID" ] && [ "$ID" != "null" ]; then
    CODE=$(curl -s -o /dev/null -w "%{http_code}" -X PUT -H "Authorization: Bearer $TOKEN" "$BASE/tabs/$ID/focus")
    assert_http_code "TAB-07 切换焦点" "200" "$CODE"
fi

# TAB-08 焦点不存在 ID
CODE=$(curl -s -o /dev/null -w "%{http_code}" -X PUT -H "Authorization: Bearer $TOKEN" "$BASE/tabs/fake-nonexistent-id/focus")
assert_http_code "TAB-08 焦点不存在 ID 返回 404" "404" "$CODE"

# =========================================
echo -e "\n--- RW: 数据读写 ---"

if [ -n "$ID" ] && [ "$ID" != "null" ]; then
    sleep 1  # 等 shell 启动

    # RW-01 写入+读取
    curl -s -X POST -H "Authorization: Bearer $TOKEN" \
        -H "Content-Type: application/json" \
        -d '{"data": "echo LITETERM_TEST_MARKER_42\n"}' \
        "$BASE/tabs/$ID/write" > /dev/null
    sleep 1
    RESP=$(curl -s -H "Authorization: Bearer $TOKEN" "$BASE/tabs/$ID/read")
    DATA=$(echo "$RESP" | jq -r .data 2>/dev/null || echo "")
    assert_contains "RW-01 写入+读取" "LITETERM_TEST_MARKER_42" "$DATA"

    # RW-02 增量读取
    CURSOR=$(echo "$RESP" | jq -r .cursor 2>/dev/null || echo "0")
    curl -s -X POST -H "Authorization: Bearer $TOKEN" \
        -H "Content-Type: application/json" \
        -d '{"data": "echo INCREMENTAL_READ_99\n"}' \
        "$BASE/tabs/$ID/write" > /dev/null
    sleep 1
    RESP2=$(curl -s -H "Authorization: Bearer $TOKEN" "$BASE/tabs/$ID/read?cursor=$CURSOR")
    DATA2=$(echo "$RESP2" | jq -r .data 2>/dev/null || echo "")
    assert_contains "RW-02 增量读取只返回新输出" "INCREMENTAL_READ_99" "$DATA2"
    # 增量不应包含上一条命令的标记
    if echo "$DATA2" | grep -q "LITETERM_TEST_MARKER_42"; then
        echo -e "  ${RED}✗${NC} RW-02b 增量不含旧输出"
        ((FAIL++))
    else
        echo -e "  ${GREEN}✓${NC} RW-02b 增量不含旧输出"
        ((PASS++))
    fi

    # RW-06 空 data
    CODE=$(curl -s -o /dev/null -w "%{http_code}" -X POST -H "Authorization: Bearer $TOKEN" \
        -H "Content-Type: application/json" \
        -d '{"data": ""}' \
        "$BASE/tabs/$ID/write")
    assert_http_code "RW-06 空 data 写入" "200" "$CODE"
fi

# RW-04 写入不存在 ID
CODE=$(curl -s -o /dev/null -w "%{http_code}" -X POST -H "Authorization: Bearer $TOKEN" \
    -H "Content-Type: application/json" \
    -d '{"data": "test\n"}' \
    "$BASE/tabs/fake-nonexistent-id/write")
assert_http_code "RW-04 写入不存在 ID 返回 404" "404" "$CODE"

# RW-05 读取不存在 ID
CODE=$(curl -s -o /dev/null -w "%{http_code}" -H "Authorization: Bearer $TOKEN" \
    "$BASE/tabs/fake-nonexistent-id/read")
assert_http_code "RW-05 读取不存在 ID 返回 404" "404" "$CODE"

# =========================================
echo -e "\n--- E2E: 端到端测试 ---"

# E2E-01 完整流程
E2E_ID=$(curl -s -X POST -H "Authorization: Bearer $TOKEN" "$BASE/tabs/local" | jq -r .id)
if [ -n "$E2E_ID" ] && [ "$E2E_ID" != "null" ]; then
    CREATED_TABS+=("$E2E_ID")
    sleep 1
    curl -s -X POST -H "Authorization: Bearer $TOKEN" \
        -H "Content-Type: application/json" \
        -d '{"data": "echo E2E_FULL_FLOW_OK\n"}' \
        "$BASE/tabs/$E2E_ID/write" > /dev/null
    sleep 1
    E2E_DATA=$(curl -s -H "Authorization: Bearer $TOKEN" "$BASE/tabs/$E2E_ID/read" | jq -r .data)
    assert_contains "E2E-01 完整流程" "E2E_FULL_FLOW_OK" "$E2E_DATA"

    # E2E-03 中断长命令
    curl -s -X POST -H "Authorization: Bearer $TOKEN" \
        -H "Content-Type: application/json" \
        -d '{"data": "sleep 999\n"}' \
        "$BASE/tabs/$E2E_ID/write" > /dev/null
    sleep 1
    # 发送 Ctrl+C (\x03)
    curl -s -X POST -H "Authorization: Bearer $TOKEN" \
        -H "Content-Type: application/json" \
        -d "{\"data\": \"\"}" \
        "$BASE/tabs/$E2E_ID/write" > /dev/null
    sleep 1
    # 验证 shell 仍然响应
    CURSOR_E2E=$(curl -s -H "Authorization: Bearer $TOKEN" "$BASE/tabs/$E2E_ID/read" | jq -r .cursor)
    curl -s -X POST -H "Authorization: Bearer $TOKEN" \
        -H "Content-Type: application/json" \
        -d '{"data": "echo AFTER_CTRL_C\n"}' \
        "$BASE/tabs/$E2E_ID/write" > /dev/null
    sleep 1
    E2E_DATA2=$(curl -s -H "Authorization: Bearer $TOKEN" "$BASE/tabs/$E2E_ID/read?cursor=$CURSOR_E2E" | jq -r .data)
    assert_contains "E2E-03 Ctrl+C 中断后 shell 仍可用" "AFTER_CTRL_C" "$E2E_DATA2"
fi

# TAB-09 关闭标签
if [ -n "$ID2" ] && [ "$ID2" != "null" ]; then
    CODE=$(curl -s -o /dev/null -w "%{http_code}" -X DELETE -H "Authorization: Bearer $TOKEN" "$BASE/tabs/$ID2")
    assert_http_code "TAB-09 关闭标签页" "200" "$CODE"
    # 从清理列表移除已关闭的
    CREATED_TABS=("${CREATED_TABS[@]/$ID2}")
fi

# TAB-10 关闭不存在 ID
CODE=$(curl -s -o /dev/null -w "%{http_code}" -X DELETE -H "Authorization: Bearer $TOKEN" "$BASE/tabs/fake-nonexistent-id")
assert_http_code "TAB-10 关闭不存在 ID 返回 404" "404" "$CODE"

echo ""
