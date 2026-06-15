import { useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { Tab } from '../types';

interface Props {
  onClose: () => void;
  tabs: Tab[];
}

export function BatchCommand({ onClose, tabs }: Props) {
  const sshTabs = tabs.filter(t => t.type === 'ssh');
  const [selected, setSelected] = useState<Set<string>>(new Set(sshTabs.map(t => t.id)));
  const [command, setCommand] = useState('');
  const [executing, setExecuting] = useState(false);
  const [result, setResult] = useState<{ sent: string[]; failed: string[] } | null>(null);

  function toggleTab(id: string) {
    setSelected(prev => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
  }

  function toggleAll() {
    if (selected.size === sshTabs.length) {
      setSelected(new Set());
    } else {
      setSelected(new Set(sshTabs.map(t => t.id)));
    }
  }

  async function handleExecute() {
    if (!command.trim() || selected.size === 0) return;
    setExecuting(true);
    setResult(null);
    const sent: string[] = [];
    const failed: string[] = [];
    const data = Array.from(new TextEncoder().encode(command + '\n'));

    for (const tab of sshTabs) {
      if (!selected.has(tab.id)) continue;
      try {
        await invoke('terminal_write', { id: tab.id, data });
        sent.push(tab.label);
      } catch {
        failed.push(tab.label);
      }
    }

    setResult({ sent, failed });
    setExecuting(false);
  }

  return (
    <div className="fixed inset-0 bg-black/60 flex items-center justify-center z-50" onClick={onClose}>
      <div
        className="bg-surface-light border border-surface-border rounded-lg shadow-xl"
        style={{ width: '480px', maxHeight: '80vh', display: 'flex', flexDirection: 'column' }}
        onClick={(e) => e.stopPropagation()}
      >
        {/* Header */}
        <div className="flex items-center justify-between px-5 py-3 border-b border-surface-border">
          <h2 className="text-sm font-semibold text-gray-200">批量命令</h2>
          <button
            onClick={onClose}
            className="text-gray-500 hover:text-white text-lg leading-none"
          >{'×'}</button>
        </div>

        {/* Body */}
        <div className="flex-1 overflow-y-auto px-5 py-4" style={{ minHeight: 0 }}>
          {sshTabs.length === 0 ? (
            <div className="text-center text-gray-500 text-sm py-8">暂无打开的 SSH 连接</div>
          ) : (
            <>
              {/* Select all toggle */}
              <div className="flex items-center justify-between mb-2">
                <div className="text-xs text-gray-400 font-semibold">目标终端 ({selected.size}/{sshTabs.length})</div>
                <button
                  onClick={toggleAll}
                  className="text-[10px] px-2 py-0.5 border border-surface-border rounded text-gray-400 hover:text-accent-cyan hover:border-accent-cyan/50"
                >{selected.size === sshTabs.length ? '取消全选' : '全选'}</button>
              </div>

              {/* SSH tab checkboxes */}
              <div className="space-y-1 mb-4">
                {sshTabs.map((tab) => (
                  <label
                    key={tab.id}
                    className="flex items-center gap-2 px-3 py-2 bg-surface rounded border border-surface-border hover:border-gray-600 cursor-pointer"
                  >
                    <input
                      type="checkbox"
                      checked={selected.has(tab.id)}
                      onChange={() => toggleTab(tab.id)}
                      className="accent-cyan-400"
                    />
                    <div className="flex-1 min-w-0">
                      <div className="text-xs text-gray-200 truncate">{tab.label}</div>
                      {tab.sshParams && (
                        <div className="text-[10px] text-gray-500 truncate">
                          {tab.sshParams.user}@{tab.sshParams.host}:{tab.sshParams.port}
                        </div>
                      )}
                    </div>
                  </label>
                ))}
              </div>

              {/* Command input */}
              <div className="mb-4">
                <div className="text-xs text-gray-400 font-semibold mb-1">命令</div>
                <input
                  type="text"
                  value={command}
                  onChange={(e) => setCommand(e.target.value)}
                  onKeyDown={(e) => { if (e.key === 'Enter') handleExecute(); }}
                  placeholder="输入要执行的命令..."
                  className="w-full bg-surface border border-surface-border rounded px-3 py-2 text-xs text-gray-200 font-mono outline-none focus:border-accent-cyan"
                  autoFocus
                />
              </div>

              {/* Execute button */}
              <button
                onClick={handleExecute}
                disabled={executing || selected.size === 0 || !command.trim()}
                className="w-full px-3 py-2 text-xs bg-accent-cyan/20 text-accent-cyan rounded hover:bg-accent-cyan/30 disabled:opacity-50 disabled:cursor-not-allowed"
              >{executing ? '发送中...' : '执行'}</button>

              {/* Results */}
              {result && (
                <div className="mt-4 px-3 py-2 bg-surface rounded border border-surface-border">
                  <div className="text-xs text-gray-200 mb-1">
                    已发送到 {result.sent.length} 个终端
                  </div>
                  {result.sent.length > 0 && (
                    <div className="text-[10px] text-gray-500">
                      {result.sent.join(', ')}
                    </div>
                  )}
                  {result.failed.length > 0 && (
                    <div className="text-[10px] text-red-400 mt-1">
                      发送失败: {result.failed.join(', ')}
                    </div>
                  )}
                </div>
              )}
            </>
          )}
        </div>

        {/* Footer */}
        <div className="px-5 py-3 border-t border-surface-border flex justify-end">
          <button
            onClick={onClose}
            className="px-3 py-1 text-xs text-gray-400 hover:text-white"
          >关闭</button>
        </div>
      </div>
    </div>
  );
}
