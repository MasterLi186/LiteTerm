import { useState, useEffect } from 'react';

export interface ShortcutConfig {
  newTab: string;
  closeTab: string;
  search: string;
  splitH: string;
  splitV: string;
  nextTab: string;
  prevTab: string;
  copyText: string;
  pasteText: string;
}

export const DEFAULT_SHORTCUTS: ShortcutConfig = {
  newTab: 'Ctrl+Shift+T',
  closeTab: 'Ctrl+Shift+W',
  search: 'Ctrl+Shift+F',
  splitH: 'Ctrl+Shift+D',
  splitV: 'Ctrl+Shift+R',
  nextTab: 'Ctrl+Tab',
  prevTab: 'Ctrl+Shift+Tab',
  copyText: 'Ctrl+Shift+C',
  pasteText: 'Ctrl+Shift+V',
};

export function loadShortcuts(): ShortcutConfig {
  try {
    const saved = localStorage.getItem('guishell_shortcuts');
    if (saved) return { ...DEFAULT_SHORTCUTS, ...JSON.parse(saved) };
  } catch {}
  return { ...DEFAULT_SHORTCUTS };
}

export function saveShortcuts(shortcuts: ShortcutConfig): void {
  localStorage.setItem('guishell_shortcuts', JSON.stringify(shortcuts));
}

export function matchShortcut(e: KeyboardEvent, shortcut: string): boolean {
  const parts = shortcut.split('+');
  const key = parts[parts.length - 1];
  const modifiers = parts.slice(0, -1).map((m) => m.toLowerCase());

  const needCtrl = modifiers.includes('ctrl');
  const needShift = modifiers.includes('shift');
  const needAlt = modifiers.includes('alt');

  if (e.ctrlKey !== needCtrl) return false;
  if (e.shiftKey !== needShift) return false;
  if (e.altKey !== needAlt) return false;

  if (key.toLowerCase() === 'tab') return e.key === 'Tab';

  return e.key.toLowerCase() === key.toLowerCase();
}

const ACTION_LABELS: Record<keyof ShortcutConfig, string> = {
  newTab: '新建标签页',
  closeTab: '关闭标签页',
  search: '搜索',
  splitH: '水平分屏',
  splitV: '垂直分屏',
  nextTab: '下一个标签页',
  prevTab: '上一个标签页',
  copyText: '复制',
  pasteText: '粘贴',
};

const ACTION_KEYS: (keyof ShortcutConfig)[] = [
  'newTab',
  'closeTab',
  'search',
  'splitH',
  'splitV',
  'nextTab',
  'prevTab',
  'copyText',
  'pasteText',
];

interface Props {
  onClose: () => void;
}

function formatKeyFromEvent(e: KeyboardEvent): string | null {
  // Ignore bare modifier keys
  if (['Control', 'Shift', 'Alt', 'Meta'].includes(e.key)) return null;

  const parts: string[] = [];
  if (e.ctrlKey) parts.push('Ctrl');
  if (e.shiftKey) parts.push('Shift');
  if (e.altKey) parts.push('Alt');

  let key = e.key;
  // Normalize special keys
  if (key === ' ') key = 'Space';
  else if (key === 'Escape') key = 'Escape';
  else if (key === 'Tab') key = 'Tab';
  else if (key === 'Enter') key = 'Enter';
  else if (key === 'Backspace') key = 'Backspace';
  else if (key === 'Delete') key = 'Delete';
  else if (key === 'ArrowUp') key = 'Up';
  else if (key === 'ArrowDown') key = 'Down';
  else if (key === 'ArrowLeft') key = 'Left';
  else if (key === 'ArrowRight') key = 'Right';
  else if (key.startsWith('F') && key.length >= 2 && !isNaN(Number(key.slice(1)))) {
    // F1-F12, keep as-is
  } else {
    // Regular character — capitalize first letter
    key = key.length === 1 ? key.toUpperCase() : key.charAt(0).toUpperCase() + key.slice(1);
  }

  parts.push(key);
  return parts.join('+');
}

