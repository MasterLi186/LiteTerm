# ZMODEM Send Rust 实现设计

## 目标

用 Rust 从零实现 ZMODEM Send 协议，替代前端 zmodem.js 的 Send session。对标 lrzsz 兼容性，XShell 级可靠性。

仅实现 **Send（上传到远端 rz）**。Receive（从远端 sz 下载）继续用 zmodem.js。

## 成功标准

1. 完全兼容 lrzsz 的 rz：ZHEX/ZBIN32 帧、CRC-32、ZDLE 转义、ZRPOS 重传、crash recovery
2. 稳定传输 3GB+ 文件，失败率 < 1%
3. 拖拽文件到终端 → 自动上传到终端当前目录
4. 终端无乱码，协议帧不泄漏到 xterm.js

## 架构

```
前端: 拖拽文件 → invoke('zmodem_send', {session_id, files: [path]})
                              ↓
后端: Tauri command → 向终端 input_tx 发 "rz\r"
      → 在 reader 线程拦截终端输出 → 喂给 ZmodemSender 状态机
      → ZmodemSender 产出的字节通过 input_tx 写入终端
      → 完成后恢复正常终端输出
                              ↓
前端: 监听 "transfer-progress" 事件 → 右上角浮窗显示进度
```

## 模块结构

### `src-tauri/src/core/zmodem/mod.rs`

模块入口。导出子模块，定义协议常量：

```
ZPAD = 0x2A ('*')
ZDLE = 0x18 (CAN)
ZHEX = 0x42 ('B')
ZBIN32 = 0x43 ('C')
XON = 0x11
XOFF = 0x13

帧类型: ZRQINIT(0), ZRINIT(1), ZSINIT(2), ZACK(3), ZFILE(4), ZSKIP(5),
        ZNAK(6), ZABORT(7), ZFIN(8), ZRPOS(9), ZDATA(10), ZEOF(11),
        ZFERR(12), ZCRC(13), ZCHALLENGE(14), ZCOMPL(15), ZCAN(16)

子包结束标记: ZCRCW(1), ZCRCE(2), ZCRCG(3), ZCRCQ(4)
```

### `src-tauri/src/core/zmodem/encode.rs`

编码层。负责将帧和数据转换为线路字节。

功能：
- `zdle_encode(data: &[u8]) -> Vec<u8>` — ZDLE 转义所有控制字符
- `encode_zhex_header(frame_type, flags: [u8;4]) -> Vec<u8>` — ZHEX 格式头（CRC-16）
- `encode_zbin32_header(frame_type, flags: [u8;4]) -> Vec<u8>` — ZBIN32 格式头（CRC-32）
- `encode_data_subpacket(data: &[u8], end_type: u8) -> Vec<u8>` — ZDLE 编码数据子包 + CRC-32
- `crc16(data: &[u8]) -> u16` — CRC-16/XMODEM
- `crc32(data: &[u8]) -> u32` — CRC-32
- `zcancel() -> Vec<u8>` — 8×CAN + 8×BS 取消序列
- `encode_zfile_subpacket(name: &str, size: u64, mtime: u64) -> Vec<u8>` — ZFILE 文件信息子包

ZDLE 转义规则（对标 lrzsz ESCALL 模式）：
- 0x00-0x1f: 全部转义为 ZDLE + (byte | 0x40)
- 0x7f (DEL): 转义为 ZDLE + 0x6f
- 0x80-0x9f: 转义为 ZDLE + (byte | 0x40) (high-bit variants)
- ZDLE (0x18) 自身: ZDLE + 0x58
- 0xff: 转义为 ZDLE + 0x6f (lrzsz 兼容)

### `src-tauri/src/core/zmodem/decode.rs`

解码层。解析从远端（rz）收到的帧。

功能：
- `ZmodemDecoder` 结构体 — 增量解析器，处理跨包边界的帧
  - `feed(data: &[u8]) -> Vec<DecodedFrame>` — 喂入字节，返回解析出的帧列表
  - 内部缓冲区处理碎片化输入
- `DecodedFrame { frame_type, flags: [u8;4] }` — 解析结果
- 支持 ZHEX 和 ZBIN32 两种格式（rz 可能用任一种回复）
- ZDLE 反转义
- CRC-16 和 CRC-32 校验

rz 回复的帧类型：
- `ZRINIT` — 准备接收，flags 包含窗口大小和能力位
- `ZRPOS(offset)` — 请求从 offset 重传（flags 低4字节 = LE u32 offset）
- `ZSKIP` — 跳过当前文件
- `ZACK(offset)` — 确认到 offset
- `ZFIN` — 结束
- `ZNAK` — 否认，请求重发上一个头
- `ZABORT` — 中止
- `ZCAN` — 取消（5×CAN）

### `src-tauri/src/core/zmodem/sender.rs`

Send 协议状态机。

```rust
pub struct ZmodemSender {
    state: SenderState,
    files: Vec<FileInfo>,
    current_file_idx: usize,
    file_handle: Option<std::fs::File>,
    file_offset: u64,
    receiver_window: u32,     // 从 ZRINIT flags 读取
    receiver_can_crc32: bool, // ZRINIT 能力位
}

enum SenderState {
    WaitZrinit,        // 等待 rz 发 ZRINIT
    SendingZfile,      // 发 ZFILE 头 + 文件信息子包
    WaitZrposOrZskip,  // 等 rz 回 ZRPOS(0) 接受或 ZSKIP 跳过
    SendingData,       // 发 ZDATA 数据子包
    WaitZrposAfterEof, // 发了 ZEOF，等 ZRINIT（下一个文件）或 ZRPOS（重传）
    SendingZfin,       // 发 ZFIN
    WaitZfinReply,     // 等 ZFIN 回复
    Done,              // 发 OO，完成
}
```

