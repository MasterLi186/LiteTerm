# 快捷命令栏 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 在命令输入栏下方新增 XShell 风格的常驻快捷命令按钮栏。

**Architecture:** 新建独立组件 `QuickCommandBar.tsx`，自管理 state 和 localStorage 持久化。App.tsx 仅在 CommandInputBar 下方插入一行渲染，传入 `sendCommand` 回调。右键菜单和编辑浮层均在组件内部实现。

**Tech Stack:** React + TypeScript + Tailwind CSS + localStorage

## Global Constraints

- 所有界面文本为中文
- commit 信息使用中文
- 构建使用 `./build.sh`，禁止单独 `npm run build` 或 `cargo build`
- 样式使用项目现有 Tailwind 类名（`bg-surface`, `bg-surface-light`, `border-surface-border`, `text-accent-*` 等）

---

### Task 1: QuickCommandBar 组件 — 按钮渲染 + 点击执行 + 数据持久化

**Files:**
- Create: `src/components/QuickCommandBar.tsx`
- Modify: `src/App.tsx:1924-1928`

**Interfaces:**
- Consumes: `invoke('terminal_write', { id, data })` from `@tauri-apps/api/core`（通过 props.sendCommand 间接使用）
- Produces: `<QuickCommandBar sendCommand={(cmd: string) => void} />` 组件

- [ ] **Step 1: 创建 QuickCommandBar.tsx 基础组件**

```tsx
// src/components/QuickCommandBar.tsx
import { useState } from 'react';

export interface QuickCommand {
  label: string;
  command: string;
}

const STORAGE_KEY = 'guishell_quick_commands';
const OLD_FAVORITES_KEY = 'guishell_cmd_favorites';

function loadQuickCommands(): QuickCommand[] {
  try {
    const saved = localStorage.getItem(STORAGE_KEY);
    if (saved) return JSON.parse(saved);
    // 迁移旧收藏数据
    const oldFav = localStorage.getItem(OLD_FAVORITES_KEY);
    if (oldFav) {
      const cmds: string[] = JSON.parse(oldFav);
      const migrated = cmds.map(cmd => ({ label: cmd.slice(0, 6), command: cmd }));
      localStorage.setItem(STORAGE_KEY, JSON.stringify(migrated));
      return migrated;
    }
  } catch {}
  return [];
}

function saveQuickCommands(cmds: QuickCommand[]) {
  localStorage.setItem(STORAGE_KEY, JSON.stringify(cmds));
}

interface Props {
  sendCommand: (cmd: string) => void;
}

export function QuickCommandBar({ sendCommand }: Props) {
  const [commands, setCommands] = useState<QuickCommand[]>(loadQuickCommands);

  const updateCommands = (updated: QuickCommand[]) => {
    setCommands(updated);
    saveQuickCommands(updated);
  };

  return (
    <div className="h-7 bg-surface-light border-b border-surface-border flex items-center px-1 gap-1 overflow-x-auto">
      {/* + 按钮 */}
      <button
        className="flex-shrink-0 w-6 h-5 bg-surface border border-surface-border rounded text-xs text-gray-400 hover:bg-surface-lighter hover:text-white"
        title="添加快捷命令"
      >
        +
      </button>
      {/* 命令按钮列表 */}
      {commands.map((cmd, i) => (
        <button
          key={i}
          className="flex-shrink-0 bg-surface border border-surface-border rounded px-2 py-0.5 text-xs text-gray-300 hover:bg-surface-lighter hover:text-white cursor-pointer"
          title={cmd.command}
          onClick={() => sendCommand(cmd.command)}
        >
          {cmd.label}
        </button>
      ))}
    </div>
  );
}
```

- [ ] **Step 2: 在 App.tsx 中引入并渲染**

在 `src/App.tsx` 顶部 import 区域添加：
```tsx
import { QuickCommandBar } from './components/QuickCommandBar';
```

