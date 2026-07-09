import { useRef, useState } from 'react';
import { log } from '../utils/logger';

export interface QuickCommand {
  label: string;
  command: string;
  system?: boolean;
}

const STORAGE_KEY = 'guishell_quick_commands';
const VERSION_KEY = 'guishell_quick_commands_v';
const CURRENT_VERSION = '4';

const SYSTEM_DEFAULTS: QuickCommand[] = [
  { label: '查看磁盘', command: 'df -h', system: true },
  { label: '查看内存', command: 'free -h', system: true },
  { label: '查看进程', command: 'top -bn1 | head -20', system: true },
  { label: '网络连接', command: 'ss -tlnp', system: true },
];

function loadQuickCommands(): QuickCommand[] {
  try {
    const ver = localStorage.getItem(VERSION_KEY);
    const saved = localStorage.getItem(STORAGE_KEY);
    if (ver === CURRENT_VERSION && saved) return JSON.parse(saved);
    // 升级或首次: 保留用户自定义,替换系统默认
    const old: QuickCommand[] = saved ? JSON.parse(saved) : [];
    const userCmds = old.filter(c => !c.system);
    const merged = [...SYSTEM_DEFAULTS, ...userCmds];
    localStorage.setItem(STORAGE_KEY, JSON.stringify(merged));
    localStorage.setItem(VERSION_KEY, CURRENT_VERSION);
    return merged;
  } catch {}
  return SYSTEM_DEFAULTS;
}

function saveQuickCommands(cmds: QuickCommand[]) {
  localStorage.setItem(STORAGE_KEY, JSON.stringify(cmds));
}

interface Props {
  sendCommand: (cmd: string) => void;
}

