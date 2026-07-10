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
