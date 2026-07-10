# HTTP API 测试计划

> 对应设计: [HTTP API 设计文档](../superpowers/specs/2026-07-10-http-api-design.md)
> 对应 API: [HTTP API 文档](../api/http-api.md)

## 1. 测试策略

### 1.1 测试层级

| 层级 | 方法 | 覆盖范围 |
|------|------|----------|
| 单元测试 | Rust `#[cfg(test)]` | 环形缓冲区、ANSI 过滤、token 校验 |
| 集成测试 | Shell 脚本 + curl | 完整 HTTP 请求→响应链路 |
| 端到端测试 | Shell 脚本自动化流程 | 打开终端→发命令→读输出→关闭 |

### 1.2 测试环境

- LiteTerm 已启动并监听 `127.0.0.1:19526`
- `~/.config/guishell/api_token` 存在且可读
- 系统有可用的 shell（bash/zsh/fish）

## 2. 单元测试用例

### 2.1 环形缓冲区 (TerminalOutputBuffer)

| 编号 | 用例 | 输入 | 预期 |
|------|------|------|------|
| BUF-01 | 空缓冲区读取 | cursor=0 | data=""，cursor=0 |
| BUF-02 | 写入后读取 | 写入 "hello"，cursor=0 | data="hello"，cursor=5 |
| BUF-03 | 增量读取 | 写入 "ab"+"cd"，cursor=2 | data="cd"，cursor=4 |
| BUF-04 | 缓冲区回绕 | 写入 >1MB 数据，cursor=0 | 返回最近 1MB，truncated=true |
| BUF-05 | cursor 过旧 | cursor 指向已覆盖位置 | 返回当前全部，truncated=true |
| BUF-06 | cursor 等于 write_pos | 无新数据 | data=""，cursor 不变 |
| BUF-07 | 并发写入安全 | 多线程同时写入 | 数据不丢不乱 |

### 2.2 ANSI 过滤

| 编号 | 用例 | 输入 | 预期 |
|------|------|------|------|
| ANSI-01 | 颜色转义码 | `\x1b[31mred\x1b[0m` | `red` |
| ANSI-02 | 光标移动 | `\x1b[2J\x1b[H` | `` (空) |
| ANSI-03 | 无转义码 | `plain text` | `plain text` |
| ANSI-04 | raw=true 模式 | `\x1b[31mred\x1b[0m` | 原样返回 |

### 2.3 Token 认证

| 编号 | 用例 | 输入 | 预期 |
|------|------|------|------|
| AUTH-01 | 正确 token | 有效 Bearer token | 200 |
| AUTH-02 | 错误 token | 随机字符串 | 401 |
| AUTH-03 | 无 header | 不带 Authorization | 401 |
| AUTH-04 | 空 token | `Bearer ` (空值) | 401 |

## 3. 集成测试用例

### 3.1 标签页管理

| 编号 | 用例 | 步骤 | 预期 |
|------|------|------|------|
| TAB-01 | 列出空标签 | GET /tabs（无标签时） | `[]` |
| TAB-02 | 打开本地终端 | POST /tabs/local | 返回 id+label，GET /tabs 能看到 |
| TAB-03 | 指定 shell | POST /tabs/local `{"shell_path":"/bin/bash"}` | 成功打开 bash |
| TAB-04 | 无效 shell | POST /tabs/local `{"shell_path":"/nonexistent"}` | 返回错误 |
| TAB-05 | 打开 SSH | POST /tabs/ssh（有效参数） | 返回 id+label |
| TAB-06 | SSH 缺必填字段 | POST /tabs/ssh `{"port":22}` | 400 错误 |
| TAB-07 | 切换焦点 | PUT /tabs/:id/focus | 200，前端标签切换 |
| TAB-08 | 焦点不存在 ID | PUT /tabs/fake-id/focus | 404 |
| TAB-09 | 关闭标签 | DELETE /tabs/:id | 200，GET /tabs 不再包含 |
| TAB-10 | 关闭不存在 ID | DELETE /tabs/fake-id | 404 |

### 3.2 数据读写

| 编号 | 用例 | 步骤 | 预期 |
|------|------|------|------|
| RW-01 | 写入+读取 | write `echo hello\n`，sleep 1s，read | data 包含 "hello" |
| RW-02 | 增量读取 | read 获取 cursor，write 新命令，用 cursor read | 只返回新输出 |
| RW-03 | Ctrl+C | write 一个长命令，write ``，read | 命令被中断 |
| RW-04 | 写入不存在 ID | POST /tabs/fake-id/write | 404 |
| RW-05 | 读取不存在 ID | GET /tabs/fake-id/read | 404 |
| RW-06 | 空 data | POST write `{"data":""}` | 200（写入空数据） |
| RW-07 | 多次增量读取 | 连续 3 次 write+read，每次 cursor 递增 | 每次只返回增量 |

### 3.3 前端联动

| 编号 | 用例 | 步骤 | 预期 |
|------|------|------|------|
| FE-01 | API 开标签 UI 同步 | POST /tabs/local | 前端出现新标签 |
| FE-02 | API 关标签 UI 同步 | DELETE /tabs/:id | 前端标签消失 |
| FE-03 | API 切焦点 UI 同步 | PUT /tabs/:id/focus | 前端切换到该标签 |

## 4. 端到端测试用例

### 4.1 完整自动化流程

| 编号 | 用例 | 步骤 | 预期 |
|------|------|------|------|
| E2E-01 | 基本流程 | 打开终端→发 `echo test`→读输出→关闭 | 输出包含 "test" |
| E2E-02 | 多标签 | 打开 3 个终端，各发不同命令，各自读取 | 输出不串台 |
| E2E-03 | 中断长命令 | 发 `sleep 999`→Ctrl+C→读输出 | 命令被中断 |
| E2E-04 | 大量输出 | 发 `seq 1 10000`→读取 | 数据完整（或 truncated 标记） |
| E2E-05 | 快速连续读写 | 循环 50 次 write+read | 无错误，cursor 单调递增 |

## 5. 测试脚本位置

| 文件 | 内容 |
|------|------|
| `src-tauri/src/state.rs` 内 `#[cfg(test)]` | 单元测试（环形缓冲区 BUF-01~06） |
| `tests/test_http_api.sh` | 集成 + 端到端测试脚本（AUTH、TAB、RW、E2E） |

> ANSI 过滤依赖 `strip-ansi-escapes` crate 自身的正确性，不额外测试。
> Token 认证在集成测试脚本中通过 HTTP 请求覆盖（AUTH-01~04）。
