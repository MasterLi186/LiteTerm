# 158BitNet 本地 AI 集成实现计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 在 LiteTerm 中集成 158BitNet 本地推理引擎，提供"自然语言→命令"和"终端输出解释"两项 AI 功能，通过 sidecar 进程 + OpenAI 兼容 HTTP API 实现。

**Architecture:** 158BitNet 的 `openai_server` 作为 Tauri sidecar 子进程运行（绑 `127.0.0.1:动态端口`），Rust 后端管理其生命周期并通过 HTTP 调用 `/v1/chat/completions`；模型文件首次使用时后台静默下载到 `~/.config/guishell/models/`。React 前端通过 Tauri invoke 触发 AI 请求，结果展示在浮层/浮窗中。

**Tech Stack:** 158BitNet (C/CMake) → openai_server binary; Rust (reqwest HTTP client, tokio::process sidecar management); React/TypeScript (AI UI components)

---

## 文件结构

### 新建文件

| 文件 | 职责 |
|------|------|
| `src-tauri/src/commands/ai.rs` | AI 命令：sidecar 生命周期管理 + HTTP 调用 + 模型下载 |
| `src/components/AI/AiCommandInput.tsx` | 自然语言→命令输入框 + 结果浮窗组件 |

### 修改文件

| 文件 | 改动 |
|------|------|
| `src-tauri/src/commands/mod.rs` | 加 `pub mod ai;` |
| `src-tauri/src/config/settings.rs` | 加 `AiSettings` 结构体 + 默认值 |
| `src-tauri/src/state.rs` | AppState 加 AI sidecar 状态字段 |
| `src-tauri/src/lib.rs` | 注册 AI 命令 + 启动时拉起 sidecar |
| `src-tauri/Cargo.toml` | 加 `reqwest` 依赖 |
| `src/components/Terminal/TerminalPane.tsx` | 右键菜单加"AI 解释" |
| `src/App.tsx` | 挂载 AiCommandInput + 快捷键 Ctrl+I |

---

### Task 1: Rust 配置与状态基础

**Files:**
- Modify: `src-tauri/src/config/settings.rs`
- Modify: `src-tauri/src/state.rs`
- Modify: `src-tauri/Cargo.toml`

- [ ] **Step 1: 在 Cargo.toml 加 reqwest 依赖**

在 `src-tauri/Cargo.toml` 的 `[dependencies]` 末尾加：

```toml
reqwest = { version = "0.12", features = ["json"] }
```

- [ ] **Step 2: 在 settings.rs 加 AiSettings**

在 `src-tauri/src/config/settings.rs` 中，`ZmodemSettings` 结构体之后加：

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AiSettings {
    pub enabled: bool,
    pub model_path: String,
}

impl Default for AiSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            model_path: "~/.config/guishell/models/bitcpm4-0.5b-tq2_0.gguf".to_string(),
        }
    }
}
```

在 `Settings` 结构体的字段列表末尾加 `pub ai: AiSettings,`，在 `impl Default for Settings` 的 `Self { ... }` 里加 `ai: AiSettings::default(),`。

- [ ] **Step 3: 在 state.rs 加 AI sidecar 状态**

在 `src-tauri/src/state.rs` 的 `AppState` 结构体末尾加：

```rust
    pub ai_port: Mutex<Option<u16>>,
    pub ai_child: Mutex<Option<u32>>, // sidecar 进程 PID
    pub ai_downloading: Arc<AtomicBool>,
```

在文件顶部确保有 `use std::sync::Arc;` 和 `use std::sync::atomic::AtomicBool;`（可能已有）。

- [ ] **Step 4: 在 lib.rs 初始化新的 AppState 字段**

在 `src-tauri/src/lib.rs` 中 `AppState { ... }` 初始化块里加：

```rust
                ai_port: Mutex::new(None),
                ai_child: Mutex::new(None),
                ai_downloading: Arc::new(AtomicBool::new(false)),
