# 设置标签页 实现计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 将设置从模态弹窗改为独立标签页（对标 Tabby），左侧分类导航，右侧详细配置，配色方案用全宽终端预览列表展示。

**Architecture:** 新增 `settings` 标签页类型，在 App.tsx 中和 process/recording 标签一样处理。设置标签页由 `SettingsTab` 主组件管理，左侧导航切换 7 个分区组件。settings.toml 配置通过 get_settings/update_settings Tauri 命令读写。

**Tech Stack:** React/TypeScript, Tailwind CSS, Tauri 2 (Rust)

## Global Constraints

- 构建统一用 `./build.sh`，禁止 `npm run build` 或 `cargo build`
- 界面文本中文，commit 信息中文
- 不做 git 操作，等 review 后由用户决定
- 遵循现有 Tailwind 类名风格（bg-surface, bg-surface-light, border-surface-border, text-gray-*, text-accent-cyan）

---

### Task 1: Tauri 命令 get_settings / update_settings + Tab 类型扩展

**Files:**
- Modify: `src-tauri/src/commands/terminal.rs` — 新增 get_settings / update_settings 命令
- Modify: `src-tauri/src/lib.rs` — 注册新命令
- Modify: `src/types/index.ts` — Tab.type 加入 `'settings'`

**Interfaces:**
- Produces:
  - Tauri command `get_settings` → 返回 Settings 的 JSON
  - Tauri command `update_settings(patch: serde_json::Value)` → 合并更新并写入 settings.toml
  - `Tab.type` 包含 `'settings'`

- [ ] **Step 1: 在 terminal.rs 底部添加 get_settings / update_settings**

```rust
#[tauri::command]
pub async fn get_settings(state: State<'_, AppState>) -> Result<serde_json::Value, String> {
    let settings = state.settings.lock().unwrap();
    serde_json::to_value(&*settings).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn update_settings(state: State<'_, AppState>, patch: serde_json::Value) -> Result<(), String> {
    let mut settings = state.settings.lock().unwrap();
    // 读取当前值为 JSON，合并 patch，再反序列化回 Settings
    let mut current = serde_json::to_value(&*settings).map_err(|e| e.to_string())?;
    if let (Some(cur_obj), Some(patch_obj)) = (current.as_object_mut(), patch.as_object()) {
        for (key, val) in patch_obj {
            if let Some(existing) = cur_obj.get_mut(key) {
                if let (Some(e), Some(v)) = (existing.as_object_mut(), val.as_object()) {
                    for (k2, v2) in v {
                        e.insert(k2.clone(), v2.clone());
                    }
                } else {
                    cur_obj.insert(key.clone(), val.clone());
                }
            } else {
                cur_obj.insert(key.clone(), val.clone());
            }
        }
    }
    let updated: crate::config::settings::Settings = serde_json::from_value(current).map_err(|e| e.to_string())?;
    updated.save().map_err(|e| e.to_string())?;
    *settings = updated;
    Ok(())
}
```

- [ ] **Step 2: 在 lib.rs 的 invoke_handler 中注册**

在 `commands::terminal::unregister_tab,` 后追加：

```rust
commands::terminal::get_settings,
commands::terminal::update_settings,
```

- [ ] **Step 3: 修改 types/index.ts 的 Tab.type**

```typescript
type: 'local' | 'ssh' | 'process' | 'serial' | 'recording' | 'settings';
```

- [ ] **Step 4: 验证编译**

```bash
cd src-tauri && cargo check
npx tsc --noEmit
```

---

### Task 2: ThemePreview 组件 + SettingsTab 主组件骨架

**Files:**
- Create: `src/components/settings/ThemePreview.tsx`
- Create: `src/components/settings/SettingsTab.tsx`

**Interfaces:**
- Consumes: `TERMINAL_THEMES` from `../../themes`
- Produces:
  - `ThemePreview` 组件: `{ name: string, theme: ITheme, selected: boolean, onClick: () => void }`
  - `SettingsTab` 组件: `{ onApply: () => void }`

- [ ] **Step 1: 创建 ThemePreview.tsx**

