# 快捷命令栏设计 (XShell 风格)

## 目标

在命令输入栏下方新增一行常驻的快捷命令按钮栏，用户可自定义按钮标签和命令，点击即执行。对标 XShell 的快捷命令栏 UI。

## 布局

```
┌──────────────────────────────────────────────────────┐
│ 命令输入: [________________] [↑历史] [★收藏]         │  现有命令输入栏(保留不变)
├──────────────────────────────────────────────────────┤
│ [+] [重启服务] [查日志] [磁盘] [top]    (右键菜单)   │  新增：快捷命令栏
├──────────────────────────────────────────────────────┤
│                   终端区域                            │
└──────────────────────────────────────────────────────┘
```

快捷命令栏固定高度 28px，按钮超出容器宽度时横向滚动。仅在有活跃终端标签时显示（和命令输入栏一致）。

## 数据结构

```typescript
interface QuickCommand {
  label: string;    // 按钮显示文字，如"重启服务"
  command: string;  // 实际命令，如"systemctl restart app"
}
```

存储位置：`localStorage` key `guishell_quick_commands`，JSON 序列化的 `QuickCommand[]`。

启动时自动迁移：如果 `guishell_quick_commands` 不存在但 `guishell_cmd_favorites` 存在，将旧数据 `string[]` 转换为 `QuickCommand[]`（label 取命令前 6 字符）。迁移后不删除旧数据（收藏功能仍使用）。

## 交互

| 操作 | 行为 |
|------|------|
| 点击按钮 | 立即发送 command 到当前活跃终端 |
| 鼠标悬停按钮 | tooltip 显示完整命令文本 |
| 点击最左侧 `+` 按钮 | 弹出内联表单：输入标签 + 命令，确认后追加到列表末尾 |
| 空白区域右键 | 上下文菜单：新增、管理（打开管理弹窗） |
| 按钮上右键 | 上下文菜单：编辑、删除 |

### 新增/编辑表单

点击 `+` 或右键"新增"/"编辑"时，在按钮栏上方弹出小浮层：
- 两个输入框：标签（必填，限 20 字符）、命令（必填）
- 确认/取消按钮
- 编辑时预填现有值

### 管理弹窗

右键"管理"打开模态弹窗：
- 列表显示所有快捷命令（标签 + 命令）
- 每行：编辑按钮、删除按钮
- 支持拖拽排序（可选，第一版不实现）
- 底部"添加"按钮

## 组件结构

新建 `src/components/QuickCommandBar.tsx`：

```
QuickCommandBar
├── props: { sendCommand: (cmd: string) => void }
├── state: quickCommands: QuickCommand[]
├── 渲染:
│   ├── [+] 按钮 (最左侧)
│   ├── QuickCommand 按钮列表 (横向排列，overflow-x: auto)
│   ├── 新增/编辑浮层 (条件渲染)
│   ├── 右键上下文菜单 (条件渲染)
│   └── 管理弹窗 (条件渲染)
└── localStorage 读写
```

修改 `App.tsx`：
- 在 CommandBar 组件下方插入 `<QuickCommandBar sendCommand={sendCommand} />`
- 不修改现有 CommandBar、收藏功能、历史功能

## 样式

- 按钮：`bg-surface border border-surface-border rounded px-2 py-0.5 text-xs text-gray-300 hover:bg-surface-lighter hover:text-white cursor-pointer`
- `+` 按钮：同样式但固定宽度，显示 `+` 字符
- 整栏背景：`bg-surface-light border-b border-surface-border`
- 右键菜单：复用项目中已有的右键菜单样式（参考终端右键菜单）

## 不做的事

- 拖拽排序（第一版不做，管理弹窗里可以后续加）
- 按钮分组/分类
- 命令参数模板（如 `${input}` 替换）
- 导入/导出快捷命令