```

- [ ] **Step 5: 编译验证**

Run: `cd src-tauri && cargo build 2>&1 | tail -5`
Expected: `Finished` 无错误

- [ ] **Step 6: 提交**

```bash
git add src-tauri/Cargo.toml src-tauri/src/config/settings.rs src-tauri/src/state.rs src-tauri/src/lib.rs
git commit -m "feat(ai): 加 AiSettings 配置 + AppState AI 状态字段 + reqwest 依赖"
```

---

### Task 2: AI 命令模块 — sidecar 管理 + HTTP 调用 + 模型下载

**Files:**
- Create: `src-tauri/src/commands/ai.rs`
- Modify: `src-tauri/src/commands/mod.rs`
- Modify: `src-tauri/src/lib.rs`

- [ ] **Step 1: 创建 ai.rs**

创建 `src-tauri/src/commands/ai.rs`，内容：

```rust
use std::io::Write;
use std::process::Command as StdCommand;
use std::sync::atomic::Ordering;

use tauri::State;

use crate::app_log;
use crate::config::settings::Settings;
use crate::state::AppState;

const MODEL_URL: &str = "https://huggingface.co/OpenBMB/BitCPM-CANN-8B-gguf/resolve/main/bitcpm4-0.5b-tq2_0.gguf";
const SIDECAR_BIN: &str = "openai_server";
const MAX_RESTART: u32 = 3;

fn model_path(settings: &Settings) -> String {
    shellexpand::tilde(&settings.ai.model_path).to_string()
}

fn sidecar_bin_path() -> Option<String> {
    // 开发环境：在项目目录的 158BitNet/build/ 下查找
    let dev_path = std::env::current_exe()
        .ok()?
        .parent()?
        .join("../../158BitNet/build/openai_server");
    if dev_path.exists() {
        return Some(dev_path.to_string_lossy().to_string());
    }
    // 生产环境：在同一目录下查找打包的二进制
    let prod_path = std::env::current_exe()
        .ok()?
        .parent()?
        .join(SIDECAR_BIN);
    if prod_path.exists() {
        return Some(prod_path.to_string_lossy().to_string());
    }
    None
}

fn find_free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .and_then(|l| l.local_addr())
        .map(|a| a.port())
        .unwrap_or(18573)
}

/// 启动 sidecar 进程。成功时把 PID 和端口写入 AppState。
pub fn start_sidecar(state: &AppState) -> Result<(), String> {
    let settings = state.settings.lock().unwrap();
    if !settings.ai.enabled {
        return Err("AI 未启用".into());
    }
    let mpath = model_path(&settings);
    drop(settings);

    if !std::path::Path::new(&mpath).exists() {
        return Err("模型文件不存在，正在后台下载".into());
    }

    let bin = sidecar_bin_path().ok_or("找不到 openai_server 二进制")?;
    let port = find_free_port();

    let child = StdCommand::new(&bin)
        .arg(&mpath)
        .arg("--host").arg("127.0.0.1")
        .arg("--port").arg(port.to_string())
        .arg("--ctx").arg("2048")
        .arg("--max-tokens").arg("256")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|e| format!("启动 openai_server 失败: {}", e))?;

    let pid = child.id();
    app_log!("AI", "sidecar 已启动: pid={} port={} model={}", pid, port, mpath);

    *state.ai_port.lock().unwrap() = Some(port);
    *state.ai_child.lock().unwrap() = Some(pid);

    Ok(())
}

/// 停止 sidecar。
pub fn stop_sidecar(state: &AppState) {
    if let Some(pid) = state.ai_child.lock().unwrap().take() {
        #[cfg(unix)]
        {
            unsafe { libc::kill(pid as i32, libc::SIGTERM); }
        }
        // 非 unix 平台：忽略（Windows 不支持 AI）
        app_log!("AI", "sidecar 已停止: pid={}", pid);
    }
    *state.ai_port.lock().unwrap() = None;
}