export function QuickCommandBar({ sendCommand }: Props) {
  const [commands, setCommands] = useState<QuickCommand[]>(loadQuickCommands);
  const barRef = useRef<HTMLDivElement>(null);

  const updateCommands = (updated: QuickCommand[]) => {
    setCommands(updated);
    saveQuickCommands(updated);
  };

  const [editForm, setEditForm] = useState<{ label: string; command: string; index: number | null } | null>(null);
  const [editError, setEditError] = useState('');

  const openEditForm = (form: { label: string; command: string; index: number | null }) => {
    setEditForm(form);
    setEditError('');
  };

  const handleSaveEdit = () => {
    if (!editForm) return;
    if (!editForm.label.trim()) { setEditError('请输入标签名称'); return; }
    if (!editForm.command.trim()) { setEditError('请输入命令内容'); return; }
    const newCmd: QuickCommand = { label: editForm.label.trim(), command: editForm.command.trim() };
    if (editForm.index === null) {
      updateCommands([...commands, newCmd]);
    } else {
      const updated = [...commands];
      updated[editForm.index] = newCmd;
      updateCommands(updated);
    }
    setEditForm(null);
    setEditError('');
  };

  const [contextMenu, setContextMenu] = useState<{ x: number; y: number; index: number | null } | null>(null);
  const [showManage, setShowManage] = useState(false);
  const [confirmDelete, setConfirmDelete] = useState<number | null>(null);

  // 计算浮层位置(在栏上方弹出)
  const getEditFormStyle = (): React.CSSProperties => {
    const rect = barRef.current?.getBoundingClientRect();
    if (!rect) return { bottom: '40px', left: '8px' };
    return { top: rect.top - 4 + 'px', left: rect.left + 'px', transform: 'translateY(-100%)' };
  };

  return (
    <div
      ref={barRef}
      className="h-7 bg-surface-light border-b border-surface-border flex items-center px-1 gap-1 overflow-x-auto"
      onContextMenu={(e) => {
        e.preventDefault();
        setContextMenu({ x: e.clientX, y: e.clientY, index: null });
      }}
    >
      <button
        className="flex-shrink-0 w-6 h-5 bg-surface border border-surface-border rounded text-xs text-gray-400 hover:bg-surface-lighter hover:text-white"
        title="添加快捷命令"
        onClick={() => openEditForm({ label: '', command: '', index: null })}
      >
        +
      </button>
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
      {/* 右键菜单(fixed,不受 overflow 影响) */}
      {contextMenu && (
        <>
          <div className="fixed inset-0 z-40" onClick={() => setContextMenu(null)} onContextMenu={(e) => { e.preventDefault(); setContextMenu(null); }} />
          <div
            className="fixed z-50 bg-surface-light border border-surface-border rounded shadow-lg py-1 min-w-[120px]"
            style={{
              left: contextMenu.x,
              ...(contextMenu.y > window.innerHeight - 80
                ? { bottom: window.innerHeight - contextMenu.y }
                : { top: contextMenu.y }),
            }}
            onMouseDown={(e) => e.stopPropagation()}
          >
            {contextMenu.index !== null ? (
              <>
                <button
                  className="w-full text-left px-3 py-1 text-xs text-gray-300 hover:bg-surface-lighter hover:text-white"
                  onClick={() => {
                    const cmd = commands[contextMenu.index!];
                    openEditForm({ label: cmd.label, command: cmd.command, index: contextMenu.index });
                    setContextMenu(null);
                  }}
                >编辑</button>
                <button
                  className="w-full text-left px-3 py-1 text-xs text-accent-red hover:bg-surface-lighter"
                  onClick={() => {
                    const cmd = commands[contextMenu.index!];
                    setConfirmDelete(contextMenu.index);
                    setContextMenu(null);
                  }}
                >删除</button>
              </>
            ) : (
              <>
                <button
                  className="w-full text-left px-3 py-1 text-xs text-gray-300 hover:bg-surface-lighter hover:text-white"
                  onClick={() => {
                    openEditForm({ label: '', command: '', index: null });
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
      {/* 管理弹窗(fixed 全屏遮罩) */}
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
                    <span className="w-24 text-xs text-gray-300 truncate flex-shrink-0">{cmd.label}{cmd.system && <span className="text-[9px] text-gray-600 ml-1">内置</span>}</span>
                    <span className="flex-1 text-xs text-gray-500 font-mono truncate min-w-0 px-2">{cmd.command}</span>
                    <button
                      className="px-1.5 text-gray-500 hover:text-accent-cyan text-xs opacity-0 group-hover:opacity-100"
                      onClick={() => {
                        openEditForm({ label: cmd.label, command: cmd.command, index: i });
                        setShowManage(false);
                      }}
                    >编辑</button>
                    <button
                      className="px-1.5 text-gray-500 hover:text-accent-red text-xs opacity-0 group-hover:opacity-100"
                      onClick={() => { setConfirmDelete(i); setShowManage(false); }}
                    >删除</button>
                  </div>
                ))
              )}
            </div>
            <div className="px-4 py-2 border-t border-surface-border">
              <button
                className="text-xs text-accent-cyan hover:text-accent-cyan/80"
                onClick={() => {
                  openEditForm({ label: '', command: '', index: null });
                  setShowManage(false);
                }}
              >+ 添加</button>
            </div>
          </div>
        </div>
      )}
      {/* 删除确认弹窗(居中遮罩) */}
      {confirmDelete !== null && commands[confirmDelete] && (
        <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/50" onClick={() => setConfirmDelete(null)}>
          <div className="bg-surface-light border border-surface-border rounded-lg shadow-xl p-4 w-80" onClick={(e) => e.stopPropagation()}>
            <div className="text-sm text-gray-200 mb-4 text-center">
              确认删除快捷命令「<span className="text-accent-red font-bold">{commands[confirmDelete].label}</span>」？
            </div>
            <div className="flex justify-center gap-3">
              <button
                onClick={() => setConfirmDelete(null)}
                className="px-4 py-1 text-xs text-gray-400 hover:text-white border border-surface-border rounded"
              >取消</button>
              <button
                onClick={() => {
                  updateCommands(commands.filter((_, j) => j !== confirmDelete));
                  setConfirmDelete(null);
                }}
                className="px-4 py-1 text-xs bg-accent-red/20 text-accent-red rounded hover:bg-accent-red/30"
              >删除</button>
            </div>
          </div>
        </div>
      )}
      {/* 新增/编辑浮层(fixed 定位,避免被 overflow-x: auto 裁剪) */}
      {editForm && (
        <>
          <div className="fixed inset-0 z-30" onClick={() => setEditForm(null)} />
          <div
            className="fixed z-40 bg-surface-light border border-surface-border rounded shadow-lg p-2 w-72"
            style={getEditFormStyle()}
          >
            <div className="flex flex-col gap-1.5">
              <input
                autoFocus
                className="bg-surface border border-surface-border rounded px-2 py-1 text-xs text-gray-200 outline-none focus:border-accent-cyan"
                placeholder="标签名称(必填,最多20字)"
                maxLength={20}
                value={editForm.label}
                onChange={(e) => { setEditForm({ ...editForm, label: e.target.value }); setEditError(''); }}
                onKeyDown={(e) => {
                  if (e.key === 'Enter') handleSaveEdit();
                  if (e.key === 'Escape') { setEditForm(null); setEditError(''); }
                }}
              />
              <input
                className="bg-surface border border-surface-border rounded px-2 py-1 text-xs text-gray-200 outline-none focus:border-accent-cyan font-mono"
                placeholder="命令内容(必填)"
                value={editForm.command}
                onChange={(e) => { setEditForm({ ...editForm, command: e.target.value }); setEditError(''); }}
                onKeyDown={(e) => {
                  if (e.key === 'Enter') handleSaveEdit();
                  if (e.key === 'Escape') { setEditForm(null); setEditError(''); }
                }}
              />
              {editError && <span className="text-[10px] text-accent-red">{editError}</span>}
              <div className="flex justify-end gap-1">
                <button
                  onClick={() => setEditForm(null)}
                  className="px-2 py-0.5 text-xs text-gray-400 hover:text-white"
                >取消</button>
                <button
                  onClick={handleSaveEdit}
                  className="px-2 py-0.5 text-xs bg-accent-cyan/20 text-accent-cyan rounded hover:bg-accent-cyan/30"
                >确定</button>
              </div>
            </div>
          </div>
        </>
      )}
    </div>
  );
}