export function ShortcutSettings({ onClose }: Props) {
  const [shortcuts, setShortcuts] = useState<ShortcutConfig>(loadShortcuts);
  const [editingKey, setEditingKey] = useState<keyof ShortcutConfig | null>(null);
  const [recordedCombo, setRecordedCombo] = useState<string | null>(null);

  useEffect(() => {
    if (!editingKey) return;

    function handleKeyDown(e: KeyboardEvent) {
      e.preventDefault();
      e.stopPropagation();
      const combo = formatKeyFromEvent(e);
      if (combo) {
        setRecordedCombo(combo);
      }
    }

    document.addEventListener('keydown', handleKeyDown, true);
    return () => document.removeEventListener('keydown', handleKeyDown, true);
  }, [editingKey]);

  function handleEdit(key: keyof ShortcutConfig) {
    setEditingKey(key);
    setRecordedCombo(null);
  }

  function handleConfirm() {
    if (editingKey && recordedCombo) {
      const updated = { ...shortcuts, [editingKey]: recordedCombo };
      setShortcuts(updated);
      saveShortcuts(updated);
    }
    setEditingKey(null);
    setRecordedCombo(null);
  }

  function handleCancel() {
    setEditingKey(null);
    setRecordedCombo(null);
  }

  function handleResetAll() {
    setShortcuts({ ...DEFAULT_SHORTCUTS });
    saveShortcuts({ ...DEFAULT_SHORTCUTS });
    setEditingKey(null);
    setRecordedCombo(null);
  }

  return (
    <div className="fixed inset-0 bg-black/60 flex items-center justify-center z-50" onClick={onClose}>
      <div
        className="bg-surface-light border border-surface-border rounded-lg shadow-xl"
        style={{ width: '500px', maxHeight: '80vh', display: 'flex', flexDirection: 'column' }}
        onClick={(e) => e.stopPropagation()}
      >
        {/* Header */}
        <div className="flex items-center justify-between px-5 py-3 border-b border-surface-border">
          <h2 className="text-sm font-semibold text-gray-200">快捷键设置</h2>
          <button
            onClick={onClose}
            className="text-gray-500 hover:text-white text-lg leading-none"
          >{'×'}</button>
        </div>

        {/* Body */}
        <div className="flex-1 overflow-y-auto px-5 py-4" style={{ minHeight: 0 }}>
          <div className="space-y-1">
            {ACTION_KEYS.map((key) => (
              <div
                key={key}
                className="flex items-center justify-between px-3 py-2 bg-surface rounded border border-surface-border hover:border-gray-600"
              >
                <div className="text-xs text-gray-200 w-28 flex-shrink-0">
                  {ACTION_LABELS[key]}
                </div>
                <div className="flex-1 flex items-center justify-center">
                  {editingKey === key ? (
                    recordedCombo ? (
                      <span className="px-2 py-0.5 bg-accent-cyan/20 text-accent-cyan rounded text-xs font-mono">
                        {recordedCombo}
                      </span>
                    ) : (
                      <span className="text-xs text-gray-400 animate-pulse">
                        请按下新的快捷键...
                      </span>
                    )
                  ) : (
                    <span className="px-2 py-0.5 bg-surface border border-surface-border rounded text-xs text-gray-300 font-mono">
                      {shortcuts[key]}
                    </span>
                  )}
                </div>
                <div className="flex items-center gap-1 ml-2 flex-shrink-0">
                  {editingKey === key ? (
                    <>
                      <button
                        onClick={handleConfirm}
                        disabled={!recordedCombo}
                        className="text-[10px] px-2 py-0.5 bg-accent-cyan/20 text-accent-cyan rounded hover:bg-accent-cyan/30 disabled:opacity-50"
                      >确认</button>
                      <button
                        onClick={handleCancel}
                        className="text-[10px] px-2 py-0.5 border border-surface-border rounded text-gray-400 hover:text-white hover:border-gray-500"
                      >取消</button>
                    </>
                  ) : (
                    <button
                      onClick={() => handleEdit(key)}
                      className="text-[10px] px-2 py-0.5 border border-surface-border rounded text-gray-400 hover:text-accent-cyan hover:border-accent-cyan/50"
                    >修改</button>
                  )}
                </div>
              </div>
            ))}
          </div>
        </div>

        {/* Footer */}
        <div className="px-5 py-3 border-t border-surface-border flex items-center justify-between">
          <button
            onClick={handleResetAll}
            className="px-3 py-1 text-xs text-gray-400 hover:text-white border border-surface-border rounded hover:border-gray-500"
          >恢复默认</button>
          <button
            onClick={onClose}
            className="px-3 py-1 text-xs text-gray-400 hover:text-white"
          >关闭</button>
        </div>
      </div>
    </div>
  );
}