/// 后台静默下载模型。
#[tauri::command]
pub async fn ai_download_model(state: State<'_, AppState>) -> Result<(), String> {
    let settings = state.settings.lock().unwrap().clone();
    let mpath = model_path(&settings);

    if std::path::Path::new(&mpath).exists() {
        return Ok(());
    }

    if state.ai_downloading.load(Ordering::Acquire) {
        return Err("模型正在下载中".into());
    }
    state.ai_downloading.store(true, Ordering::Release);

    let downloading = state.ai_downloading.clone();
    let state_inner = state.inner().clone();

    tokio::spawn(async move {
        app_log!("AI", "开始下载模型: {} -> {}", MODEL_URL, mpath);
        let result = async {
            let resp = reqwest::get(MODEL_URL).await
                .map_err(|e| format!("下载请求失败: {}", e))?;
            if !resp.status().is_success() {
                return Err(format!("下载失败: HTTP {}", resp.status()));
            }
            let bytes = resp.bytes().await
                .map_err(|e| format!("下载数据失败: {}", e))?;

            let dir = std::path::Path::new(&mpath).parent()
                .ok_or("无效模型路径")?;
            std::fs::create_dir_all(dir)
                .map_err(|e| format!("创建目录失败: {}", e))?;

            let mut file = std::fs::File::create(&mpath)
                .map_err(|e| format!("创建文件失败: {}", e))?;
            file.write_all(&bytes)
                .map_err(|e| format!("写入文件失败: {}", e))?;

            app_log!("AI", "模型下载完成: {} ({} MB)", mpath, bytes.len() / 1_048_576);
            Ok::<(), String>(())
        }.await;

        downloading.store(false, Ordering::Release);

        match result {
            Ok(()) => {
                let _ = start_sidecar(&state_inner);
            }
            Err(e) => {
                app_log!("AI", "模型下载失败: {}", e);
            }
        }
    });

    Ok(())
}

/// 调用 AI：发送消息到 sidecar 的 /v1/chat/completions
#[tauri::command]
pub async fn ai_chat(
    state: State<'_, AppState>,
    system_prompt: String,
    user_message: String,
    max_tokens: Option<u32>,
) -> Result<String, String> {
    let port = state.ai_port.lock().unwrap()
        .ok_or("AI 服务未启动（模型可能正在下载）")?;

    let url = format!("http://127.0.0.1:{}/v1/chat/completions", port);
    let body = serde_json::json!({
        "messages": [
            {"role": "system", "content": system_prompt},
            {"role": "user", "content": user_message}
        ],
        "max_tokens": max_tokens.unwrap_or(256)
    });

    let resp = reqwest::Client::new()
        .post(&url)
        .json(&body)
        .timeout(std::time::Duration::from_secs(60))
        .send()
        .await
        .map_err(|e| format!("AI 请求失败: {}", e))?;

    if !resp.status().is_success() {
        let text = resp.text().await.unwrap_or_default();
        return Err(format!("AI 返回错误: {}", text));
    }

    let json: serde_json::Value = resp.json().await
        .map_err(|e| format!("解析 AI 响应失败: {}", e))?;

    json["choices"][0]["message"]["content"]
        .as_str()
        .map(|s| s.trim().to_string())
        .ok_or_else(|| "AI 响应格式异常".to_string())
}

/// 查询 AI 状态
#[tauri::command]
pub async fn ai_status(state: State<'_, AppState>) -> Result<serde_json::Value, String> {
    let port = *state.ai_port.lock().unwrap();
    let downloading = state.ai_downloading.load(Ordering::Relaxed);
    let settings = state.settings.lock().unwrap();
    let enabled = settings.ai.enabled;
    let mpath = model_path(&settings);
    let model_exists = std::path::Path::new(&mpath).exists();

    Ok(serde_json::json!({
        "enabled": enabled,
        "model_exists": model_exists,
        "downloading": downloading,
        "sidecar_running": port.is_some(),
        "port": port,
    }))
}
```

- [ ] **Step 2: 在 mod.rs 注册模块**

在 `src-tauri/src/commands/mod.rs` 末尾加：

```rust
pub mod ai;
```

- [ ] **Step 3: 在 lib.rs 注册命令 + 启动钩子**

在 `src-tauri/src/lib.rs` 的 `invoke_handler` 宏参数列表末尾加：

```rust
            commands::ai::ai_chat,
            commands::ai::ai_download_model,
            commands::ai::ai_status,