找到 `<CommandInputBar>` 的渲染位置（约 1924 行），在其下方插入 QuickCommandBar：
```tsx
        {/* Command input bar — 连接断开时隐藏 */}
        {!(activeTabId && reconnecting[activeTabId]) && (
          <CommandInputBar
            terminalId={focusedTerminalId || activeTabId}
          />
        )}
        {/* 快捷命令栏 — 连接断开时隐藏 */}
        {!(activeTabId && reconnecting[activeTabId]) && (
          <QuickCommandBar
            sendCommand={(cmd) => {
              const tid = focusedTerminalId || activeTabId;
              if (tid) invoke('terminal_write', { id: tid, data: Array.from(new TextEncoder().encode(cmd + '\n')) });
            }}
          />
        )}
```

- [ ] **Step 3: 编译验证**

Run: `npx tsc --noEmit`
Expected: 无错误

- [ ] **Step 4: 手动测试**

Run: `./run.sh`
验证：
1. 命令输入栏下方出现快捷命令栏（28px 高度的条形区域）
2. 最左侧有 `+` 按钮
3. 如果有旧收藏命令，应自动迁移显示为按钮
4. 点击按钮直接发送命令到终端
5. 鼠标悬停显示完整命令 tooltip

- [ ] **Step 5: Commit**

```bash
git add src/components/QuickCommandBar.tsx src/App.tsx
git commit -m "feat: 快捷命令栏 — 按钮渲染 + 点击执行 + 旧数据迁移"
```

---

### Task 2: 新增/编辑浮层 + `+` 按钮功能

**Files:**
- Modify: `src/components/QuickCommandBar.tsx`

**Interfaces:**
- Consumes: `QuickCommand` 类型, `updateCommands` 函数（Task 1 产出）
- Produces: 新增/编辑浮层 UI，`+` 按钮触发新增，按钮右键触发编辑

- [ ] **Step 1: 添加新增/编辑浮层状态和 UI**

在 `QuickCommandBar` 组件中添加状态：
```tsx
  const [editForm, setEditForm] = useState<{ label: string; command: string; index: number | null } | null>(null);
```

在 return JSX 中，`+` 按钮的 onClick 改为：
```tsx
      <button
        className="flex-shrink-0 w-6 h-5 bg-surface border border-surface-border rounded text-xs text-gray-400 hover:bg-surface-lighter hover:text-white"
        title="添加快捷命令"
        onClick={() => setEditForm({ label: '', command: '', index: null })}
      >
        +
      </button>
```

在整个 div 末尾、关闭标签 `</div>` 之前，添加浮层：
```tsx
      {/* 新增/编辑浮层 */}
      {editForm && (
        <>
          <div className="fixed inset-0 z-30" onClick={() => setEditForm(null)} />
          <div className="absolute bottom-8 left-1 z-40 bg-surface-light border border-surface-border rounded shadow-lg p-2 w-72">
            <div className="flex flex-col gap-1.5">
              <input
                autoFocus
                className="bg-surface border border-surface-border rounded px-2 py-1 text-xs text-gray-200 outline-none focus:border-accent-cyan"
                placeholder="标签名称(必填,最多20字)"
                maxLength={20}
                value={editForm.label}
                onChange={(e) => setEditForm({ ...editForm, label: e.target.value })}
              />
              <input
                className="bg-surface border border-surface-border rounded px-2 py-1 text-xs text-gray-200 outline-none focus:border-accent-cyan font-mono"
                placeholder="命令内容(必填)"
                value={editForm.command}
                onChange={(e) => setEditForm({ ...editForm, command: e.target.value })}
                onKeyDown={(e) => {
                  if (e.key === 'Enter') handleSaveEdit();
                  if (e.key === 'Escape') setEditForm(null);
                }}
              />
              <div className="flex justify-end gap-1">
                <button
                  onClick={() => setEditForm(null)}
                  className="px-2 py-0.5 text-xs text-gray-400 hover:text-white"
                >取消</button>
                <button
                  onClick={handleSaveEdit}
                  className="px-2 py-0.5 text-xs bg-accent-cyan/20 text-accent-cyan rounded hover:bg-accent-cyan/30"
                  disabled={!editForm.label.trim() || !editForm.command.trim()}
                >确定</button>
              </div>
            </div>
          </div>
        </>
      )}
```

- [ ] **Step 2: 添加 handleSaveEdit 函数**