```tsx
import type { ITheme } from '@xterm/xterm';

interface Props {
  name: string;
  theme: ITheme;
  selected: boolean;
  onClick: () => void;
}

export function ThemePreview({ name, theme, selected, onClick }: Props) {
  const colors = [
    theme.black, theme.red, theme.green, theme.yellow,
    theme.blue, theme.magenta, theme.cyan, theme.white,
    theme.brightBlack, theme.brightRed,
  ];

  return (
    <div
      onClick={onClick}
      className={`cursor-pointer rounded-lg overflow-hidden border-2 transition-colors ${
        selected ? 'border-accent-cyan' : 'border-transparent hover:border-surface-border'
      }`}
    >
      {/* 标题行：名称 + 色点 */}
      <div className="flex items-center justify-between px-3 py-1.5 bg-surface-light">
        <div className="flex items-center gap-2">
          {selected && <span className="text-accent-cyan text-sm">✓</span>}
          <span className={`text-sm ${selected ? 'text-accent-cyan' : 'text-gray-300'}`}>{name}</span>
        </div>
        <div className="flex gap-0.5">
          {colors.map((c, i) => (
            <span
              key={i}
              className="w-2.5 h-2.5 rounded-full inline-block"
              style={{ backgroundColor: c || '#888' }}
            />
          ))}
        </div>
      </div>
      {/* 终端预览 */}
      <div
        style={{
          backgroundColor: theme.background || '#000',
          fontFamily: "'Ubuntu Mono', 'DejaVu Sans Mono', monospace",
          fontSize: '12px',
          lineHeight: 1.4,
          padding: '6px 10px',
        }}
      >
        <div>
          <span style={{ color: theme.green || '#0f0' }}>john</span>
          <span style={{ color: theme.foreground || '#fff' }}>@</span>
          <span style={{ color: theme.blue || '#00f' }}>doe-pc</span>
          <span style={{ color: theme.foreground || '#fff' }}>$ </span>
          <span style={{ color: theme.foreground || '#fff' }}>ls</span>
        </div>
        <div>
          <span style={{ color: theme.foreground || '#fff' }}>-rwxr-xr-x 1 root </span>
          <span style={{ color: theme.green || '#0f0' }}>Documents</span>
        </div>
        <div>
          <span style={{ color: theme.foreground || '#fff' }}>-rwxr-xr-x 1 root </span>
          <span style={{ backgroundColor: theme.yellow || '#ff0', color: theme.black || '#000' }}>Downloads</span>
        </div>
        <div>
          <span style={{ color: theme.foreground || '#fff' }}>-rwxr-xr-x 1 root </span>
          <span style={{ color: theme.cyan || '#0ff' }}>Pictures</span>
        </div>
        <div>
          <span style={{ color: theme.foreground || '#fff' }}>-rwxr-xr-x 1 root </span>
          <span style={{ color: theme.magenta || '#f0f' }}>Music</span>
        </div>
      </div>
    </div>
  );
}
```

- [ ] **Step 2: 创建 SettingsTab.tsx 骨架**

```tsx
import { useState } from 'react';

const SECTIONS = [
  { key: 'terminal', label: '终端', icon: '⌨' },
  { key: 'appearance', label: '外观', icon: '🎨' },
  { key: 'shortcuts', label: '快捷键', icon: '⌘' },
  { key: 'ssh', label: 'SSH', icon: '🔒' },
  { key: 'transfer', label: '传输', icon: '📁' },
  { key: 'zmodem', label: 'ZMODEM', icon: '📡' },
  { key: 'about', label: '关于', icon: 'ℹ' },
] as const;

type SectionKey = typeof SECTIONS[number]['key'];

interface Props {
  onApply: () => void;
}

export function SettingsTab({ onApply }: Props) {
  const [activeSection, setActiveSection] = useState<SectionKey>('terminal');

  return (
    <div className="flex h-full bg-surface">
      {/* 左侧导航 */}
      <div className="w-[180px] border-r border-surface-border bg-surface-light flex flex-col py-2">
        {SECTIONS.map(s => (
          <button
            key={s.key}
            onClick={() => setActiveSection(s.key)}
            className={`flex items-center gap-2 px-4 py-2 text-sm text-left transition-colors ${
              activeSection === s.key
                ? 'text-accent-cyan bg-accent-cyan/10 border-r-2 border-accent-cyan'
                : 'text-gray-400 hover:text-gray-200 hover:bg-surface-lighter'
            }`}
          >
            <span className="text-base">{s.icon}</span>
            {s.label}
          </button>
        ))}
      </div>
      {/* 右侧内容 */}
      <div className="flex-1 overflow-auto p-6">
        {activeSection === 'terminal' && <div className="text-gray-400">终端设置（Task 3 实现）</div>}
        {activeSection === 'appearance' && <div className="text-gray-400">外观设置（Task 4 实现）</div>}
        {activeSection === 'shortcuts' && <div className="text-gray-400">快捷键设置（Task 4 实现）</div>}
        {activeSection === 'ssh' && <div className="text-gray-400">SSH 设置（Task 4 实现）</div>}
        {activeSection === 'transfer' && <div className="text-gray-400">传输设置（Task 4 实现）</div>}
        {activeSection === 'zmodem' && <div className="text-gray-400">ZMODEM 设置（Task 4 实现）</div>}
        {activeSection === 'about' && <div className="text-gray-400">关于（Task 4 实现）</div>}
      </div>
    </div>
  );
}
```