```

在 `invoke_handler` 之后、`.run(...)` 之前加 sidecar 自动启动 + 退出清理：

```rust
        .setup(|app| {
            let state = app.state::<AppState>();
            // 启动时尝试拉起 AI sidecar（模型不存在会静默跳过）
            if let Err(e) = commands::ai::start_sidecar(state.inner()) {
                app_log!("AI", "启动时 sidecar 未就绪: {}", e);
            }
            Ok(())
        })
```

注意：如果 `lib.rs` 已有 `.setup()`，把 sidecar 启动代码加到现有 setup 闭包内部即可。

在 `Cargo.toml` 的 `[dependencies]` 加 `libc = "0.2"`（用于 kill 信号）。

- [ ] **Step 4: 编译验证**

Run: `cd src-tauri && cargo build 2>&1 | tail -5`
Expected: `Finished` 无错误

- [ ] **Step 5: 提交**

```bash
git add src-tauri/src/commands/ai.rs src-tauri/src/commands/mod.rs src-tauri/src/lib.rs src-tauri/Cargo.toml
git commit -m "feat(ai): AI 命令模块 — sidecar 管理 + HTTP 调用 + 模型下载"
```

---

### Task 3: 前端 — 自然语言→命令输入框

**Files:**
- Create: `src/components/AI/AiCommandInput.tsx`
- Modify: `src/App.tsx`

- [ ] **Step 1: 创建 AiCommandInput.tsx**

创建 `src/components/AI/` 目录和 `AiCommandInput.tsx`：

```tsx
import { useState } from 'react';
import { invoke } from '@tauri-apps/api/core';

const SYSTEM_PROMPT = '你是一个 Linux/macOS 命令助手。根据用户的描述，只返回一条可直接执行的 shell 命令，不要解释，不要 markdown。';

interface Props {
  visible: boolean;
  onClose: () => void;
  onExecute: (command: string) => void;
}

