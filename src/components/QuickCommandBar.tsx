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

  return (
    <div className="h-7 bg-surface-light border-b border-surface-border flex items-center px-1 gap-1 overflow-x-auto">
      <button
        className="flex-shrink-0 w-6 h-5 bg-surface border border-surface-border rounded text-xs text-gray-400 hover:bg-surface-lighter hover:text-white"
        title="添加快捷命令"
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
    </div>
  );
}