- [ ] **Step 3: 验证 TypeScript 编译**

```bash
npx tsc --noEmit
```

---

### Task 3: TerminalSection（配色预览列表 + 字体/字号）

**Files:**
- Create: `src/components/settings/TerminalSection.tsx`
- Modify: `src/components/settings/SettingsTab.tsx` — 替换占位为实际组件

**Interfaces:**
- Consumes: `ThemePreview`, `TERMINAL_THEMES`, `getTerminalFont`, `getTerminalFontSize`, `getTerminalFontFamily` from `../Settings`
- Produces: `TerminalSection` 组件: `{ onApply: () => void }`

- [ ] **Step 1: 创建 TerminalSection.tsx**

```tsx
import { useState, useEffect } from 'react';
import { TERMINAL_THEMES } from '../../themes';
import { ThemePreview } from './ThemePreview';
import { getTerminalFont, getTerminalFontSize } from '../Settings';

const FONT_OPTIONS = [
  'JetBrains Mono', 'Fira Code', 'Source Code Pro',
  'Ubuntu Mono', 'DejaVu Sans Mono', 'Liberation Mono', 'Noto Sans Mono',
  'Cascadia Code', 'Consolas', 'Courier New',
  'SF Mono', 'Menlo', 'Monaco', 'monospace',
];

interface Props {
  onApply: () => void;
}

export function TerminalSection({ onApply }: Props) {
  const [theme, setTheme] = useState(() => localStorage.getItem('guishell_terminal_theme') || '暗色默认');
  const [useSystemFont, setUseSystemFont] = useState(() => localStorage.getItem('guishell_use_system_font') === 'true');
  const [font, setFont] = useState(getTerminalFont);
  const [fontSize, setFontSize] = useState(getTerminalFontSize);
  const [search, setSearch] = useState('');

  const allThemeNames = Object.keys(TERMINAL_THEMES);
  const filteredNames = search
    ? allThemeNames.filter(n => n.toLowerCase().includes(search.toLowerCase()))
    : allThemeNames;

  useEffect(() => {
    const timer = setTimeout(() => {
      localStorage.setItem('guishell_terminal_theme', theme);
      localStorage.setItem('guishell_use_system_font', String(useSystemFont));
      localStorage.setItem('guishell_terminal_font', useSystemFont ? 'monospace' : font);
      localStorage.setItem('guishell_terminal_fontsize', String(fontSize));
      onApply();
    }, 150);
    return () => clearTimeout(timer);
  }, [theme, useSystemFont, font, fontSize, onApply]);

  return (
    <div className="space-y-6">
      <h2 className="text-lg font-semibold text-gray-200">终端</h2>

      {/* 当前配色 */}
      <div>
        <div className="flex items-center justify-between mb-2">
          <span className="text-sm text-gray-400">当前配色方案: <span className="text-accent-cyan">{theme}</span></span>
          <span className="text-xs text-gray-500">{allThemeNames.length} 套</span>
        </div>
        <ThemePreview
          name={theme}
          theme={TERMINAL_THEMES[theme] || TERMINAL_THEMES['暗色默认']}
          selected={true}
          onClick={() => {}}
        />
      </div>

      {/* 搜索 */}
      <input
        value={search}
        onChange={e => setSearch(e.target.value)}
        placeholder="搜索配色方案..."
        className="w-full bg-surface border border-surface-border rounded px-3 py-1.5 text-sm text-gray-200 outline-none focus:border-accent-cyan"
      />

      {/* 配色列表 */}
      <div className="space-y-3 max-h-[500px] overflow-auto">
        {filteredNames.map(name => (
          <ThemePreview
            key={name}
            name={name}
            theme={TERMINAL_THEMES[name]}
            selected={theme === name}
            onClick={() => setTheme(name)}
          />
        ))}
      </div>

      {/* 字体设置 */}
      <div className="border-t border-surface-border pt-4 space-y-4">
        <label className="flex items-center gap-2 cursor-pointer">
          <input
            type="checkbox"
            checked={useSystemFont}
            onChange={e => setUseSystemFont(e.target.checked)}
            className="accent-accent-cyan"
          />
          <span className="text-sm text-gray-300">使用系统字体</span>
        </label>

        <div style={{ opacity: useSystemFont ? 0.4 : 1, pointerEvents: useSystemFont ? 'none' : 'auto' }}
             className="space-y-3">
          <div>
            <label className="text-sm text-gray-400 block mb-1">终端字体</label>
            <select
              value={font}
              onChange={e => setFont(e.target.value)}
              className="w-full bg-surface border border-surface-border rounded px-3 py-1.5 text-sm text-gray-200 outline-none focus:border-accent-cyan max-w-xs"
            >
              {FONT_OPTIONS.map(f => <option key={f} value={f}>{f}</option>)}
            </select>
          </div>
          <div>
            <label className="text-sm text-gray-400 block mb-1">字号 (8-48)</label>
            <div className="flex items-center gap-2">
              <button onClick={() => setFontSize(s => Math.max(8, s - 1))}
                className="w-7 h-7 flex items-center justify-center border border-surface-border rounded text-gray-300 hover:bg-surface-lighter">−</button>
              <span className="text-sm text-gray-200 w-8 text-center">{fontSize}</span>
              <button onClick={() => setFontSize(s => Math.min(48, s + 1))}
                className="w-7 h-7 flex items-center justify-center border border-surface-border rounded text-gray-300 hover:bg-surface-lighter">+</button>
              <input type="range" min={8} max={48} value={fontSize}
                onChange={e => setFontSize(parseInt(e.target.value))} className="flex-1 max-w-[200px]" />
            </div>
          </div>
        </div>
      </div>
    </div>
  );
}
```