在 `updateCommands` 函数后添加：
```tsx
  const handleSaveEdit = () => {
    if (!editForm || !editForm.label.trim() || !editForm.command.trim()) return;
    const newCmd = { label: editForm.label.trim(), command: editForm.command.trim() };
    if (editForm.index === null) {
      updateCommands([...commands, newCmd]);
    } else {
      const updated = [...commands];
      updated[editForm.index] = newCmd;
      updateCommands(updated);
    }
    setEditForm(null);
  };
```

- [ ] **Step 3: 给外层 div 加 `relative` 定位（浮层锚点）**

```tsx
  return (
    <div className="h-7 bg-surface-light border-b border-surface-border flex items-center px-1 gap-1 overflow-x-auto relative">
```

- [ ] **Step 4: 编译验证**

Run: `npx tsc --noEmit`
Expected: 无错误

- [ ] **Step 5: 手动测试**

Run: `./run.sh`
验证：
1. 点击 `+` 弹出浮层，有标签和命令两个输入框
2. 输入内容后点"确定"，按钮出现在栏中
3. 点击空白处或"取消"关闭浮层
4. 新增的按钮点击能执行命令
5. 命令输入框中按 Enter 也能提交
6. 标签或命令为空时"确定"按钮 disabled

- [ ] **Step 6: Commit**

```bash
git add src/components/QuickCommandBar.tsx
git commit -m "feat: 快捷命令栏 — 新增/编辑浮层"
```

---

### Task 3: 右键上下文菜单 + 管理弹窗

**Files:**
- Modify: `src/components/QuickCommandBar.tsx`

**Interfaces:**
- Consumes: `QuickCommand[]`, `updateCommands`, `setEditForm`（Task 1-2 产出）
- Produces: 按钮右键菜单（编辑/删除）、空白右键菜单（新增/管理）、管理弹窗

- [ ] **Step 1: 添加右键菜单状态**

```tsx
  const [contextMenu, setContextMenu] = useState<{ x: number; y: number; index: number | null } | null>(null);
  const [showManage, setShowManage] = useState(false);
```

- [ ] **Step 2: 给按钮添加 onContextMenu**

每个命令按钮添加右键事件：
```tsx
      {commands.map((cmd, i) => (
        <button
          key={i}
          className="flex-shrink-0 bg-surface border border-surface-border rounded px-2 py-0.5 text-xs text-gray-300 hover:bg-surface-lighter hover:text-white cursor-pointer"
          title={cmd.command}
          onClick={() => sendCommand(cmd.command)}
          onContextMenu={(e) => {
            e.preventDefault();
            e.stopPropagation();
            setContextMenu({ x: e.clientX, y: e.clientY, index: i });
          }}
        >
          {cmd.label}
        </button>
      ))}
```

- [ ] **Step 3: 给整个栏添加空白处右键**

外层 div 添加：
```tsx
    <div
      className="h-7 bg-surface-light border-b border-surface-border flex items-center px-1 gap-1 overflow-x-auto relative"
      onContextMenu={(e) => {
        e.preventDefault();
        setContextMenu({ x: e.clientX, y: e.clientY, index: null });
      }}
    >
```

- [ ] **Step 4: 渲染右键菜单**

