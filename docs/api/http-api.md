# LiteTerm HTTP API

> 版本：v1
> 基础路径：`http://127.0.0.1:<port>/api/v1`

HTTP API 仅监听 IPv4 loopback。Native 与 legacy/Tauri 使用相同的 v1 路由，但 discovery 文件不同；客户端不要混用 token。

## 服务发现与认证

| 实现 | Token 文件 | Port 文件 | 说明 |
|------|------------|-----------|------|
| Native | `~/.config/guishell/native-api-token` | `~/.config/guishell/native-api-port` | 当前 Native 接口；port 文件是 JSON |
| legacy/Tauri | `~/.config/guishell/api_token` | 无 | 兼容旧客户端，固定使用其既有端口配置 |

Native 默认监听端口为 `19526`，但客户端应在每次连接前读取 `native-api-port`，以发现实际监听端口。该文件格式如下：

```json
{"port":19526,"pid":12345,"instance":"随机实例标识"}
```

Native 启动时生成 32 字节随机 token，以 64 个十六进制字符写入 `native-api-token`。两个 discovery 文件仅允许当前用户访问，并在服务正常关闭时删除。文件不存在通常表示 Native API 未运行；客户端不得自动改读 legacy token。

```bash
CONFIG_DIR="${XDG_CONFIG_HOME:-$HOME/.config}/guishell"
TOKEN=$(tr -d '\r\n' < "$CONFIG_DIR/native-api-token")
PORT=$(jq -r '.port // 19526' "$CONFIG_DIR/native-api-port")
BASE="http://127.0.0.1:$PORT/api/v1"
```

所有请求都必须携带 Bearer token：

```text
Authorization: Bearer <token>
```

Token 缺失或错误时返回 `401 Unauthorized`。Native 使用常量时间比较认证信息，并在解析请求体之前完成认证。

## 通用约定

- 成功响应为 HTTP 2xx 和 JSON body。
- Native 错误响应为 `{"error":"描述信息","code":"错误代码"}`；legacy 客户端仍应兼容仅含 `error` 的响应。
- 请求体最大为 64 KiB，读取请求体超时为 5 秒；对应返回 `413 body_too_large` 或 `408 body_timeout`。
- 需要主线程处理的请求默认超时为 5 秒，超时返回 `504 main_thread_timeout`。超时操作不会稍后补执行。
- 服务最多同时处理 32 个请求。自动化客户端建议设置略大于 5 秒的自身超时。
- `focus`、`write`、`read` 和 `DELETE` 都接受可选的 `pane_id` 查询参数。省略时使用该标签页的当前活跃面板；空字符串无效。

## 接口概览

| 方法 | 路径 | 用途 |
|------|------|------|
| GET | `/tabs` | 列出标签页和面板 |
| POST | `/tabs/local` | 打开本地终端 |
| POST | `/tabs/ssh` | 打开 SSH 终端 |
| PUT | `/tabs/:id/focus` | 聚焦标签页或面板 |
| POST | `/tabs/:id/write` | 写入终端 |
| GET | `/tabs/:id/read` | 增量读取终端输出 |
| DELETE | `/tabs/:id` | 关闭标签页 |

## GET /tabs

列出所有标签页。Native 响应包含分屏信息：

```json
[
  {
    "id": "tab-uuid",
    "label": "本地终端 1",
    "type": "local",
    "panes": [
      {"id": "pane-uuid-1", "active": true},
      {"id": "pane-uuid-2", "active": false}
    ],
    "active_pane_id": "pane-uuid-1"
  }
]
```

`type` 当前可能为 `local`、`ssh`、`serial`、`process` 或 `network`。旧实现可能不返回 `panes` 和 `active_pane_id`，客户端应将它们视为可选字段。

```bash
curl -sS -H "Authorization: Bearer $TOKEN" "$BASE/tabs"
```

## POST /tabs/local

打开本地终端。请求体可省略，也可以传空对象：

```json
{"shell_path":"/usr/bin/fish"}
```

省略 `shell_path` 或传 `null` 时使用 `$SHELL`，未设置则使用 `/bin/bash`。显式路径必须是绝对路径、普通文件且可执行；空字符串无效。

响应：

```json
{"id":"tab-uuid","label":"本地终端 1"}
```

```bash
# 默认 shell
curl -sS -X POST -H "Authorization: Bearer $TOKEN" "$BASE/tabs/local"

# 指定 shell
curl -sS -X POST \
  -H "Authorization: Bearer $TOKEN" \
  -H "Content-Type: application/json" \
  -d '{"shell_path":"/usr/bin/fish"}' \
  "$BASE/tabs/local"
```

## POST /tabs/ssh

请求体：

```json
{
  "host": "192.168.1.10",
  "port": 22,
  "user": "root",
  "password": "secret",
  "auth_method": "password",
  "key_path": null,
  "proxy_jump": null
}
```