- [ ] **Step 2: 更新 SettingsTab.tsx — 导入并使用 TerminalSection**

在文件顶部添加 import：
```tsx
import { TerminalSection } from './TerminalSection';
```

替换终端占位行：
```tsx
{activeSection === 'terminal' && <TerminalSection onApply={onApply} />}
```

- [ ] **Step 3: 验证 TypeScript 编译**

```bash
npx tsc --noEmit
```

---

### Task 4: 其余 6 个分区组件 + App.tsx 集成

**Files:**
- Create: `src/components/settings/AppearanceSection.tsx`
- Create: `src/components/settings/ShortcutsSection.tsx`
- Create: `src/components/settings/SshSection.tsx`
- Create: `src/components/settings/TransferSection.tsx`
- Create: `src/components/settings/ZmodemSection.tsx`
- Create: `src/components/settings/AboutSection.tsx`
- Modify: `src/components/settings/SettingsTab.tsx` — 导入所有分区
- Modify: `src/App.tsx` — 齿轮按钮打开设置标签 + 渲染 SettingsTab

**Interfaces:**
- Consumes: Tauri commands `get_settings`, `update_settings`, `get_system_info`; `ShortcutConfig`/`loadShortcuts`/`saveShortcuts` from `../ShortcutSettings`
- Produces: 完整可用的设置标签页

- [ ] **Step 1: 创建 AppearanceSection.tsx**

```tsx
import { useState, useEffect } from 'react';
import { invoke } from '@tauri-apps/api/core';

interface Props { onApply: () => void; }

export function AppearanceSection({ onApply }: Props) {
  const [sidebarWidth, setSidebarWidth] = useState(220);
  const [fileBrowserHeight, setFileBrowserHeight] = useState(200);
  const [showSidebar, setShowSidebar] = useState(true);
  const [showFileBrowser, setShowFileBrowser] = useState(true);
  const [loaded, setLoaded] = useState(false);

  useEffect(() => {
    invoke<any>('get_settings').then(s => {
      const a = s.appearance || {};
      setSidebarWidth(a.sidebar_width ?? 220);
      setFileBrowserHeight(a.file_browser_height ?? 200);
      setShowSidebar(a.show_sidebar ?? true);
      setShowFileBrowser(a.show_file_browser ?? true);
      setLoaded(true);
    }).catch(() => setLoaded(true));
  }, []);

  useEffect(() => {
    if (!loaded) return;
    const timer = setTimeout(() => {
      invoke('update_settings', { patch: { appearance: {
        sidebar_width: sidebarWidth, file_browser_height: fileBrowserHeight,
        show_sidebar: showSidebar, show_file_browser: showFileBrowser,
      }}}).catch(() => {});
      onApply();
    }, 300);
    return () => clearTimeout(timer);
  }, [sidebarWidth, fileBrowserHeight, showSidebar, showFileBrowser, loaded, onApply]);

  return (
    <div className="space-y-6">
      <h2 className="text-lg font-semibold text-gray-200">外观</h2>
      <div className="space-y-4">
        <label className="flex items-center gap-2 cursor-pointer">
          <input type="checkbox" checked={showSidebar} onChange={e => setShowSidebar(e.target.checked)} className="accent-accent-cyan" />
          <span className="text-sm text-gray-300">显示侧边栏</span>
        </label>
        <div>
          <label className="text-sm text-gray-400 block mb-1">侧边栏宽度 ({sidebarWidth}px)</label>
          <input type="range" min={100} max={400} value={sidebarWidth} onChange={e => setSidebarWidth(parseInt(e.target.value))} className="w-full max-w-xs" />
        </div>
        <label className="flex items-center gap-2 cursor-pointer">
          <input type="checkbox" checked={showFileBrowser} onChange={e => setShowFileBrowser(e.target.checked)} className="accent-accent-cyan" />
          <span className="text-sm text-gray-300">显示文件管理器</span>
        </label>
        <div>
          <label className="text-sm text-gray-400 block mb-1">文件管理器高度 ({fileBrowserHeight}px)</label>
          <input type="range" min={100} max={500} value={fileBrowserHeight} onChange={e => setFileBrowserHeight(parseInt(e.target.value))} className="w-full max-w-xs" />
        </div>
      </div>
    </div>
  );
}
```

