# HTTP API 自动化交互 — 功能说明

## 功能简介

LiteTerm HTTP API 允许外部工具通过 HTTP 请求程序化地操作终端标签页，实现自动化开发调试。

典型场景：Claude Code 通过 `curl` 命令在 LiteTerm 中自动执行编译、测试、日志分析，无需人工切换窗口手动操作。

## 工作原理

LiteTerm 启动时在后台开启一个 HTTP 服务（默认 `127.0.0.1:19526`），外部工具通过 REST API 与之交互。

### 核心操作

| 操作 | 说明 |
|------|------|
| 列出标签页 | 查看当前所有打开的终端 |
| 打开本地终端 | 创建新的本地 shell 标签页 |
| 打开 SSH 连接 | 创建新的 SSH 远程连接标签页 |
| 切换焦点 | 切换前端显示的活跃标签页 |
| 写入数据 | 向终端发送命令或控制字符（回车、Ctrl+C 等） |
| 读取输出 | 获取终端的文本输出（支持增量读取） |
| 关闭标签页 | 关闭终端并清理资源 |

### 自动化流程示意

```
外部工具                    LiteTerm
  │                           │
  │── POST /tabs/local ──────→│  打开终端
  │←── {id: "abc"} ──────────│
  │                           │
  │── POST /tabs/abc/write ──→│  发送 "make build\n"
  │←── {ok: true} ───────────│
  │                           │
  │      (等待命令执行)        │
  │                           │
  │── GET /tabs/abc/read ────→│  读取编译输出
  │←── {data: "...", cursor} ─│
  │                           │
  │  分析输出，发现错误         │
  │                           │
  │── POST /tabs/abc/write ──→│  发送修复命令
  │      ...循环...            │
```

### 输出捕获

终端输出自动存入 1MB 环形缓冲区。API 读取支持增量模式：

1. 首次调用 `read` 不带 cursor → 返回缓冲区全部内容 + cursor
2. 后续调用带上 cursor → 只返回新增内容 + 新 cursor
3. 长时间不读导致旧数据被覆盖 → 返回当前内容 + `truncated: true`

输出默认过滤 ANSI 转义码（颜色、光标移动等），返回纯文本，方便 AI 分析。

## 安全

- 仅监听 `127.0.0.1`，外部网络无法访问
- 每次启动生成随机 token，存于 `~/.config/guishell/api_token`
- 实际端口写入 `~/.config/guishell/api_port`（JSON: `{"port": N, "pid": N}`）
- 所有请求需携带 `Authorization: Bearer <token>`

### 多实例支持

默认端口 19526，如果被占用自动尝试 19527、19528...（最多 10 个）。也可通过环境变量强制指定：
```bash
LITETERM_API_PORT=19600 ./run.sh
```

## 快速开始

```bash
# 1. 确保 LiteTerm 已启动
# 2. 获取端口和 token
PORT=$(cat ~/.config/guishell/api_port | jq -r .port)
TOKEN=$(cat ~/.config/guishell/api_token)
BASE="http://127.0.0.1:$PORT/api/v1"

# 3. 打开终端
ID=$(curl -s -X POST -H "Authorization: Bearer $TOKEN" $BASE/tabs/local | jq -r .id)

# 4. 发送命令
curl -s -X POST -H "Authorization: Bearer $TOKEN" \
  -H "Content-Type: application/json" \
  -d '{"data": "echo hello\n"}' \
  "$BASE/tabs/$ID/write"

# 5. 读取输出
sleep 1
curl -s -H "Authorization: Bearer $TOKEN" "$BASE/tabs/$ID/read" | jq -r .data
```

## 限制

- 不自动判断命令执行完毕，需调用方自行控制读取时机
- 缓冲区 1MB 上限，超长输出会丢失早期数据
- 第一期不支持 resize、SFTP、串口、隧道等操作

## 参考文档

- [API 接口详情](../api/http-api.md)
- [设计文档](../superpowers/specs/2026-07-10-http-api-design.md)
- [测试计划](../testing/http-api-test-plan.md)