export function AiCommandInput({ visible, onClose, onExecute }: Props) {
  const [input, setInput] = useState('');
  const [result, setResult] = useState('');
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState('');

  if (!visible) return null;

  async function handleSubmit() {
    if (!input.trim() || loading) return;
    setLoading(true);
    setError('');
    setResult('');

    try {
      const status: any = await invoke('ai_status');
      if (!status.sidecar_running) {
        if (!status.model_exists && !status.downloading) {
          invoke('ai_download_model').catch(() => {});
        }
        setError(status.downloading ? 'AI 模型正在下载中，请稍候...' : 'AI 服务未就绪');
        setLoading(false);
        return;
      }

      const resp: string = await invoke('ai_chat', {
        systemPrompt: SYSTEM_PROMPT,
        userMessage: input.trim(),
        maxTokens: 128,
      });
      setResult(resp);
    } catch (e: any) {
      setError(typeof e === 'string' ? e : e.message || 'AI 请求失败');
    }
    setLoading(false);
  }

  function handleKeyDown(e: React.KeyboardEvent) {
    if (e.key === 'Enter' && !e.shiftKey) {
      e.preventDefault();
      handleSubmit();
    }
    if (e.key === 'Escape') {
      onClose();
    }
  }

  return (
    <div
      style={{
        position: 'absolute', top: 36, left: 0, right: 0, zIndex: 50,
        background: '#1c2128', border: '1px solid #30363d', borderRadius: '0 0 8px 8px',
        padding: '8px 12px', display: 'flex', flexDirection: 'column', gap: '6px',
        boxShadow: '0 4px 12px rgba(0,0,0,0.4)',
      }}
      onClick={(e) => e.stopPropagation()}
    >
      <div style={{ display: 'flex', gap: '8px', alignItems: 'center' }}>
        <span style={{ color: '#00d4ff', fontSize: '14px', flexShrink: 0 }}>AI</span>
        <input
          autoFocus
          value={input}
          onChange={(e) => setInput(e.target.value)}
          onKeyDown={handleKeyDown}
          placeholder="描述你想执行的操作（如：查看 80 端口占用）"
          style={{
            flex: 1, background: '#0d1117', border: '1px solid #30363d', borderRadius: '4px',
            padding: '4px 8px', color: '#e6edf3', fontSize: '13px', outline: 'none',
          }}
        />
        <button
          onClick={handleSubmit}
          disabled={loading || !input.trim()}
          style={{
            background: '#00d4ff', color: '#0d1117', border: 'none', borderRadius: '4px',
            padding: '4px 12px', fontSize: '12px', cursor: loading ? 'wait' : 'pointer',
            opacity: loading || !input.trim() ? 0.5 : 1,
          }}
        >
          {loading ? '...' : '生成'}
        </button>
        <span
          onClick={onClose}
          style={{ color: '#8b949e', cursor: 'pointer', fontSize: '16px' }}
        >×</span>
      </div>

      {error && (
        <div style={{ color: '#f85149', fontSize: '12px' }}>{error}</div>
      )}

      {result && (
        <div style={{
          display: 'flex', alignItems: 'center', gap: '8px',
          background: '#0d1117', borderRadius: '4px', padding: '6px 10px',
        }}>
          <code style={{ flex: 1, color: '#3fb950', fontSize: '13px', fontFamily: 'monospace' }}>
            {result}
          </code>
          <button
            onClick={() => { onExecute(result); onClose(); }}
            style={{
              background: '#238636', color: '#fff', border: 'none', borderRadius: '4px',
              padding: '2px 10px', fontSize: '12px', cursor: 'pointer',
            }}
          >执行</button>
          <button
            onClick={() => { navigator.clipboard.writeText(result); }}
            style={{
              background: '#30363d', color: '#e6edf3', border: 'none', borderRadius: '4px',
              padding: '2px 10px', fontSize: '12px', cursor: 'pointer',
            }}
          >复制</button>
        </div>
      )}
    </div>
  );
}
```

- [ ] **Step 2: 在 App.tsx 挂载 AiCommandInput + 快捷键**

在 `src/App.tsx` 顶部 import 区加：

```tsx
import { AiCommandInput } from './components/AI/AiCommandInput';
```

在 state 声明区（`const [tabContextMenu, ...]` 附近）加：

```tsx
const [showAiInput, setShowAiInput] = useState(false);
```

在 `handleKeyDown` 函数里（快捷键区域）加一行：

```tsx
      if (e.ctrlKey && e.key === 'i') { e.preventDefault(); setShowAiInput(v => !v); }
```

在标签栏 `<div>` 的结束 `</div>` 之后、tab content 之前（`{/* Tab bar */}` 块后面），加：

```tsx
                <AiCommandInput
                  visible={showAiInput}
                  onClose={() => setShowAiInput(false)}
                  onExecute={(cmd) => {
                    if (focusedTerminalId) {
                      const bytes = Array.from(new TextEncoder().encode(cmd + '\n'));
                      invoke('terminal_write', { id: focusedTerminalId, data: bytes });
                    }
                  }}
                />