- [ ] **Step 2: 创建 ShortcutsSection.tsx**

```tsx
import { useState } from 'react';
import { loadShortcuts, saveShortcuts, DEFAULT_SHORTCUTS, type ShortcutConfig } from '../ShortcutSettings';

interface Props { onApply: () => void; }

const LABELS: Record<keyof ShortcutConfig, string> = {
  newTab: '新建标签', closeTab: '关闭标签', search: '搜索',
  splitH: '水平分屏', splitV: '垂直分屏', nextTab: '下一标签',
  prevTab: '上一标签', copyText: '复制', pasteText: '粘贴',
};

export function ShortcutsSection({ onApply }: Props) {
  const [shortcuts, setShortcuts] = useState<ShortcutConfig>(loadShortcuts);
  const [recording, setRecording] = useState<keyof ShortcutConfig | null>(null);

  function handleKeyDown(e: React.KeyboardEvent, key: keyof ShortcutConfig) {
    e.preventDefault();
    const parts: string[] = [];
    if (e.ctrlKey) parts.push('Ctrl');
    if (e.shiftKey) parts.push('Shift');
    if (e.altKey) parts.push('Alt');
    if (!['Control', 'Shift', 'Alt', 'Meta'].includes(e.key)) {
      parts.push(e.key.length === 1 ? e.key.toUpperCase() : e.key);
      const combo = parts.join('+');
      const updated = { ...shortcuts, [key]: combo };
      setShortcuts(updated);
      saveShortcuts(updated);
      setRecording(null);
      onApply();
    }
  }

  return (
    <div className="space-y-6">
      <h2 className="text-lg font-semibold text-gray-200">快捷键</h2>
      <div className="space-y-2">
        {(Object.keys(LABELS) as Array<keyof ShortcutConfig>).map(key => (
          <div key={key} className="flex items-center justify-between py-1.5">
            <span className="text-sm text-gray-300">{LABELS[key]}</span>
            <button
              onKeyDown={e => handleKeyDown(e, key)}
              onClick={() => setRecording(recording === key ? null : key)}
              className={`px-3 py-1 text-xs border rounded min-w-[140px] text-center ${
                recording === key
                  ? 'border-accent-cyan text-accent-cyan bg-accent-cyan/10'
                  : 'border-surface-border text-gray-400 hover:border-gray-500'
              }`}
            >
              {recording === key ? '按下快捷键...' : shortcuts[key]}
            </button>
          </div>
        ))}
      </div>
      <button
        onClick={() => { setShortcuts({ ...DEFAULT_SHORTCUTS }); saveShortcuts(DEFAULT_SHORTCUTS); onApply(); }}
        className="text-xs text-gray-500 hover:text-gray-300"
      >
        恢复默认快捷键
      </button>
    </div>
  );
}
```

- [ ] **Step 3: 创建 SshSection.tsx**