在浮层之前添加：
```tsx
      {/* 右键菜单 */}
      {contextMenu && (
        <>
          <div className="fixed inset-0 z-40" onClick={() => setContextMenu(null)} onContextMenu={(e) => { e.preventDefault(); setContextMenu(null); }} />
          <div
            className="fixed z-50 bg-surface-light border border-surface-border rounded shadow-lg py-1 min-w-[120px]"
            style={{ left: contextMenu.x, top: contextMenu.y }}
            onMouseDown={(e) => e.stopPropagation()}
          >
            {contextMenu.index !== null ? (
              <>
                <button
                  className="w-full text-left px-3 py-1 text-xs text-gray-300 hover:bg-surface-lighter hover:text-white"
                  onClick={() => {
                    const cmd = commands[contextMenu.index!];
                    setEditForm({ label: cmd.label, command: cmd.command, index: contextMenu.index });
                    setContextMenu(null);
                  }}
                >编辑</button>
                <button
                  className="w-full text-left px-3 py-1 text-xs text-accent-red hover:bg-surface-lighter"
                  onClick={() => {
                    updateCommands(commands.filter((_, j) => j !== contextMenu.index));
                    setContextMenu(null);
                  }}
                >删除</button>
              </>
            ) : (
              <>
                <button
                  className="w-full text-left px-3 py-1 text-xs text-gray-300 hover:bg-surface-lighter hover:text-white"
                  onClick={() => {
                    setEditForm({ label: '', command: '', index: null });
                    setContextMenu(null);
                  }}
                >新增</button>
                <button
                  className="w-full text-left px-3 py-1 text-xs text-gray-300 hover:bg-surface-lighter hover:text-white"
                  onClick={() => {
                    setShowManage(true);
                    setContextMenu(null);
                  }}
                >管理</button>
              </>
            )}
          </div>
        </>
      )}
```

- [ ] **Step 5: 渲染管理弹窗**

在右键菜单之后添加：
```tsx
      {/* 管理弹窗 */}
      {showManage && (
        <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/50" onClick={() => setShowManage(false)}>
          <div className="bg-surface-light border border-surface-border rounded-lg shadow-xl w-[480px] max-h-[60vh] flex flex-col" onClick={(e) => e.stopPropagation()}>
            <div className="flex items-center justify-between px-4 py-2 border-b border-surface-border">
              <span className="text-sm text-gray-200">管理快捷命令</span>
              <button onClick={() => setShowManage(false)} className="text-gray-400 hover:text-white text-lg">×</button>
            </div>
            <div className="flex-1 overflow-y-auto">
              {commands.length === 0 ? (
                <div className="px-4 py-8 text-xs text-gray-500 text-center">暂无快捷命令</div>
              ) : (
                commands.map((cmd, i) => (
                  <div key={i} className="flex items-center px-4 py-1.5 border-b border-surface-border/30 hover:bg-surface-lighter group">
                    <span className="w-24 text-xs text-gray-300 truncate flex-shrink-0">{cmd.label}</span>
                    <span className="flex-1 text-xs text-gray-500 font-mono truncate min-w-0 px-2">{cmd.command}</span>
                    <button
                      className="px-1.5 text-gray-500 hover:text-accent-cyan text-xs opacity-0 group-hover:opacity-100"
                      onClick={() => {
                        setEditForm({ label: cmd.label, command: cmd.command, index: i });
                        setShowManage(false);
                      }}
                    >编辑</button>
                    <button
                      className="px-1.5 text-gray-500 hover:text-accent-red text-xs opacity-0 group-hover:opacity-100"
                      onClick={() => updateCommands(commands.filter((_, j) => j !== i))}
                    >删除</button>
                  </div>
                ))
              )}
            </div>
            <div className="px-4 py-2 border-t border-surface-border">
              <button
                className="text-xs text-accent-cyan hover:text-accent-cyan/80"
                onClick={() => {
                  setEditForm({ label: '', command: '', index: null });
                  setShowManage(false);
                }}
              >+ 添加</button>
            </div>
          </div>
        </div>
      )}
```

- [ ] **Step 6: 编译验证**

Run: `npx tsc --noEmit`
Expected: 无错误

- [ ] **Step 7: 全量构建验证**

Run: `./build.sh`
Expected: 全部通过

- [ ] **Step 8: 手动测试**

Run: `./run.sh`
验证：
1. 按钮上右键 → 弹出"编辑/删除"菜单
2. 点击"编辑" → 弹出预填的编辑浮层，修改后保存
3. 点击"删除" → 按钮消失
4. 空白处右键 → 弹出"新增/管理"菜单
5. 点击"管理" → 弹出管理弹窗，显示所有命令
6. 管理弹窗中编辑/删除功能正常
7. 管理弹窗底部"添加"按钮能新增
8. 关闭后重新打开，数据持久化正常

- [ ] **Step 9: Commit**

```bash
git add src/components/QuickCommandBar.tsx
git commit -m "feat: 快捷命令栏 — 右键菜单 + 管理弹窗"
```