| 字段 | 类型 | 必填 | 说明 |
|------|------|------|------|
| `host` | string | 是 | 主机地址，不能为空 |
| `port` | number | 否 | 默认 `22`，不能为 `0` |
| `user` | string | 是 | 用户名，不能为空 |
| `password` | string | 否 | 密码认证所需；API 提供的密码仅用于该连接 |
| `auth_method` | string | 否 | `password`、`key` 或 `agent` |
| `key_path` | string | 否 | `key` 认证使用的私钥路径 |
| `proxy_jump` | string | 否 | Native 当前不支持；非空值返回 `400` |

省略 `auth_method` 时，存在 `password` 则推断为 `password`，否则存在 `key_path` 则推断为 `key`，其余情况使用 `agent`。接口在创建 SSH 占位标签后返回；连接尚未就绪时，写入可能暂时返回 `404`。

响应：

```json
{"id":"tab-uuid","label":"root@192.168.1.10"}
```

## PUT /tabs/:id/focus

聚焦标签页。传入 `pane_id` 时同时聚焦指定分屏；省略时聚焦当前活跃面板。

```bash
curl -sS -X PUT -H "Authorization: Bearer $TOKEN" \
  "$BASE/tabs/tab-uuid/focus?pane_id=pane-uuid"
```

响应：

```json
{"ok":true}
```

标签或面板不存在时返回 `404`。

## POST /tabs/:id/write

向指定终端面板写入 UTF-8 文本：

```json
{"data":"ls -la\n"}
```

```bash
curl -sS -X POST \
  -H "Authorization: Bearer $TOKEN" \
  -H "Content-Type: application/json" \
  -d '{"data":"ls -la\n"}' \
  "$BASE/tabs/tab-uuid/write?pane_id=pane-uuid"
```

响应：

```json
{"ok":true}
```

控制字符使用 JSON Unicode 转义，例如 Ctrl+C 为 `"\u0003"`、Ctrl+D 为 `"\u0004"`、Ctrl+Z 为 `"\u001a"`。终端未就绪或不可写时返回 `404`，写入队列已满时返回 `429 write_queue_full`，ZMODEM 传输期间返回 `409 zmodem_active`。

## GET /tabs/:id/read

从指定面板的内存环形缓冲区增量读取输出。

| 查询参数 | 类型 | 默认值 | 说明 |
|----------|------|--------|------|
| `pane_id` | string | 活跃面板 | 指定分屏 |
| `cursor` | uint64 | `0` | 原始终端字节流中的读取位置 |
| `stream_id` | uint64 | 无 | 上次响应的流标识，用于发现终端重连或替换 |
| `limit` | usize | 256 KiB | 本次最多消费的原始字节数，上限 256 KiB |
| `raw` | bool | `false` | `true` 保留 ANSI 控制序列；`false` 过滤 ANSI CSI/OSC 序列 |

响应：

```json
{
  "data": "total 42\n",
  "cursor": 12345,
  "truncated": false,
  "stream_id": 7,
  "pane_id": "pane-uuid"
}
```

每个面板最多保留最近 1 MiB 原始输出，每次响应最多读取 256 KiB。`cursor` 按原始字节计数，不是 `data` 的字符数；过滤 ANSI 或丢弃无效 UTF-8 后，`data` 长度可能小于 cursor 的增量。

响应始终提供有效 UTF-8 字符串。读取边界会避开 UTF-8 continuation byte；无效或不完整的 UTF-8 字节会被跳过，不插入替换字符。`raw=true` 只表示保留 ANSI 序列，不会把任意二进制字节改为可逆编码。

以下情况会返回 `truncated=true`，并从当前仍保留的最早位置继续：

- cursor 已被 1 MiB 环形缓冲区覆盖；
- cursor 超出当前流末尾；
- 请求的 `stream_id` 与当前流不同。

客户端每次都应保存响应中的 `cursor` 和 `stream_id`：

```bash
curl -sS -H "Authorization: Bearer $TOKEN" \
  "$BASE/tabs/tab-uuid/read?pane_id=pane-uuid&cursor=12345&stream_id=7&limit=65536&raw=false"
```

## DELETE /tabs/:id

关闭整个标签页并释放其终端资源：

```bash
curl -sS -X DELETE -H "Authorization: Bearer $TOKEN" \
  "$BASE/tabs/tab-uuid"
```

响应：

```json
{"ok":true}
```

Native 当前接受可选的 `pane_id`，并在关闭前验证该面板属于标签页，但关闭操作仍会关闭整个标签页，而不是只关闭一个分屏。

## Native 冒烟测试

仓库中的 `native-prototype/scripts/test_http_api.sh` 只读取 Native discovery 文件，不会启动应用，也不会读取 legacy token。它验证认证、列表、本地终端打开、写入、增量读取和关闭；异常退出时只尝试关闭由脚本自己创建的标签页。

先由用户手动启动 Native 应用，再运行：

```bash
native-prototype/scripts/test_http_api.sh
```