```tsx
import { useState, useEffect } from 'react';
import { invoke } from '@tauri-apps/api/core';

interface Props { onApply: () => void; }

export function SshSection({ onApply }: Props) {
  const [connectTimeout, setConnectTimeout] = useState(10);
  const [keepalive, setKeepalive] = useState(30);
  const [charset, setCharset] = useState('UTF-8');
  const [loaded, setLoaded] = useState(false);

  useEffect(() => {
    invoke<any>('get_settings').then(s => {
      const ssh = s.ssh || {};
      setConnectTimeout(ssh.connect_timeout_secs ?? 10);
      setKeepalive(ssh.keepalive_interval_secs ?? 30);
      setCharset(ssh.default_charset ?? 'UTF-8');
      setLoaded(true);
    }).catch(() => setLoaded(true));
  }, []);

  useEffect(() => {
    if (!loaded) return;
    const timer = setTimeout(() => {
      invoke('update_settings', { patch: { ssh: {
        connect_timeout_secs: connectTimeout, keepalive_interval_secs: keepalive, default_charset: charset,
      }}}).catch(() => {});
      onApply();
    }, 300);
    return () => clearTimeout(timer);
  }, [connectTimeout, keepalive, charset, loaded, onApply]);

  return (
    <div className="space-y-6">
      <h2 className="text-lg font-semibold text-gray-200">SSH</h2>
      <div className="space-y-4">
        <div>
          <label className="text-sm text-gray-400 block mb-1">连接超时 (秒)</label>
          <input type="number" min={1} max={120} value={connectTimeout}
            onChange={e => setConnectTimeout(Math.max(1, parseInt(e.target.value) || 10))}
            className="bg-surface border border-surface-border rounded px-3 py-1.5 text-sm text-gray-200 outline-none focus:border-accent-cyan w-24" />
        </div>
        <div>
          <label className="text-sm text-gray-400 block mb-1">Keepalive 间隔 (秒)</label>
          <input type="number" min={0} max={300} value={keepalive}
            onChange={e => setKeepalive(Math.max(0, parseInt(e.target.value) || 30))}
            className="bg-surface border border-surface-border rounded px-3 py-1.5 text-sm text-gray-200 outline-none focus:border-accent-cyan w-24" />
        </div>
        <div>
          <label className="text-sm text-gray-400 block mb-1">默认字符集</label>
          <select value={charset} onChange={e => setCharset(e.target.value)}
            className="bg-surface border border-surface-border rounded px-3 py-1.5 text-sm text-gray-200 outline-none focus:border-accent-cyan">
            <option value="UTF-8">UTF-8</option>
            <option value="GBK">GBK</option>
            <option value="GB2312">GB2312</option>
          </select>
        </div>
      </div>
    </div>
  );
}
```

- [ ] **Step 4: 创建 TransferSection.tsx**

```tsx
import { useState, useEffect } from 'react';
import { invoke } from '@tauri-apps/api/core';

interface Props { onApply: () => void; }

export function TransferSection({ onApply }: Props) {
  const [downloadDir, setDownloadDir] = useState('~/Downloads');
  const [resumeThreshold, setResumeThreshold] = useState(10);
  const [maxRetries, setMaxRetries] = useState(3);
  const [concurrent, setConcurrent] = useState(2);
  const [loaded, setLoaded] = useState(false);

  useEffect(() => {
    invoke<any>('get_settings').then(s => {
      const t = s.transfer || {};
      setDownloadDir(t.default_download_dir ?? '~/Downloads');
      setResumeThreshold(t.resume_threshold_mb ?? 10);
      setMaxRetries(t.max_retries ?? 3);
      setConcurrent(t.concurrent_transfers ?? 2);
      setLoaded(true);
    }).catch(() => setLoaded(true));
  }, []);

  useEffect(() => {
    if (!loaded) return;
    const timer = setTimeout(() => {
      invoke('update_settings', { patch: { transfer: {
        default_download_dir: downloadDir, resume_threshold_mb: resumeThreshold,
        max_retries: maxRetries, concurrent_transfers: concurrent,
      }}}).catch(() => {});
      onApply();
    }, 300);
    return () => clearTimeout(timer);
  }, [downloadDir, resumeThreshold, maxRetries, concurrent, loaded, onApply]);

  return (
    <div className="space-y-6">
      <h2 className="text-lg font-semibold text-gray-200">传输</h2>
      <div className="space-y-4">
        <div>
          <label className="text-sm text-gray-400 block mb-1">默认下载目录</label>
          <input type="text" value={downloadDir} onChange={e => setDownloadDir(e.target.value)}
            className="w-full bg-surface border border-surface-border rounded px-3 py-1.5 text-sm text-gray-200 outline-none focus:border-accent-cyan max-w-md" />
        </div>
        <div>
          <label className="text-sm text-gray-400 block mb-1">断点续传阈值 (MB)</label>
          <input type="number" min={1} max={1000} value={resumeThreshold}
            onChange={e => setResumeThreshold(Math.max(1, parseInt(e.target.value) || 10))}
            className="bg-surface border border-surface-border rounded px-3 py-1.5 text-sm text-gray-200 outline-none focus:border-accent-cyan w-24" />
        </div>
        <div>
          <label className="text-sm text-gray-400 block mb-1">最大重试次数</label>
          <input type="number" min={0} max={10} value={maxRetries}
            onChange={e => setMaxRetries(Math.max(0, parseInt(e.target.value) || 3))}
            className="bg-surface border border-surface-border rounded px-3 py-1.5 text-sm text-gray-200 outline-none focus:border-accent-cyan w-24" />
        </div>
        <div>
          <label className="text-sm text-gray-400 block mb-1">并发传输数 (1-8)</label>
          <input type="number" min={1} max={8} value={concurrent}
            onChange={e => setConcurrent(Math.min(8, Math.max(1, parseInt(e.target.value) || 2)))}
            className="bg-surface border border-surface-border rounded px-3 py-1.5 text-sm text-gray-200 outline-none focus:border-accent-cyan w-24" />
        </div>
      </div>
    </div>
  );
}
```