```

- [ ] **Step 3: 编译验证**

Run:
```bash
npm run build 2>&1 | tail -3 && touch src-tauri/build.rs && cd src-tauri && cargo build 2>&1 | tail -3
```
Expected: 前后端均编译成功

- [ ] **Step 4: 提交**

```bash
git add src/components/AI/AiCommandInput.tsx src/App.tsx
git commit -m "feat(ai): 自然语言→命令输入框（Ctrl+I 触发）"
```

---

### Task 4: 前端 — 终端右键"AI 解释"

**Files:**
- Modify: `src/components/Terminal/TerminalPane.tsx`

- [ ] **Step 1: 加 AI 解释状态和处理函数**

在 `src/components/Terminal/TerminalPane.tsx` 的 state 声明区（`const [searchVisible, ...]` 附近）加：

```tsx
  const [aiExplain, setAiExplain] = useState<{ text: string; result: string; loading: boolean; error: string } | null>(null);
```

在 `handlePaste` 函数后面加：

```tsx
  async function handleAiExplain() {
    const term = termRef.current;
    if (!term) return;
    const selection = term.getSelection();
    if (!selection) return;

    setAiExplain({ text: selection, result: '', loading: true, error: '' });

    try {
      const status: any = await invoke('ai_status');
      if (!status.sidecar_running) {
        if (!status.model_exists && !status.downloading) {
          invoke('ai_download_model').catch(() => {});
        }
        setAiExplain(prev => prev ? { ...prev, loading: false, error: status.downloading ? 'AI 模型正在下载中...' : 'AI 服务未就绪' } : null);
        return;
      }

      const resp: string = await invoke('ai_chat', {
        systemPrompt: '用简洁的中文解释以下终端输出的含义，如果是错误请给出可能的原因和解决方法。',
        userMessage: selection,
        maxTokens: 256,
      });
      setAiExplain(prev => prev ? { ...prev, result: resp, loading: false } : null);
    } catch (e: any) {
      const msg = typeof e === 'string' ? e : e.message || 'AI 请求失败';
      setAiExplain(prev => prev ? { ...prev, loading: false, error: msg } : null);
    }
  }
```

- [ ] **Step 2: 右键菜单加"AI 解释"项**

在 `contextMenuItems` 数组中，`{ label: '搜索 (Ctrl+Shift+F)', ... }` 的前面加：

```tsx
    { label: 'AI 解释', onClick: handleAiExplain, disabled: !termRef.current?.getSelection() },
    { label: '', onClick: () => {}, separator: true },
