# LiteTerm HTTP API 文档

> 版本: v1
> 基础路径: `http://127.0.0.1:19526/api/v1`

## 认证

所有请求必须携带 `Authorization` 头：

```
Authorization: Bearer <token>
```

Token 位于 `~/.config/guishell/api_token`，每次 LiteTerm 启动时重新生成。

```bash
TOKEN=$(cat ~/.config/guishell/api_token)
```

认证失败返回 `401 Unauthorized`。

## 通用响应格式

成功：HTTP 2xx + JSON body
错误：HTTP 4xx/5xx + `{"error": "描述信息"}`

## 接口列表

---

### GET /tabs

列出所有打开的标签页。

**响应 200:**

```json
[
  {"id": "uuid-1", "label": "本地终端 1", "type": "local"},
  {"id": "uuid-2", "label": "root@192.168.1.1", "type": "ssh"},
  {"id": "uuid-3", "label": "/dev/ttyUSB0", "type": "serial"}
]
```

**示例:**

```bash
curl -s -H "Authorization: Bearer $TOKEN" http://127.0.0.1:19526/api/v1/tabs
```

---

### POST /tabs/local

打开一个本地终端标签页。

**请求 body (可选):**

```json
{"shell_path": "/usr/bin/fish"}
```

不传 body 或 `shell_path` 为空时使用系统默认 shell。

**响应 200:**

```json
{"id": "uuid-new", "label": "本地终端 1"}
```

**示例:**

```bash
# 默认 shell
curl -s -X POST -H "Authorization: Bearer $TOKEN" \
  http://127.0.0.1:19526/api/v1/tabs/local

# 指定 fish
curl -s -X POST -H "Authorization: Bearer $TOKEN" \
  -H "Content-Type: application/json" \
  -d '{"shell_path": "/usr/bin/fish"}' \
  http://127.0.0.1:19526/api/v1/tabs/local
```

---

### POST /tabs/ssh

打开一个 SSH 连接标签页。

**请求 body:**

```json
{
  "host": "192.168.1.1",
  "port": 22,
  "user": "root",
  "password": "secret",
  "auth_method": "keyring",
  "key_path": null,
  "proxy_jump": null
}
```

| 字段 | 类型 | 必填 | 说明 |
|------|------|------|------|
| host | string | 是 | 主机地址 |
| port | number | 否 | 端口，默认 22 |
| user | string | 是 | 用户名 |
| password | string | 否 | 密码（auth_method 为 keyring 时需要） |
| auth_method | string | 否 | `keyring`/`key`/`agent`，默认 `keyring` |
| key_path | string | 否 | 密钥路径（auth_method 为 key 时需要） |
| proxy_jump | string | 否 | ProxyJump 跳板机 |

**响应 200:**

```json
{"id": "uuid-new", "label": "root@192.168.1.1"}
```

**错误 400:**

```json
{"error": "缺少必填字段: host"}
```

**示例:**

```bash
curl -s -X POST -H "Authorization: Bearer $TOKEN" \
  -H "Content-Type: application/json" \
  -d '{"host":"192.168.1.1","port":22,"user":"root","password":"123456"}' \
  http://127.0.0.1:19526/api/v1/tabs/ssh
```

---

### PUT /tabs/:id/focus

切换活跃标签页（前端焦点跟随）。

**参数:** `:id` — 标签页 ID

**响应 200:**

```json
{"ok": true}
```

**错误 404:**

```json
{"error": "terminal not found"}
```

**示例:**

```bash
curl -s -X PUT -H "Authorization: Bearer $TOKEN" \
  http://127.0.0.1:19526/api/v1/tabs/uuid-1/focus
```

---

### POST /tabs/:id/write

向指定标签页写入数据。

**参数:** `:id` — 标签页 ID

**请求 body:**

```json
{"data": "ls -la\n"}
```

`data` 字段为要发送的文本。使用标准 JSON 转义序列：

| JSON 转义 | 含义 | 用途 |
|-----------|------|------|
| `\n` | 换行（回车） | 执行命令 |
| `` | Ctrl+C (ETX) | 中断当前命令 |
| `` | Ctrl+D (EOT) | EOF/退出 |
| `` | Ctrl+Z (SUB) | 挂起进程 |
| `\t` | Tab | 补全 |