- [ ] **Step 5: 创建 ZmodemSection.tsx**

```tsx
import { useState, useEffect } from 'react';
import { invoke } from '@tauri-apps/api/core';

interface Props { onApply: () => void; }

export function ZmodemSection({ onApply }: Props) {
  const [enabled, setEnabled] = useState(true);
  const [autoDetect, setAutoDetect] = useState(true);
  const [downloadDir, setDownloadDir] = useState('~/Downloads');
  const [timeout, setTimeoutSecs] = useState(60);
  const [loaded, setLoaded] = useState(false);

  useEffect(() => {
    invoke<any>('get_settings').then(s => {
      const z = s.zmodem || {};
      setEnabled(z.enabled ?? true);
      setAutoDetect(z.auto_detect ?? true);
      setDownloadDir(z.download_dir ?? '~/Downloads');
      setTimeoutSecs(z.timeout_secs ?? 60);
      setLoaded(true);
    }).catch(() => setLoaded(true));
  }, []);

  useEffect(() => {
    if (!loaded) return;
    const timer = setTimeout(() => {
      invoke('update_settings', { patch: { zmodem: {
        enabled, auto_detect: autoDetect, download_dir: downloadDir, timeout_secs: timeout,
      }}}).catch(() => {});
      onApply();
    }, 300);
    return () => clearTimeout(timer);
  }, [enabled, autoDetect, downloadDir, timeout, loaded, onApply]);

  return (
    <div className="space-y-6">
      <h2 className="text-lg font-semibold text-gray-200">ZMODEM</h2>
      <div className="space-y-4">
        <label className="flex items-center gap-2 cursor-pointer">
          <input type="checkbox" checked={enabled} onChange={e => setEnabled(e.target.checked)} className="accent-accent-cyan" />
          <span className="text-sm text-gray-300">启用 ZMODEM</span>
        </label>
        <label className="flex items-center gap-2 cursor-pointer">
          <input type="checkbox" checked={autoDetect} onChange={e => setAutoDetect(e.target.checked)} className="accent-accent-cyan" />
          <span className="text-sm text-gray-300">自动检测</span>
        </label>
        <div>
          <label className="text-sm text-gray-400 block mb-1">下载目录</label>
          <input type="text" value={downloadDir} onChange={e => setDownloadDir(e.target.value)}
            className="w-full bg-surface border border-surface-border rounded px-3 py-1.5 text-sm text-gray-200 outline-none focus:border-accent-cyan max-w-md" />
        </div>
        <div>
          <label className="text-sm text-gray-400 block mb-1">超时时间 (秒)</label>
          <input type="number" min={10} max={600} value={timeout}
            onChange={e => setTimeoutSecs(Math.max(10, parseInt(e.target.value) || 60))}
            className="bg-surface border border-surface-border rounded px-3 py-1.5 text-sm text-gray-200 outline-none focus:border-accent-cyan w-24" />
        </div>
      </div>
    </div>
  );
}
```

- [ ] **Step 6: 创建 AboutSection.tsx**

```tsx
import { useState, useEffect } from 'react';
import { invoke } from '@tauri-apps/api/core';

export function AboutSection() {
  const [info, setInfo] = useState<any>(null);

  useEffect(() => {
    invoke<any>('get_system_info').then(setInfo).catch(() => {});
  }, []);

  return (
    <div className="space-y-6">
      <h2 className="text-lg font-semibold text-gray-200">关于 LiteTerm</h2>
      {info && (
        <div className="space-y-3">
          <div className="flex items-center gap-3">
            <span className="text-2xl">⌨</span>
            <div>
              <div className="text-lg text-gray-200 font-semibold">LiteTerm</div>
              <div className="text-sm text-gray-400">轻量级跨平台 SSH 客户端</div>
            </div>
          </div>
          <div className="bg-surface-light rounded-lg p-4 space-y-2 text-sm">
            <div className="flex justify-between"><span className="text-gray-400">版本</span><span className="text-gray-200">v{info.app_version}</span></div>
            <div className="flex justify-between"><span className="text-gray-400">操作系统</span><span className="text-gray-200">{info.os} ({info.arch})</span></div>
            <div className="flex justify-between"><span className="text-gray-400">主机名</span><span className="text-gray-200">{info.hostname}</span></div>
            <div className="flex justify-between"><span className="text-gray-400">用户</span><span className="text-gray-200">{info.username}</span></div>
          </div>
          <div className="text-xs text-gray-500">
            基于 Tauri 2 + React + xterm.js 构建
          </div>
        </div>
      )}
    </div>
  );
}
```