核心方法：
- `new(files: Vec<(PathBuf, String)>) -> Self`
- `handle_frame(frame: DecodedFrame) -> SenderAction` — 状态机转移
- `next_data_chunk() -> Option<Vec<u8>>` — 读文件并编码为数据子包
- `handle_zrpos(offset: u64)` — seek 文件到 offset，重发

`SenderAction` 枚举：
```rust
enum SenderAction {
    Send(Vec<u8>),           // 向终端写入这些字节
    Progress(u64, u64, String), // (bytes_sent, total, filename)
    FileComplete(String),
    AllComplete,
    Error(String),
    None,                    // 不需要操作
}
```

数据发送策略（对标 lrzsz）：
- 使用 ZBIN32 帧格式发数据（比 ZHEX 高效）
- 数据子包大小：8192 字节（lrzsz 默认）
- 子包结束标记：ZCRCG（连续发送，不等 ACK），最后一个用 ZCRCE
- 每发 32 个子包（256KB）发一个 ZCRCQ 子包请求 ZACK（流控）
- 收到 ZRPOS 时：seek 文件到 offset，从该位置重发（关键的错误恢复）

### `src-tauri/src/commands/zmodem.rs`

Tauri 命令层。桥接前端和 ZmodemSender。

```rust
#[tauri::command]
pub async fn zmodem_send(
    state: State<'_, AppState>,
    app: AppHandle,
    session_id: String,
    files: Vec<String>,  // 本地文件路径列表
) -> Result<(), String>
```

实现流程：
1. 获取 session 的 `input_tx`（写入终端）
2. 创建一对 channel `(zmodem_tx, zmodem_rx)` 用于接收终端输出
3. 设置 `session.zmodem_active = true`，`session.zmodem_tx = Some(zmodem_tx)`
4. 向终端发 `rz\r`（通过 input_tx）
5. 启动 ZmodemSender 状态机
6. 循环：
   - 从 `zmodem_rx` 读取终端输出 → `decoder.feed()` → 解析帧
   - `sender.handle_frame(frame)` → 获取 action
   - 执行 action（Send → input_tx, Progress → emit event）
   - 如果 sender 在 SendingData 状态 → `sender.next_data_chunk()` → input_tx
7. 完成或错误后：`session.zmodem_active = false`，清理 `zmodem_tx`

超时处理：
- 等待 ZRINIT 超时 5 秒 → 取消
- 等待 ZRPOS/ZACK 超时 30 秒 → 发 ZCAN 取消
- cancel_transfer 支持：检查 AtomicBool flag

### reader 线程修改（`commands/ssh.rs`）

在 `ManagedSession` 增加字段：
```rust
pub zmodem_active: Arc<AtomicBool>,
pub zmodem_tx: Mutex<Option<mpsc::Sender<Vec<u8>>>>,
```

reader 线程循环修改：
```rust
match channel.read(&mut buf) {
    Ok(n) if n > 0 => {
        if zmodem_active.load(Relaxed) {
            // ZMODEM 模式：数据发给状态机
            if let Some(tx) = zmodem_tx.lock().unwrap().as_ref() {
                let _ = tx.send(buf[..n].to_vec());
            }
        } else {
            // 正常模式：数据发给前端
            app.emit("terminal-output", ...);
        }
    }
    ...
}

// 写入也需要修改：ZMODEM 模式下忽略用户键盘输入
if !zmodem_active.load(Relaxed) {
    while let Ok(data) = input_rx.try_recv() {
        channel.write_all(&data);
    }
} else {
    // ZMODEM 模式下 input_rx 的数据由 zmodem command 写入
    while let Ok(data) = input_rx.try_recv() {
        channel.write_all(&data);
    }
}
```

### 前端改动

**App.tsx 拖拽处理：**
```typescript
// 不再查询 pwd，直接调用 zmodem_send
const paths = event.payload.paths;
await invoke('zmodem_send', { sessionId: sid, files: paths });
```

**TerminalPane.tsx：**
- zmodem.js 的 Send session 代码移除（`role === 'send'` 分支）
- Receive session（`role === 'receive'`）保留
- 移除 `pendingUploadRef`、`onRegisterPwdQuery` 等上传相关的前端逻辑

**进度显示：**
- 复用现有 `transfer-progress` 事件和右上角浮窗
- `direction: "zmodem-upload"` 区分于 SFTP 上传

## 不实现的功能

- ZMODEM Receive（sz 下载）— 继续用 zmodem.js
- ZMODEM-90（加密扩展）
- YMODEM/XMODEM 兼容
- 自动检测 rz 是否安装（假设远端已装 lrzsz）

## 测试计划

1. 单元测试：encode/decode 帧编码解码正确性、CRC 计算、ZDLE 转义/反转义
2. 集成测试：向本地 rz 进程传输文件并校验 md5
3. 手工测试：
   - 小文件（< 1KB）
   - 大文件（> 1GB）
   - 含控制字符的二进制文件
   - 网络不稳定（人为制造延迟）下的 ZRPOS 重传
   - 连续多文件拖拽
   - 传输中取消