```

- [ ] **Step 3: 加 AI 解释浮窗 UI**

在 TerminalPane 的 return JSX 里，`{contextMenu && (` 块之后、组件末尾的 `</>` 之前加：

```tsx
      {aiExplain && (
        <div style={{
          position: 'absolute', bottom: 8, right: 8, width: '360px', maxHeight: '300px',
          background: '#1c2128', border: '1px solid #30363d', borderRadius: '8px',
          padding: '10px', zIndex: 40, overflowY: 'auto', boxShadow: '0 4px 12px rgba(0,0,0,0.4)',
        }}>
          <div style={{ display: 'flex', justifyContent: 'space-between', marginBottom: '6px' }}>
            <span style={{ color: '#00d4ff', fontSize: '12px', fontWeight: 600 }}>AI 解释</span>
            <span
              onClick={() => setAiExplain(null)}
              style={{ color: '#8b949e', cursor: 'pointer', fontSize: '14px' }}
            >×</span>
          </div>
          <div style={{
            background: '#0d1117', borderRadius: '4px', padding: '6px 8px', marginBottom: '6px',
            fontSize: '11px', color: '#8b949e', fontFamily: 'monospace', maxHeight: '60px',
            overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'pre-wrap',
          }}>
            {aiExplain.text.substring(0, 200)}{aiExplain.text.length > 200 ? '...' : ''}
          </div>
          {aiExplain.loading && (
            <div style={{ color: '#8b949e', fontSize: '12px' }}>正在分析...</div>
          )}
          {aiExplain.error && (
            <div style={{ color: '#f85149', fontSize: '12px' }}>{aiExplain.error}</div>
          )}
          {aiExplain.result && (
            <div style={{ color: '#e6edf3', fontSize: '12px', lineHeight: 1.5, whiteSpace: 'pre-wrap' }}>
              {aiExplain.result}
            </div>
          )}
        </div>
      )}
```

- [ ] **Step 4: 编译验证**

Run:
```bash
npm run build 2>&1 | tail -3 && touch src-tauri/build.rs && cd src-tauri && cargo build 2>&1 | tail -3
```
Expected: 前后端均编译成功

- [ ] **Step 5: 提交**

```bash
git add src/components/Terminal/TerminalPane.tsx
git commit -m "feat(ai): 终端右键'AI 解释'功能"
```

---

### Task 5: 158BitNet 构建集成

**Files:**
- Modify: `build.sh`

- [ ] **Step 1: 克隆 158BitNet 到项目目录**

Run:
```bash
cd /home/lfl/ssd/code/guishell
git clone https://github.com/primagen-agent/158BitNet.git 158BitNet
echo "158BitNet/" >> .gitignore
```

- [ ] **Step 2: 编译 openai_server**

Run:
```bash
cd /home/lfl/ssd/code/guishell/158BitNet
cmake -S . -B build -DCMAKE_BUILD_TYPE=Release
cmake --build build --target openai_server -j $(nproc)
ls -lh build/openai_server
```

Expected: `build/openai_server` 二进制生成成功

- [ ] **Step 3: 在 build.sh 加 158BitNet 构建步骤**

在 `build.sh` 的 `echo "=== 编译后端 ==="` 之前加：

```bash
echo "=== 编译 AI sidecar (158BitNet) ==="
if [ -d "158BitNet" ]; then
  cd 158BitNet
  cmake -S . -B build -DCMAKE_BUILD_TYPE=Release 2>&1 | tail -3
  cmake --build build --target openai_server -j $(nproc) 2>&1 | tail -3
  ls -lh build/openai_server
  cd ..
else
  echo "跳过：158BitNet 目录不存在"
fi
```

- [ ] **Step 4: 测试完整构建**

Run: `./build.sh`
Expected: AI sidecar 和主程序均编译成功

- [ ] **Step 5: 提交**

```bash
git add build.sh .gitignore
git commit -m "feat(ai): build.sh 集成 158BitNet openai_server 编译"
```

---

### Task 6: 端到端测试

- [ ] **Step 1: 下载测试模型**

Run:
```bash
mkdir -p ~/.config/guishell/models
# 如果网络允许，从 HuggingFace 下载 0.5B 模型（约 250MB）
# 否则可以先手动放置任意 GGUF 文件做 smoke test
```

- [ ] **Step 2: 手动测试 sidecar**

Run:
```bash
cd /home/lfl/ssd/code/guishell
./158BitNet/build/openai_server ~/.config/guishell/models/bitcpm4-0.5b-tq2_0.gguf --host 127.0.0.1 --port 18573 --ctx 2048 --max-tokens 256
```

在另一个终端验证：
```bash
curl http://127.0.0.1:18573/health
curl -X POST http://127.0.0.1:18573/v1/chat/completions \
  -H "Content-Type: application/json" \
  -d '{"messages":[{"role":"system","content":"只返回命令"},{"role":"user","content":"查看磁盘使用率"}],"max_tokens":64}'
```

Expected: health 返回 `{"status":"ok"}`，chat 返回含 `choices[0].message.content` 的 JSON

- [ ] **Step 3: 启动 LiteTerm 测试 AI 功能**

Run: `./run.sh`

测试：
1. 按 `Ctrl+I` → AI 输入框弹出
2. 输入"查看 80 端口占用" → 点"生成" → 看到命令结果
3. 点"执行" → 命令注入终端
4. 在终端里选中一段文本 → 右键 → "AI 解释" → 浮窗显示解释
5. 查 `~/guishell.log` 确认 sidecar 启动/调用日志

- [ ] **Step 4: 提交最终状态**

如果测试通过：
```bash
git add -A
git commit -m "feat(ai): 158BitNet 本地 AI 集成完成 — 自然语言→命令 + 终端输出解释"
```