- [ ] **Step 7: 更新 SettingsTab.tsx — 导入所有分区并替换占位**

在 SettingsTab.tsx 顶部添加所有 import：

```tsx
import { TerminalSection } from './TerminalSection';
import { AppearanceSection } from './AppearanceSection';
import { ShortcutsSection } from './ShortcutsSection';
import { SshSection } from './SshSection';
import { TransferSection } from './TransferSection';
import { ZmodemSection } from './ZmodemSection';
import { AboutSection } from './AboutSection';
```

替换右侧内容区的 7 行占位为：

```tsx
{activeSection === 'terminal' && <TerminalSection onApply={onApply} />}
{activeSection === 'appearance' && <AppearanceSection onApply={onApply} />}
{activeSection === 'shortcuts' && <ShortcutsSection onApply={onApply} />}
{activeSection === 'ssh' && <SshSection onApply={onApply} />}
{activeSection === 'transfer' && <TransferSection onApply={onApply} />}
{activeSection === 'zmodem' && <ZmodemSection onApply={onApply} />}
{activeSection === 'about' && <AboutSection />}
```

- [ ] **Step 8: 修改 App.tsx — 集成设置标签页**

8a. 在顶部 import 区域添加：

```tsx
import { SettingsTab } from './components/settings/SettingsTab';
```

8b. 添加打开设置标签函数（放在 `openRecordingTab` 附近）：

```tsx
function openSettingsTab() {
  const existing = tabs.find(t => t.type === 'settings');
  if (existing) {
    setActiveTabId(existing.id);
    return;
  }
  const id = `settings-${Date.now()}`;
  const tab: Tab = { id, label: '设置', type: 'settings' };
  setTabs(prev => [...prev, tab]);
  setActiveTabId(id);
}
```

8c. 将齿轮按钮的 `onClick={() => setShowSettings(true)}` 改为 `onClick={openSettingsTab}`。

8d. 在内容区的渲染逻辑中（`tabs.map((tab) =>` 部分），在 `tab.type === 'recording'` 分支后面添加设置标签页分支：

```tsx
) : tab.type === 'settings' ? (
  <div
    key={tab.id}
    style={{
      display: tab.id === activeTabId ? 'flex' : 'none',
      position: 'absolute',
      inset: 0,
      overflow: 'hidden',
    }}
  >
    <SettingsTab onApply={() => window.dispatchEvent(new Event('terminal-settings-changed'))} />
  </div>
```

8e. 在标签栏图标渲染处（显示标签类型图标的地方），为 settings 类型添加图标：

找到标签栏 tab 渲染区域中类似 `{tab.type === 'ssh' && (...)}` 的图标判断，添加：
```tsx
{tab.type === 'settings' && <span style={{ fontSize: '11px' }}>⚙</span>}
```

8f. 设置标签不应该在关闭时调用 `close_terminal`（它没有后端终端），需要在 `closeTab` 函数中跳过后端清理：

找到 `closeTab` 中的 `invoke('close_terminal', ...)` 调用，在之前添加判断：
```tsx
const closedTab = tabs.find(t => t.id === closedId);
if (closedTab && closedTab.type !== 'settings' && closedTab.type !== 'process') {
  invoke('close_terminal', { id: closedId }).catch(() => {});
  invoke('unregister_tab', { id: closedId }).catch(() => {});
}
```

- [ ] **Step 9: 完整构建验证**

```bash
./build.sh
```

- [ ] **Step 10: 启动并手动测试**

```bash
./run.sh
```

验证：
1. 点击齿轮打开设置标签
2. 左侧 7 个分类导航可切换
3. 配色方案预览列表滚动流畅
4. 选择配色方案后终端实时变化
5. SSH/传输/ZMODEM 设置修改后保存到 settings.toml
6. 快捷键录入正常
7. 关于页显示版本信息
8. 设置标签可关闭，再次打开复用已有标签