注意：`` 等是 JSON Unicode 转义，`curl -d` 中直接写即可，JSON 解析器会自动转换为对应的控制字符字节。

**响应 200:**

```json
{"ok": true}
```

**示例:**

```bash
# 执行命令
curl -s -X POST -H "Authorization: Bearer $TOKEN" \
  -H "Content-Type: application/json" \
  -d '{"data": "ls -la\n"}' \
  http://127.0.0.1:19526/api/v1/tabs/uuid-1/write

# 发送 Ctrl+C
curl -s -X POST -H "Authorization: Bearer $TOKEN" \
  -H "Content-Type: application/json" \
  -d '{"data": ""}' \
  http://127.0.0.1:19526/api/v1/tabs/uuid-1/write
```

---

### GET /tabs/:id/read

读取指定标签页的输出。

**参数:** `:id` — 标签页 ID

**查询参数:**

| 参数 | 类型 | 说明 |
|------|------|------|
| cursor | number | 上次返回的 cursor 值，首次不传则返回缓冲区全部内容 |
| raw | bool | `true` 时 `data` 字段保留 ANSI 转义码，默认 `false`（纯文本） |

**响应 200:**

```json
{
  "data": "total 42\ndrwxr-xr-x 2 root root 4096 Jul 10 main.rs\n",
  "cursor": 12345,
  "truncated": false
}
```

| 字段 | 类型 | 说明 |
|------|------|------|
| data | string | 输出文本（默认已过滤 ANSI） |
| cursor | number | 下次请求带上此值获取增量 |
| truncated | bool | `true` 表示 cursor 过旧，部分数据已被环形缓冲区覆盖 |

**示例:**

```bash
# 首次读取（全部缓冲区内容）
curl -s -H "Authorization: Bearer $TOKEN" \
  http://127.0.0.1:19526/api/v1/tabs/uuid-1/read

# 增量读取
curl -s -H "Authorization: Bearer $TOKEN" \
  "http://127.0.0.1:19526/api/v1/tabs/uuid-1/read?cursor=12345"

# 带 ANSI 转义码的原始输出
curl -s -H "Authorization: Bearer $TOKEN" \
  "http://127.0.0.1:19526/api/v1/tabs/uuid-1/read?cursor=12345&raw=true"
```

---

### DELETE /tabs/:id

关闭标签页并清理所有相关资源（PTY、SSH 连接、缓冲区）。

**参数:** `:id` — 标签页 ID

**响应 200:**

```json
{"ok": true}
```

**示例:**

```bash
curl -s -X DELETE -H "Authorization: Bearer $TOKEN" \
  http://127.0.0.1:19526/api/v1/tabs/uuid-1
```

---

## 典型使用流程

```bash
TOKEN=$(cat ~/.config/guishell/api_token)
BASE=http://127.0.0.1:19526/api/v1

# 1. 打开终端
ID=$(curl -s -X POST -H "Authorization: Bearer $TOKEN" $BASE/tabs/local | jq -r .id)

# 2. 等 shell 启动
sleep 1

# 3. 首次读取（获取初始 prompt）
RESP=$(curl -s -H "Authorization: Bearer $TOKEN" "$BASE/tabs/$ID/read")
CURSOR=$(echo $RESP | jq -r .cursor)

# 4. 发送命令
curl -s -X POST -H "Authorization: Bearer $TOKEN" \
  -H "Content-Type: application/json" \
  -d '{"data": "echo hello world\n"}' \
  "$BASE/tabs/$ID/write"

# 5. 等命令执行
sleep 1

# 6. 读取输出（增量）
RESP=$(curl -s -H "Authorization: Bearer $TOKEN" "$BASE/tabs/$ID/read?cursor=$CURSOR")
echo $RESP | jq -r .data
CURSOR=$(echo $RESP | jq -r .cursor)

# 7. 关闭终端
curl -s -X DELETE -H "Authorization: Bearer $TOKEN" "$BASE/tabs/$ID"
```
