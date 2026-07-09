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

  const [editForm, setEditForm] = useState<{ label: string; command: string; index: number | null } | null>(null);

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

  return (
    <div className="h-7 bg-surface-light border-b border-surface-border flex items-center px-1 gap-1 overflow-x-auto relative">
      <button
        className="flex-shrink-0 w-6 h-5 bg-surface border border-surface-border rounded text-xs text-gray-400 hover:bg-surface-lighter hover:text-white"
        title="添加快捷命令"
        onClick={() => setEditForm({ label: '', command: '', index: null })}
      >
        +
      </button>
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
    </div>
  );
}
