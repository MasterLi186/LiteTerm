# LiteTerm × 158BitNet 本地 AI 集成设计

## 目标

在 LiteTerm 中集成 158BitNet 本地推理引擎，提供三项 AI 功能：自然语言→命令、终端输出解释、智能命令补全。全部本地运行，无需联网/API key。

## 架构：Sidecar + OpenAI 兼容 HTTP API

```
[用户操作] → [React 前端] → invoke('ai_xxx')
                                  ↓
                            [Rust 后端]
                                  ↓ HTTP POST /v1/chat/completions
                          [openai_server sidecar]
                                  ↓
                      bitcpm4-0.5b-tq2_0.gguf (~250MB)
                      ~/.config/guishell/models/
```

### 组件

| 组件 | 职责 |
|------|------|
| `openai_server` | 158BitNet 提供的 C 二进制，加载 GGUF 模型，暴露 OpenAI 兼容 HTTP API |
| Rust `commands/ai.rs` | 管理 sidecar 生命周期 + 封装 HTTP 调用 + 模型下载 |
| React AI 组件 | AI 输入框 + 右键"AI 解释" + 结果浮窗 |

### Sidecar 生命周期

- LiteTerm 启动时，若模型文件存在且 AI 已启用，自动拉起 `openai_server`（绑 `127.0.0.1:随机端口`）
- LiteTerm 退出时自动 kill 子进程
- sidecar 崩溃时自动重启（最多 3 次）
- `openai_server` 二进制在构建时由 CMake 编译，打包进 Tauri bundle（三平台各一份）

### 模型分发

安装包**不包含模型**，保持轻量（~15MB）。

- 首次触发 AI 功能时，检测 `~/.config/guishell/models/bitcpm4-0.5b-tq2_0.gguf` 是否存在
- 不存在 → **后台静默下载**（从 HuggingFace CDN，约 250MB），不阻塞用户操作
- 下载期间 AI 功能不可用，状态栏/设置页显示下载进度
- 下载完成后自动启动 sidecar，AI 功能就绪
- 模型永久缓存在本地，后续启动直接读取

### 模型参数

| 参数 | 值 |
|------|-----|
| 模型 | bitcpm4-0.5b-tq2_0.gguf |
| 参数量 | 0.5B |
| 文件大小 | ~250MB |
| 量化 | TQ2_0（三值 2-bit） |
| 上下文 | 2048 tokens（默认） |
| 生成上限 | 256 tokens |

## 功能 ①：自然语言→命令（优先级最高）

### 交互流程

1. 用户按 `Ctrl+I` 或点击 AI 图标 → 弹出 AI 输入框（标签栏下方浮层）
2. 输入中文描述（如"查看 80 端口占用"）→ 回车
3. AI 返回命令（如 `lsof -i :80`），显示在结果区
4. 用户点击"执行"→ 命令注入当前终端；或点击"复制"→ 进剪贴板
5. 按 Esc 或点空白处关闭

### Prompt

```
你是一个 Linux/macOS 命令助手。根据用户的描述，只返回一条可直接执行的 shell 命令，不要解释，不要 markdown。
用户: {input}
命令:
```

### API 调用

```json
{
  "messages": [
    {"role": "system", "content": "<system prompt>"},
    {"role": "user", "content": "<用户输入>"}
  ],
  "max_tokens": 128
}
```

## 功能 ②：终端输出解释

### 交互流程

1. 用户在终端选中一段文本
2. 右键菜单 → "AI 解释"
3. 终端右侧/下方弹出浮窗，显示 AI 解释
4. 浮窗可关闭，不影响终端操作

### Prompt

```
用简洁的中文解释以下终端输出的含义，如果是错误请给出可能的原因和解决方法：

{selected_text}
```

### API 调用

```json
{
  "messages": [
    {"role": "system", "content": "<system prompt>"},
    {"role": "user", "content": "<选中文本>"}
  ],
  "max_tokens": 256
}
```

## 功能 ③：智能命令补全（后续迭代）

本次不实现。等 ①② 落地、sidecar 基础设施稳定后，作为后续迭代加入。届时需要：
- 监听终端输入（实时，低延迟要求 <500ms）
- 结合 shell history + 当前上下文生成建议
- xterm.js 输入框内联补全 UI

## 配置

`~/.config/guishell/settings.toml` 新增：

```toml
[ai]
enabled = true
model_path = "~/.config/guishell/models/bitcpm4-0.5b-tq2_0.gguf"
```

## 改动范围

| 层 | 文件 | 改动 |
|----|------|------|
| 构建 | CI workflow / build.sh | CMake 编译 158BitNet → openai_server；Tauri sidecar 配置 |
| Rust 后端 | `commands/ai.rs`（新建） | sidecar 管理 + HTTP 调用 + 模型下载 |
| Rust 后端 | `config/settings.rs` | `[ai]` 配置区段 |
| Rust 后端 | `lib.rs` | 注册 AI 命令 |
| React 前端 | `components/AI/AiCommandInput.tsx`（新建） | 自然语言→命令输入框 + 结果浮窗 |
| React 前端 | `components/Terminal/TerminalPane.tsx` | 右键菜单加"AI 解释" |
| React 前端 | `App.tsx` | AI 输入框挂载 + 快捷键 Ctrl+I |

## 不做什么

- 不做模型训练/微调
- 不做流式输出（0.5B 响应快，一次性返回够用）
- 不做多模型切换 UI（先硬编码 0.5B）
- 不做 Windows AI 支持（158BitNet 无 Windows 官方支持，CI Windows 构建跳过 AI sidecar）
- 不做云端 API 对接（纯本地）
- 不打包模型到安装包（首次使用时后台静默下载）

## 平台支持

| 平台 | AI 支持 | 说明 |
|------|---------|------|
| Linux x86_64 | 是 | 无 ARM NEON 优化，速度较慢但可用 |
| Linux ARM64 | 是 | 原生优化，最佳性能 |
| macOS (Apple Silicon) | 是 | ARM NEON + 可选 Metal 加速 |
| macOS (Intel) | 是 | 同 Linux x86_64 |
| Windows | 否 | 158BitNet 无官方 Windows 支持，AI 功能禁用 |
