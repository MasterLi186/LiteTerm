# API 自动化调试

通过 HTTP API 在后台自动复现 bug、加日志、编译、复测。用户的 LiteTerm 不受影响。

## 绝对禁止

- **禁止 kill 任何 guishell-tauri 进程**
- **禁止 pkill / killall / kill -9 任何与 guishell 相关的进程**
- **上次违反此规则导致用户丢失了所有 SSH 会话**

| 想法 | 回答 |
|------|------|
| "先杀旧的再启新的" | 不行，用户会话会丢失 |
| "只杀测试实例" | 不行，PID 可能搞混 |
| "进程太多了" | 不行，让用户自己决定关哪个 |

## 流程

### 1. 编译

```bash
cd /home/lfl/ssd/code/guishell && ./build.sh
```

### 2. 启动测试实例

直接启动，自动端口选择会避开用户占用的端口：

```bash
./run.sh &
sleep 3
```

### 3. 获取连接信息

```bash
PORT=$(cat ~/.config/guishell/api_port | jq -r .port)
TOKEN=$(cat ~/.config/guishell/api_token)
BASE="http://127.0.0.1:$PORT/api/v1"
```

验证可达：
```bash
curl -s -H "Authorization: Bearer $TOKEN" "$BASE/tabs" | jq length
```

### 4. 通过 API 复现 bug

```bash
# 打开终端
ID=$(curl -s -X POST -H "Authorization: Bearer $TOKEN" "$BASE/tabs/local" | jq -r .id)
sleep 1

# 发命令
curl -s -X POST -H "Authorization: Bearer $TOKEN" \
  -H "Content-Type: application/json" \
  -d '{"data": "要执行的命令\n"}' \
  "$BASE/tabs/$ID/write"
sleep 1

# 读输出
curl -s -H "Authorization: Bearer $TOKEN" "$BASE/tabs/$ID/read" | jq -r .data
```

### 5. 检查日志

```bash
grep "关键词" ~/guishell.log | tail -20
```

### 6. 修改代码 → 重复 1-5

修改代码后重新编译，启动新测试实例（又会拿到下一个可用端口），通过 API 复测。

### 7. 清理测试实例

**提示用户手动关闭测试实例的窗口。不要用 kill。**

## API 速查

| 操作 | 命令 |
|------|------|
| 列出标签 | `curl -s -H "Auth..." "$BASE/tabs"` |
| 开终端 | `curl -s -X POST -H "Auth..." "$BASE/tabs/local"` |
| 开 SSH | `curl -s -X POST -H "Auth..." -H "Content-Type: application/json" -d '{"host":"...","user":"...","password":"..."}' "$BASE/tabs/ssh"` |
| 写命令 | `curl -s -X POST -H "Auth..." -H "Content-Type: application/json" -d '{"data":"cmd\n"}' "$BASE/tabs/$ID/write"` |
| 读输出 | `curl -s -H "Auth..." "$BASE/tabs/$ID/read?cursor=$C"` |
| 关标签 | `curl -s -X DELETE -H "Auth..." "$BASE/tabs/$ID"` |

`Auth...` = `Authorization: Bearer $TOKEN`
