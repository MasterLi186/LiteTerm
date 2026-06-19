import { useEffect, useState, useRef, useCallback } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { save, open } from '@tauri-apps/plugin-dialog';
import { getCurrentWebview } from '@tauri-apps/api/webview';
import { TerminalPane } from './components/Terminal/TerminalPane';
import { SplitContainer } from './components/Terminal/SplitContainer';
import { ConnectionDialog } from './components/ConnectionDialog';
import { SystemInfoPanel } from './components/Sidebar/SystemInfoPanel';
import { FileBrowser } from './components/FileManager/FileBrowser';
import { ProcessTable } from './components/ProcessManager/ProcessTable';
import { NewTabSelector } from './components/NewTabSelector';
import { SshKeyManager } from './components/SshKeyManager';
import { BatchCommand } from './components/BatchCommand';
import { ShortcutSettings, loadShortcuts, matchShortcut } from './components/ShortcutSettings';
import { TunnelManager } from './components/TunnelManager';
import { RecordingPlayer } from './components/RecordingPlayer';
import type { Tab, ConnectionStore, AuthMethod, SplitNode } from './types';
import { log, getLogText } from './utils/logger';
import { IconImport, IconExport, IconKey, IconPlus, IconClose, IconStar, IconStarFilled, IconTrash, IconHistory, IconBatchCmd, IconTunnel, IconSettings, IconLog, IconChevronDown, IconChevronRight, IconReconnect, IconCopy, IconPlay } from './components/Icons';

function getTerminalSize() {
  return {
    cols: Math.floor((window.innerWidth - 280) / 8),
    rows: Math.floor((window.innerHeight - 200) / 17),
  };
}

async function sshConnect(params: Record<string, unknown>): Promise<string> {
  return invoke<string>('ssh_connect', { ...params, ...getTerminalSize() });
}

function PasswordPrompt({ hostLabel, onSubmit, onCancel }: {
  hostLabel: string;
  onSubmit: (password: string) => void;
  onCancel: () => void;
}) {
  const [password, setPassword] = useState('');
  return (
    <div className="fixed inset-0 bg-black/60 flex items-center justify-center z-50">
      <div className="bg-surface-light border border-surface-border rounded-lg p-6 w-80">
        <h3 className="text-sm font-semibold mb-3">连接到 {hostLabel}</h3>
        <input
          type="password"
          value={password}
          onChange={(e) => setPassword(e.target.value)}
          onKeyDown={(e) => e.key === 'Enter' && onSubmit(password)}
          placeholder="请输入密码"
          autoFocus
          className="w-full bg-surface border border-surface-border rounded px-3 py-2 text-sm mb-4 outline-none focus:border-accent-cyan"
        />
        <div className="flex justify-end gap-2">
          <button onClick={onCancel} className="px-3 py-1 text-sm text-gray-400 hover:text-white">取消</button>
          <button onClick={() => onSubmit(password)} className="px-3 py-1 text-sm bg-accent-cyan/20 text-accent-cyan rounded hover:bg-accent-cyan/30">连接</button>
        </div>
      </div>
    </div>
  );
}

function CommandInputBar({ terminalId }: { terminalId: string | null }) {
  const [history, setHistory] = useState<string[]>(() => {
    try { return JSON.parse(localStorage.getItem('guishell_cmd_history') || '[]'); } catch { return []; }
  });
  const [favorites, setFavorites] = useState<string[]>(() => {
    try { return JSON.parse(localStorage.getItem('guishell_cmd_favorites') || '[]'); } catch { return []; }
  });
  const [showHistory, setShowHistory] = useState(false);
  const [showFavorites, setShowFavorites] = useState(false);
  const inputRef = useRef<HTMLInputElement>(null);

  function sendCommand(cmd: string) {
    if (!cmd || !terminalId) return;
    invoke('terminal_write', { id: terminalId, data: Array.from(new TextEncoder().encode(cmd + '\n')) });
    const updated = [cmd, ...history.filter(h => h !== cmd)].slice(0, 100);
    setHistory(updated);
    localStorage.setItem('guishell_cmd_history', JSON.stringify(updated));
  }

  function toggleFavorite(cmd: string) {
    let updated: string[];
    if (favorites.includes(cmd)) {
      updated = favorites.filter(f => f !== cmd);
    } else {
      updated = [...favorites, cmd];
    }
    setFavorites(updated);
    localStorage.setItem('guishell_cmd_favorites', JSON.stringify(updated));
  }

  function removeHistory(cmd: string) {
    const updated = history.filter(h => h !== cmd);
    setHistory(updated);
    localStorage.setItem('guishell_cmd_history', JSON.stringify(updated));
  }

  function removeFavorite(cmd: string) {
    const updated = favorites.filter(f => f !== cmd);
    setFavorites(updated);
    localStorage.setItem('guishell_cmd_favorites', JSON.stringify(updated));
  }

  return (
    <div className="h-8 bg-surface-light border-t border-b border-surface-border flex items-center px-2 gap-2 relative">
      <span className="text-xs text-gray-500 flex-shrink-0">命令输入:</span>
      <input
        ref={inputRef}
        className="flex-1 bg-surface border border-surface-border rounded px-2 py-0.5 text-xs outline-none focus:border-accent-cyan text-gray-200 min-w-0"
        placeholder="输入命令后回车发送到终端"
        onKeyDown={(e) => {
          if (e.key === 'Enter' && e.currentTarget.value) {
            sendCommand(e.currentTarget.value);
            e.currentTarget.value = '';
            setShowHistory(false);
          }
          if (e.key === 'ArrowUp') {
            e.preventDefault();
            setShowHistory(true);
            setShowFavorites(false);
          }
          if (e.key === 'Escape') {
            setShowHistory(false);
            setShowFavorites(false);
          }
        }}
      />
      <button
        onClick={() => { setShowHistory(!showHistory); setShowFavorites(false); }}
        className={`px-1.5 py-0.5 border border-surface-border rounded flex-shrink-0 ${showHistory ? 'text-accent-cyan bg-accent-cyan/10' : 'text-gray-400 hover:text-white'}`}
        title="命令历史"
      >
        <IconHistory size={14} />
      </button>
      <button
        onClick={() => { setShowFavorites(!showFavorites); setShowHistory(false); }}
        className={`px-1.5 py-0.5 border border-surface-border rounded flex-shrink-0 ${showFavorites ? 'text-accent-yellow bg-accent-yellow/10' : 'text-gray-400 hover:text-white'}`}
        title="收藏命令"
      >
        <IconStar size={14} />
      </button>

      {/* History popup */}
      {showHistory && (
        <>
          <div className="fixed inset-0 z-30" onClick={() => setShowHistory(false)} />
          <div className="absolute bottom-9 right-16 z-40 bg-surface-light border border-surface-border rounded shadow-lg w-96 max-h-72 overflow-y-auto">
            <div className="flex items-center justify-between px-3 py-1.5 border-b border-surface-border">
              <span className="text-xs text-gray-400">命令历史 ({history.length})</span>
              {history.length > 0 && (
                <button
                  onClick={() => {
                    if (window.confirm('确认清空所有命令历史？')) {
                      setHistory([]);
                      localStorage.removeItem('guishell_cmd_history');
                    }
                  }}
                  className="text-[10px] text-gray-500 hover:text-accent-red"
                >清空</button>
              )}
            </div>
            {history.length === 0 ? (
              <div className="px-3 py-4 text-xs text-gray-500 text-center">暂无历史记录</div>
            ) : (
              history.map((cmd, i) => (
                <div
                  key={i}
                  className="flex items-center hover:bg-surface-lighter border-b border-surface-border/30 last:border-b-0 group"
                >
                  <span
                    className="flex-1 px-3 py-1.5 text-xs text-gray-300 truncate font-mono min-w-0 cursor-pointer"
                    onClick={() => { if (inputRef.current) inputRef.current.value = cmd; }}
                    title="点击填入输入框"
                  >
                    {cmd}
                  </span>
                  <button
                    onClick={() => { sendCommand(cmd); if (inputRef.current) inputRef.current.value = ''; setShowHistory(false); }}
                    className="px-1 text-accent-green hover:brightness-125 flex-shrink-0"
                    title="执行"
                  >
                    <IconPlay size={13} />
                  </button>
                  <button
                    onClick={() => navigator.clipboard.writeText(cmd)}
                    className="px-1 text-accent-cyan hover:brightness-125 flex-shrink-0"
                    title="复制"
                  >
                    <IconCopy size={13} />
                  </button>
                  <button
                    onClick={() => toggleFavorite(cmd)}
                    className={`px-1 flex-shrink-0 ${favorites.includes(cmd) ? 'text-accent-yellow' : 'text-gray-500 hover:text-accent-yellow'}`}
                    title={favorites.includes(cmd) ? '取消收藏' : '收藏'}
                  >
                    {favorites.includes(cmd) ? <IconStarFilled size={13} /> : <IconStar size={13} />}
                  </button>
                  <button
                    onClick={() => removeHistory(cmd)}
                    className="px-1 text-accent-red hover:brightness-125 flex-shrink-0 mr-1"
                    title="删除"
                  >
                    <IconClose size={13} />
                  </button>
                </div>
              ))
            )}
          </div>
        </>
      )}

      {/* Favorites popup */}
      {showFavorites && (
        <>
          <div className="fixed inset-0 z-30" onClick={() => setShowFavorites(false)} />
          <div className="absolute bottom-9 right-2 z-40 bg-surface-light border border-surface-border rounded shadow-lg w-96 max-h-72 overflow-y-auto">
            <div className="flex items-center justify-between px-3 py-1.5 border-b border-surface-border">
              <span className="text-xs text-gray-400">收藏命令 ({favorites.length})</span>
            </div>
            {favorites.length === 0 ? (
              <div className="px-3 py-4 text-xs text-gray-500 text-center">暂无收藏，在历史记录中点击星标收藏</div>
            ) : (
              favorites.map((cmd, i) => (
                <div
                  key={i}
                  className="flex items-center hover:bg-surface-lighter border-b border-surface-border/30 last:border-b-0 group"
                >
                  <span
                    className="flex-1 px-3 py-1.5 text-xs text-gray-300 truncate font-mono min-w-0 cursor-pointer"
                    onClick={() => { if (inputRef.current) inputRef.current.value = cmd; }}
                    title="点击填入输入框"
                  >
                    {cmd}
                  </span>
                  <button
                    onClick={() => { sendCommand(cmd); if (inputRef.current) inputRef.current.value = ''; setShowFavorites(false); }}
                    className="px-1 text-accent-green hover:brightness-125 flex-shrink-0"
                    title="执行"
                  >
                    <IconPlay size={13} />
                  </button>
                  <button
                    onClick={() => navigator.clipboard.writeText(cmd)}
                    className="px-1 text-accent-cyan hover:brightness-125 flex-shrink-0"
                    title="复制"
                  >
                    <IconCopy size={13} />
                  </button>
                  <button
                    onClick={() => removeFavorite(cmd)}
                    className="px-1 text-accent-red hover:brightness-125 flex-shrink-0 mr-1"
                    title="删除"
                  >
                    <IconClose size={13} />
                  </button>
                </div>
              ))
            )}
          </div>
        </>
      )}
    </div>
  );
}

function replaceNode(node: SplitNode, targetId: string, replacement: SplitNode): SplitNode {
  if (node.type === 'terminal') {
    return node.terminalId === targetId ? replacement : node;
  }
  return {
    ...node,
    first: replaceNode(node.first, targetId, replacement),
    second: replaceNode(node.second, targetId, replacement),
  };
}

function removeNode(node: SplitNode, targetId: string): SplitNode | null {
  if (node.type === 'terminal') {
    return node.terminalId === targetId ? null : node;
  }
  const first = removeNode(node.first, targetId);
  const second = removeNode(node.second, targetId);
  if (!first && !second) return null;
  if (!first) return second;
  if (!second) return first;
  return { ...node, first, second };
}

/** Collect all terminal IDs from a split tree. */
function collectTerminalIds(node: SplitNode): string[] {
  if (node.type === 'terminal') return [node.terminalId];
  return [...collectTerminalIds(node.first), ...collectTerminalIds(node.second)];
}

function App() {
  const [tabs, setTabs] = useState<Tab[]>([]);
  const [activeTabId, setActiveTabId] = useState<string | null>(null);
  const [connections, setConnections] = useState<ConnectionStore>({ groups: {} });
  const [showDialog, setShowDialog] = useState(false);
  const [expandedGroups, setExpandedGroups] = useState<Record<string, boolean>>({});
  const [splitTrees, setSplitTrees] = useState<Record<string, SplitNode>>({});
  const [focusedTerminalId, setFocusedTerminalId] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [showNewTab, setShowNewTab] = useState(false);
  const [passwordPrompt, setPasswordPrompt] = useState<{
    groupId: string;
    hostId: string;
    hostLabel: string;
  } | null>(null);
  const [reconnecting, setReconnecting] = useState<Record<string, { attempts: number; status: string }>>({});

  // Derive active SSH session for monitor/file browser
  const activeTab = tabs.find((t) => t.id === activeTabId);
  const activeSshSessionId = activeTab?.type === 'ssh' ? activeTab.id : null;

  // Load connections on mount
  useEffect(() => {
    loadConnections();
  }, []);

  // Restore previous sessions or open a default local terminal
  useEffect(() => {
    restoreSessions();
    // Start local system monitor
    invoke('start_local_monitor').catch(() => {});
  }, []);

  // Save open sessions whenever tabs change
  useEffect(() => {
    const sessions = tabs
      .filter(t => t.type === 'ssh' && t.sshParams)
      .map(t => ({ label: t.label, sshParams: t.sshParams! }));
    localStorage.setItem('guishell_sessions', JSON.stringify(sessions));
  }, [tabs]);

  async function restoreSessions() {
    try {
      const saved = localStorage.getItem('guishell_sessions');
      if (saved) {
        const sessions: Array<{ label: string; sshParams: Tab['sshParams'] }> = JSON.parse(saved);
        for (const s of sessions) {
          if (s.sshParams) {
            try {
              let pw = s.sshParams.password;
              if (!pw && s.sshParams.authMethod === 'password') {
                const stored = await invoke<string | null>('retrieve_password', {
                  user: s.sshParams.user,
                  host: s.sshParams.host,
                  port: s.sshParams.port,
                });
                if (stored) pw = stored;
              }
              if (pw || s.sshParams.authMethod !== 'password') {
                const id = await sshConnect({
                  host: s.sshParams.host,
                  port: s.sshParams.port,
                  user: s.sshParams.user,
                  password: pw || null,
                  authMethod: s.sshParams.authMethod,
                  keyPath: s.sshParams.keyPath,
                  label: s.label,
                  proxyJump: s.sshParams.proxyJump || null,
                });
                const tab: Tab = { id, label: s.label, type: 'ssh', sshParams: { ...s.sshParams, password: pw || null } };
                setTabs(prev => [...prev, tab]);
                setActiveTabId(id);
                setSplitTrees(prev => ({ ...prev, [id]: { type: 'terminal', terminalId: id } }));
                setFocusedTerminalId(id);
                setTimeout(() => invoke('terminal_resize', { id, cols: 120, rows: 36 }).catch(() => {}), 300);
                startMonitorAndSftp(id, s.sshParams.host, s.sshParams.port, s.sshParams.user, pw || null, s.sshParams.authMethod, s.sshParams.keyPath);
              }
            } catch (e) {
              console.error('Failed to restore session:', s.label, e);
            }
          }
        }
      }
    } catch (_) {}
    // Always open at least one local terminal
    openLocalTerminal();
  }

  // Listen for terminal-closed events (with SSH auto-reconnect)
  useEffect(() => {
    const unlisten = listen<{ id: string }>('terminal-closed', (event) => {
      const closedId = event.payload.id;
      let isReconnecting = false;
      setTabs((prev) => {
        const tab = prev.find(t => t.id === closedId);
        if (tab?.type === 'ssh' && tab.sshParams) {
          // Start reconnection for SSH tabs
          isReconnecting = true;
          setReconnecting(r => ({ ...r, [closedId]: { attempts: 0, status: '正在重连...' } }));
          attemptReconnect(tab);
          return prev; // keep the tab
        }
        // For non-SSH tabs, just remove
        return prev.filter(t => t.id !== closedId);
      });
      if (!isReconnecting) {
        setActiveTabId((prev) => prev === closedId ? null : prev);
      }
    });
    return () => {
      unlisten.then((fn) => fn());
    };
  }, []);

  // Global keyboard shortcuts
  useEffect(() => {
    const handleKeyDown = (e: KeyboardEvent) => {
      const shortcuts = loadShortcuts();
      if (matchShortcut(e, shortcuts.newTab)) { e.preventDefault(); setShowNewTab(true); }
      if (matchShortcut(e, shortcuts.closeTab)) { e.preventDefault(); if (activeTabId) closeTab(activeTabId); }
      if (matchShortcut(e, shortcuts.splitH)) {
        e.preventDefault();
        if (activeTabId && focusedTerminalId) handleSplit(activeTabId, focusedTerminalId, 'horizontal');
      }
      if (matchShortcut(e, shortcuts.splitV)) {
        e.preventDefault();
        if (activeTabId && focusedTerminalId) handleSplit(activeTabId, focusedTerminalId, 'vertical');
      }
      if (matchShortcut(e, shortcuts.nextTab)) {
        e.preventDefault();
        if (tabs.length > 0) {
          const idx = tabs.findIndex(t => t.id === activeTabId);
          const next = tabs[(idx + 1) % tabs.length];
          if (next) setActiveTabId(next.id);
        }
      }
      if (matchShortcut(e, shortcuts.prevTab)) {
        e.preventDefault();
        if (tabs.length > 0) {
          const idx = tabs.findIndex(t => t.id === activeTabId);
          const prev = tabs[(idx - 1 + tabs.length) % tabs.length];
          if (prev) setActiveTabId(prev.id);
        }
      }
    };
    window.addEventListener('keydown', handleKeyDown);
    return () => window.removeEventListener('keydown', handleKeyDown);
  }, [activeTabId, focusedTerminalId, tabs]);

  async function loadConnections() {
    try {
      const store = await invoke<ConnectionStore>('load_connections');
      setConnections(store);
      const expanded: Record<string, boolean> = {};
      for (const gid of Object.keys(store.groups)) {
        expanded[gid] = true;
      }
      setExpandedGroups(expanded);
    } catch (e) {
      console.error('Failed to load connections:', e);
    }
  }

  async function openLocalTerminal() {
    try {
      const id = await invoke<string>('open_local_terminal');
      const localCount = tabs.filter((t) => t.type === 'local').length + 1;
      const tab: Tab = { id, label: `终端 ${localCount}`, type: 'local' };
      setTabs((prev) => [...prev, tab]);
      setActiveTabId(id);
      setSplitTrees(prev => ({ ...prev, [id]: { type: 'terminal', terminalId: id } }));
      setFocusedTerminalId(id);

      setTimeout(() => {
        invoke('terminal_resize', { id, cols: 120, rows: 36 }).catch(() => {});
      }, 300);
    } catch (e) {
      setError(`打开终端失败: ${e}`);
    }
  }

  async function openShellTerminal(shellPath: string, shellName: string) {
    try {
      const id = await invoke<string>('open_shell_terminal', { shellPath });
      const tab: Tab = { id, label: shellName, type: 'local', shellPath };
      setTabs((prev) => [...prev, tab]);
      setActiveTabId(id);
      setSplitTrees(prev => ({ ...prev, [id]: { type: 'terminal', terminalId: id } }));
      setFocusedTerminalId(id);
      setTimeout(() => {
        invoke('terminal_resize', { id, cols: 120, rows: 36 }).catch(() => {});
      }, 300);
    } catch (e) {
      setError(`打开终端失败: ${e}`);
    }
  }

  async function openSerialTerminal(device: string, baudRate: number, name: string) {
    try {
      const id = await invoke<string>('open_serial_terminal', { device, baudRate });
      const tab: Tab = { id, label: `串口: ${name}`, type: 'serial', serialParams: { device, baudRate } };
      setTabs((prev) => [...prev, tab]);
      setActiveTabId(id);
      setSplitTrees(prev => ({ ...prev, [id]: { type: 'terminal', terminalId: id } }));
      setFocusedTerminalId(id);
    } catch (e) {
      setError(`打开串口失败: ${e}`);
    }
  }

  async function startMonitorAndSftp(
    sessionId: string,
    host: string,
    port: number,
    user: string,
    password: string | null,
    authMethod: string,
    keyPath: string | null,
  ) {
    log('SFTP', `启动监控和SFTP, sessionId=${sessionId}, host=${host}, pw=${password ? '有' : '无'}`);

    // Start monitor in background
    invoke('start_monitor', {
      sessionId,
      host,
      port,
      user,
      password: password || null,
      authMethod,
      keyPath: keyPath || null,
    }).catch((e) => log('监控', `启动失败: ${e}`));

    // Delay SFTP start to let SSH session fully stabilize
    log('SFTP', '等待2秒后启动SFTP session...');
    await new Promise(r => setTimeout(r, 2000));

    // SFTP 连接重试，最多3次
    for (let attempt = 1; attempt <= 3; attempt++) {
      try {
        log('SFTP', `第${attempt}次尝试建立SFTP session...`);
        await invoke('start_sftp_session', {
          sessionId,
          host,
          port,
          user,
          password: password || null,
          authMethod,
          keyPath: keyPath || null,
        });
        log('SFTP', `SFTP session 建立成功`);
        setSftpReady(prev => prev + 1);
        return;
      } catch (e) {
        log('SFTP', `第${attempt}次失败: ${e}`);
        if (attempt < 3) await new Promise(r => setTimeout(r, 2000));
      }
    }
    log('SFTP', 'SFTP session 建立最终失败');
  }

  async function attemptReconnect(tab: Tab, attempt = 1) {
    if (attempt > 5) {
      setReconnecting(prev => ({ ...prev, [tab.id]: { attempts: attempt, status: '重连失败' } }));
      return;
    }
    setReconnecting(prev => ({ ...prev, [tab.id]: { attempts: attempt, status: `正在重连... (第 ${attempt} 次)` } }));

    // Exponential backoff: 2s, 4s, 8s, 16s, 32s
    await new Promise(r => setTimeout(r, Math.min(2000 * Math.pow(2, attempt - 1), 32000)));

    // Check if user cancelled while waiting
    let cancelled = false;
    setReconnecting(prev => {
      if (!prev[tab.id]) cancelled = true;
      return prev;
    });
    if (cancelled) return;

    try {
      const newId = await sshConnect({
        host: tab.sshParams!.host,
        port: tab.sshParams!.port,
        user: tab.sshParams!.user,
        password: tab.sshParams!.password,
        authMethod: tab.sshParams!.authMethod,
        keyPath: tab.sshParams!.keyPath,
        label: tab.label,
        proxyJump: tab.sshParams!.proxyJump || null,
      });

      // Success: update the tab with new terminal ID
      setTabs(prev => prev.map(t => t.id === tab.id ? { ...t, id: newId } : t));
      setSplitTrees(prev => {
        const tree = prev[tab.id];
        const next = { ...prev };
        delete next[tab.id];
        next[newId] = tree || { type: 'terminal', terminalId: newId };
        return next;
      });
      setReconnecting(prev => {
        const next = { ...prev };
        delete next[tab.id];
        return next;
      });
      setActiveTabId(newId);
      setFocusedTerminalId(newId);
      setTimeout(() => invoke('terminal_resize', { id: newId, cols: 120, rows: 36 }).catch(() => {}), 300);
      startMonitorAndSftp(newId, tab.sshParams!.host, tab.sshParams!.port,
        tab.sshParams!.user, tab.sshParams!.password ?? null, tab.sshParams!.authMethod, tab.sshParams!.keyPath ?? null);
    } catch {
      attemptReconnect(tab, attempt + 1);
    }
  }

  function openRecordingTab(filePath: string) {
    const id = `recording-${Date.now()}`;
    const fileName = filePath.split('/').pop() || '录屏回放';
    const tab: Tab = { id, label: `回放: ${fileName}`, type: 'recording', recordingPath: filePath };
    setTabs(prev => [...prev, tab]);
    setActiveTabId(id);
  }

  async function handleConnect(params: {
    groupId: string;
    groupLabel: string;
    groupColor: string;
    hostId: string;
    host: string;
    port: number;
    user: string;
    password: string;
    authMethod: AuthMethod;
    keyPath: string;
    label: string;
    proxyJump: string;
  }) {
    setShowDialog(false);
    setError(null);

    try {
      // Save connection
      await invoke('save_connection', {
        groupId: params.groupId,
        groupLabel: params.groupLabel,
        groupColor: params.groupColor,
        hostId: params.hostId,
        config: {
          label: params.label,
          host: params.host,
          port: params.port,
          user: params.user,
          auth: params.authMethod,
          key_path: params.keyPath,
          charset: 'UTF-8',
          proxy_jump: params.proxyJump,
        },
      });
      await loadConnections();

      const resolvedAuthMethod = params.authMethod === 'keyring' ? 'password' : params.authMethod;

      // Connect
      const id = await sshConnect({
        host: params.host,
        port: params.port,
        user: params.user,
        password: params.password || null,
        authMethod: resolvedAuthMethod,
        keyPath: params.keyPath || null,
        label: params.label,
        proxyJump: params.proxyJump || null,
      });

      if (params.password && params.authMethod === 'keyring') {
        invoke('store_password', {
          user: params.user,
          host: params.host,
          port: params.port,
          password: params.password,
        }).catch(() => {});
      }

      const tab: Tab = {
        id,
        label: params.label,
        type: 'ssh',
        sshParams: {
          host: params.host,
          port: params.port,
          user: params.user,
          password: params.password || null,
          authMethod: resolvedAuthMethod,
          keyPath: params.keyPath || null,
          proxyJump: params.proxyJump || null,
        },
      };
      setTabs((prev) => [...prev, tab]);
      setActiveTabId(id);
      setSplitTrees(prev => ({ ...prev, [id]: { type: 'terminal', terminalId: id } }));
      setFocusedTerminalId(id);

      setTimeout(() => {
        invoke('terminal_resize', { id, cols: 120, rows: 36 }).catch(() => {});
      }, 300);

      // Start monitor and SFTP
      startMonitorAndSftp(
        id,
        params.host,
        params.port,
        params.user,
        params.password || null,
        resolvedAuthMethod,
        params.keyPath || null,
      );
    } catch (e) {
      setError(`连接失败: ${e}`);
    }
  }

  async function handleConnectExisting(
    groupId: string,
    hostId: string,
  ) {
    setError(null);
    const group = connections.groups[groupId];
    if (!group) return;
    const hostConfig = group.hosts[hostId];
    if (!hostConfig) return;

    if (hostConfig.auth === 'keyring') {
      try {
        const saved = await invoke<string | null>('retrieve_password', {
          user: hostConfig.user,
          host: hostConfig.host,
          port: hostConfig.port,
        });
        if (saved) {
          doConnect(hostConfig, saved);
          return;
        }
      } catch (_) {}
      setPasswordPrompt({ groupId, hostId, hostLabel: hostConfig.label });
      return;
    }

    doConnect(hostConfig, null);
  }

  async function doConnect(
    hostConfig: { host: string; port: number; user: string; auth: AuthMethod; key_path: string; label: string; proxy_jump?: string },
    password: string | null,
  ) {
    setError(null);
    const resolvedAuthMethod = hostConfig.auth === 'keyring' ? 'password' : hostConfig.auth;
    log('连接', `doConnect ${hostConfig.host}:${hostConfig.port} user=${hostConfig.user} auth=${resolvedAuthMethod} pw=${password ? '有' : '无'}`);
    try {
      const id = await sshConnect({
        host: hostConfig.host,
        port: hostConfig.port,
        user: hostConfig.user,
        password,
        authMethod: resolvedAuthMethod,
        keyPath: hostConfig.key_path || null,
        label: hostConfig.label,
        proxyJump: hostConfig.proxy_jump || null,
      });

      if (password && hostConfig.auth === 'keyring') {
        invoke('store_password', {
          user: hostConfig.user,
          host: hostConfig.host,
          port: hostConfig.port,
          password,
        }).catch(() => {});
      }

      const tab: Tab = {
        id,
        label: hostConfig.label,
        type: 'ssh',
        sshParams: {
          host: hostConfig.host,
          port: hostConfig.port,
          user: hostConfig.user,
          password,
          authMethod: resolvedAuthMethod,
          keyPath: hostConfig.key_path || null,
          proxyJump: hostConfig.proxy_jump || null,
        },
      };
      setTabs((prev) => [...prev, tab]);
      setActiveTabId(id);
      setSplitTrees(prev => ({ ...prev, [id]: { type: 'terminal', terminalId: id } }));
      setFocusedTerminalId(id);

      setTimeout(() => {
        invoke('terminal_resize', { id, cols: 120, rows: 36 }).catch(() => {});
      }, 300);

      // Start monitor and SFTP
      startMonitorAndSftp(
        id,
        hostConfig.host,
        hostConfig.port,
        hostConfig.user,
        password,
        resolvedAuthMethod,
        hostConfig.key_path || null,
      );
    } catch (e) {
      setError(`连接失败: ${e}`);
    }
  }

  function handlePasswordSubmit(password: string) {
    if (!passwordPrompt) return;
    const group = connections.groups[passwordPrompt.groupId];
    if (!group) return;
    const hostConfig = group.hosts[passwordPrompt.hostId];
    if (!hostConfig) return;
    setPasswordPrompt(null);
    doConnect(hostConfig, password);
  }

  async function handleExportConfig() {
    try {
      const content = await invoke<string>('export_config');
      const filePath = await save({
        title: '导出配置',
        defaultPath: 'guishell_connections.toml',
        filters: [{ name: 'TOML 文件', extensions: ['toml'] }],
      });
      if (filePath) {
        const bytes = Array.from(new TextEncoder().encode(content));
        await invoke('save_file', { path: filePath, data: bytes });
      }
    } catch (e) {
      setError(`导出配置失败: ${e}`);
    }
  }

  async function handleImportConfig() {
    try {
      const filePath = await open({
        title: '导入配置',
        multiple: false,
        filters: [{ name: 'TOML 文件', extensions: ['toml'] }],
      });
      if (filePath) {
        const content = await invoke<string>('read_text_file', { path: filePath as string });
        await invoke('import_config', { content });
        await loadConnections();
      }
    } catch (e) {
      setError(`导入配置失败: ${e}`);
    }
  }

  function closeTab(id: string) {
    // Also clean up any reconnecting state
    setReconnecting(prev => {
      if (prev[id]) {
        const next = { ...prev };
        delete next[id];
        return next;
      }
      return prev;
    });
    const tab = tabs.find((t) => t.id === id);
    if (tab && tab.type !== 'process' && tab.type !== 'recording') {
      // Close all terminals in the split tree
      const tree = splitTrees[id];
      if (tree) {
        const termIds = collectTerminalIds(tree);
        termIds.forEach(tid => invoke('close_terminal', { id: tid }).catch(console.error));
      } else {
        invoke('close_terminal', { id }).catch(console.error);
      }
    }
    setSplitTrees(prev => {
      const next = { ...prev };
      delete next[id];
      return next;
    });
    setTabs((prev) => prev.filter((t) => t.id !== id));
    setActiveTabId((prev) => {
      if (prev !== id) return prev;
      const remaining = tabs.filter((t) => t.id !== id);
      return remaining.length > 0 ? remaining[remaining.length - 1].id : null;
    });
  }

  function closeOtherTabs(keepId: string) {
    tabs.forEach((t) => {
      if (t.id !== keepId && t.type !== 'process') {
        const tree = splitTrees[t.id];
        if (tree) {
          collectTerminalIds(tree).forEach(tid => invoke('close_terminal', { id: tid }).catch(console.error));
        } else {
          invoke('close_terminal', { id: t.id }).catch(console.error);
        }
      }
    });
    setSplitTrees(prev => {
      const next: Record<string, SplitNode> = {};
      if (prev[keepId]) next[keepId] = prev[keepId];
      return next;
    });
    setTabs((prev) => prev.filter((t) => t.id === keepId));
    setActiveTabId(keepId);
  }

  async function duplicateTab(id: string) {
    const tab = tabs.find((t) => t.id === id);
    if (!tab) return;

    if (tab.type === 'ssh' && tab.sshParams) {
      try {
        const newId = await sshConnect({
          host: tab.sshParams.host,
          port: tab.sshParams.port,
          user: tab.sshParams.user,
          password: tab.sshParams.password,
          authMethod: tab.sshParams.authMethod,
          keyPath: tab.sshParams.keyPath,
          label: tab.label,
          proxyJump: tab.sshParams.proxyJump || null,
        });
        const newTab: Tab = {
          id: newId,
          label: tab.label,
          type: 'ssh',
          sshParams: { ...tab.sshParams },
        };
        setTabs((prev) => [...prev, newTab]);
        setActiveTabId(newId);
        setSplitTrees(prev => ({ ...prev, [newId]: { type: 'terminal', terminalId: newId } }));
        setFocusedTerminalId(newId);
        setTimeout(() => {
          invoke('terminal_resize', { id: newId, cols: 120, rows: 36 }).catch(() => {});
        }, 300);
        startMonitorAndSftp(
          newId,
          tab.sshParams.host,
          tab.sshParams.port,
          tab.sshParams.user,
          tab.sshParams.password,
          tab.sshParams.authMethod,
          tab.sshParams.keyPath,
        );
      } catch (e) {
        setError(`复制连接失败: ${e}`);
      }
    } else if (tab.type === 'serial' && tab.serialParams) {
      openSerialTerminal(tab.serialParams.device, tab.serialParams.baudRate, tab.label.replace('串口: ', ''));
    } else if (tab.shellPath) {
      openShellTerminal(tab.shellPath, tab.label);
    } else {
      openLocalTerminal();
    }
  }

  function toggleGroup(groupId: string) {
    setExpandedGroups((prev) => ({
      ...prev,
      [groupId]: !prev[groupId],
    }));
  }

  function openProcessManager() {
    if (!activeTab?.sshParams) return;
    const id = `process-${activeTab.id}`;
    // If a process tab for this session already exists, just switch to it
    const existing = tabs.find((t) => t.id === id);
    if (existing) {
      setActiveTabId(id);
      return;
    }
    const tab: Tab = {
      id,
      label: `进程 - ${activeTab.label}`,
      type: 'process',
      sshParams: activeTab.sshParams,
    };
    setTabs((prev) => [...prev, tab]);
    setActiveTabId(id);
  }

  async function handleSplit(tabId: string, terminalId: string, direction: 'horizontal' | 'vertical') {
    const tab = tabs.find(t => t.id === tabId);
    let newTerminalId: string;

    try {
      if (tab?.type === 'ssh' && tab.sshParams) {
        newTerminalId = await sshConnect({
          host: tab.sshParams.host,
          port: tab.sshParams.port,
          user: tab.sshParams.user,
          password: tab.sshParams.password,
          authMethod: tab.sshParams.authMethod,
          keyPath: tab.sshParams.keyPath,
          label: tab.label,
          proxyJump: tab.sshParams.proxyJump || null,
        });
      } else {
        newTerminalId = await invoke<string>('open_local_terminal');
      }

      setTimeout(() => invoke('terminal_resize', { id: newTerminalId, cols: 80, rows: 24 }).catch(() => {}), 300);

      setSplitTrees(prev => {
        const tree = prev[tabId];
        if (!tree) return prev;
        const newTree = replaceNode(tree, terminalId, {
          type: 'split',
          direction,
          first: { type: 'terminal', terminalId },
          second: { type: 'terminal', terminalId: newTerminalId },
          ratio: 0.5,
        });
        return { ...prev, [tabId]: newTree };
      });

      setFocusedTerminalId(newTerminalId);
    } catch (e) {
      setError(`分屏失败: ${e}`);
    }
  }

  function handleClosePane(tabId: string, terminalId: string) {
    invoke('close_terminal', { id: terminalId }).catch(() => {});
    setSplitTrees(prev => {
      const tree = prev[tabId];
      if (!tree) return prev;
      const newTree = removeNode(tree, terminalId);
      return { ...prev, [tabId]: newTree || { type: 'terminal', terminalId: tabId } };
    });
  }

  const [showSshKeyManager, setShowSshKeyManager] = useState(false);
  const [showBatchCommand, setShowBatchCommand] = useState(false);
  const [showShortcutSettings, setShowShortcutSettings] = useState(false);
  const [showTunnelManager, setShowTunnelManager] = useState(false);
  const [sidebarConnectionsOpen, setSidebarConnectionsOpen] = useState(true);
  const [fileBrowserOpen, setFileBrowserOpen] = useState(true);
  const [sftpReady, setSftpReady] = useState(0);
  const [dragOverTerminal, setDragOverTerminal] = useState(false);
  const [showLogPanel, setShowLogPanel] = useState(false);
  const [globalTransfers, setGlobalTransfers] = useState<Record<string, { filename: string; direction: string; bytes: number; total: number; speed: number; target?: string }>>({});
  const transferStatsRef = useRef<Record<string, { lastBytes: number; lastTime: number; ema: number }>>({});
  // 记录文件管理器当前远程路径 + 其所属会话；拖拽上传时按会话校验，避免跨会话误用旧路径
  const currentRemotePathRef = useRef<{ sid: string; path: string } | null>(null);
  const [transferPanelVisible, setTransferPanelVisible] = useState(true);
  const [tabContextMenu, setTabContextMenu] = useState<{ x: number; y: number; tabId: string } | null>(null);
  const [renameTab, setRenameTab] = useState<{ tabId: string; name: string } | null>(null);
  const [connContextMenu, setConnContextMenu] = useState<{ x: number; y: number; groupId: string; hostId: string } | null>(null);
  const [editingConn, setEditingConn] = useState<{ groupId: string; hostId: string } | null>(null);
  const [sidebarWidth, setSidebarWidth] = useState(240);
  const sidebarDragging = useRef(false);
  const [fileBrowserHeight, setFileBrowserHeight] = useState(256);
  const fbDragging = useRef(false);

  // 拖拽文件到终端上传：通过 SFTP 独立通道上传（drag_upload，Rust 后端处理）
  useEffect(() => {
    const webview = getCurrentWebview();
    const unlisten = webview.onDragDropEvent((event) => {
      if (!activeSshSessionId) return;
      if (event.payload.type === 'enter' || event.payload.type === 'over') {
        setDragOverTerminal(true);
      } else if (event.payload.type === 'leave') {
        setDragOverTerminal(false);
      } else if (event.payload.type === 'drop') {
        setDragOverTerminal(false);
        const paths = event.payload.paths;
        if (!paths.length) return;
        const sid = activeSshSessionId;
        // 优先使用文件管理器当前远程路径作为目标目录（仅当属于当前会话），否则交由后端兜底
        const fb = currentRemotePathRef.current;
        const fallbackDir = (fb && fb.sid === sid) ? fb.path : null;
        log('拖拽上传', `${paths.length} 个文件通过 SFTP 上传`, paths);
        invoke('drag_upload', { sessionId: sid, files: paths, fallbackDir })
          .then(() => log('拖拽上传', '完成'))
          .catch((e) => {
            log('拖拽上传', `失败: ${e}`);
            setError(`上传失败: ${e}`);
          });
      }
    });
    return () => { unlisten.then(fn => fn()); };
  }, [activeSshSessionId]);

  // 全局传输进度监听（含实时速率计算）
  useEffect(() => {
    const unlisten = listen<{
      filename: string; bytes_transferred: number; total_bytes: number; direction: string; target?: string;
    }>('transfer-progress', (event) => {
      const { filename, bytes_transferred, total_bytes, direction } = event.payload;
      // 目标目录（拖拽上传时后端附带），用于浮窗展示
      const target = event.payload.target;
      const key = `${direction}-${filename}`;
      const now = Date.now();
      // 根据相邻两次进度事件的字节增量与时间差计算瞬时速率，并做指数平滑
      const prev = transferStatsRef.current[key];
      let speed = 0;
      if (prev && now > prev.lastTime) {
        const inst = (bytes_transferred - prev.lastBytes) * 1000 / (now - prev.lastTime);
        speed = prev.ema > 0 ? prev.ema * 0.6 + inst * 0.4 : inst;
      }
      transferStatsRef.current[key] = { lastBytes: bytes_transferred, lastTime: now, ema: speed };
      setGlobalTransfers(prev => ({
        ...prev,
        [key]: { filename, direction, bytes: bytes_transferred, total: total_bytes, speed, target },
      }));
      if (bytes_transferred >= total_bytes && total_bytes > 0) {
        setTimeout(() => {
          setGlobalTransfers(prev => { const n = { ...prev }; delete n[key]; return n; });
          delete transferStatsRef.current[key];
        }, 3000);
      }
    });
    return () => { unlisten.then(fn => fn()); };
  }, []);

  const activeTransfers = Object.values(globalTransfers);

  return (
    <div className="h-screen w-screen flex bg-surface text-gray-200 overflow-hidden" style={{ maxHeight: '100vh', maxWidth: '100vw' }}>
      {/* Left: System Info Panel (resizable) */}
      <aside
        className="flex-shrink-0 bg-surface-light overflow-y-auto overflow-x-hidden"
        style={{ width: `${sidebarWidth}px` }}
        onContextMenu={(e) => e.preventDefault()}
      >
        {/* Connection list (collapsible) */}
        <div className="border-b border-surface-border">
          <div className="px-3 py-2 text-xs font-semibold text-gray-400 flex items-center justify-between">
            <button
              onClick={() => setSidebarConnectionsOpen(!sidebarConnectionsOpen)}
              className="flex items-center gap-1 hover:text-gray-200"
            >
              {sidebarConnectionsOpen ? <IconChevronDown size={12} /> : <IconChevronRight size={12} />}
              <span>连接管理</span>
            </button>
            <div className="flex items-center gap-1">
              <button
                onClick={handleImportConfig}
                className="text-gray-500 hover:text-accent-cyan p-0.5"
                title="导入配置"
              ><IconImport size={13} /></button>
              <button
                onClick={handleExportConfig}
                className="text-gray-500 hover:text-accent-cyan p-0.5"
                title="导出配置"
              ><IconExport size={13} /></button>
              <button
                onClick={() => setShowSshKeyManager(true)}
                className="text-gray-500 hover:text-accent-cyan p-0.5"
                title="SSH 密钥管理"
              ><IconKey size={13} /></button>
              <button
                onClick={() => setShowDialog(true)}
                className="text-accent-cyan hover:text-white p-0.5"
                title="新建连接"
              >
                <IconPlus size={15} />
              </button>
            </div>
          </div>
          {sidebarConnectionsOpen && (
            <div className="px-2 pb-2">
              {Object.keys(connections.groups).length === 0 ? (
                <div className="text-gray-500 text-xs px-2">暂无连接</div>
              ) : (
                Object.entries(connections.groups).map(([groupId, group]) => (
                  <div key={groupId} className="mb-0.5">
                    <button
                      onClick={() => toggleGroup(groupId)}
                      className="w-full text-left px-2 py-0.5 text-xs font-semibold text-gray-400 hover:text-gray-200 flex items-center gap-1"
                    >
                      <span className="text-[10px]">
                        {expandedGroups[groupId] ? '▼' : '▶'}
                      </span>
                      <span
                        className="w-2 h-2 rounded-full inline-block"
                        style={{ backgroundColor: group.color }}
                      />
                      {group.label}
                    </button>
                    {expandedGroups[groupId] &&
                      Object.entries(group.hosts).map(([hostId, host]) => (
                        <button
                          key={hostId}
                          onClick={() => handleConnectExisting(groupId, hostId)}
                          onContextMenu={(e) => {
                            e.preventDefault();
                            e.stopPropagation();
                            setConnContextMenu({ x: e.clientX, y: e.clientY, groupId, hostId });
                          }}
                          className="w-full text-left pl-6 pr-2 py-0.5 text-xs text-gray-300 hover:bg-surface-lighter rounded truncate"
                          title={`点击连接 ${host.host}:${host.port}`}
                        >
                          {host.label}
                        </button>
                      ))}
                  </div>
                ))
              )}
            </div>
          )}
        </div>

        {/* Connection context menu */}
        {connContextMenu && (
          <>
            <div className="fixed inset-0 z-40" onClick={() => setConnContextMenu(null)} onContextMenu={(e) => { e.preventDefault(); setConnContextMenu(null); }} />
            <div
              className="fixed z-50 bg-surface-light border border-surface-border rounded shadow-lg py-1 min-w-[140px]"
              style={{ left: connContextMenu.x, top: connContextMenu.y }}
            >
              <button
                className="w-full text-left px-3 py-1.5 text-sm hover:bg-surface-lighter text-gray-200"
                onClick={() => { handleConnectExisting(connContextMenu.groupId, connContextMenu.hostId); setConnContextMenu(null); }}
              >
                连接
              </button>
              <button
                className="w-full text-left px-3 py-1.5 text-sm hover:bg-surface-lighter text-gray-200"
                onClick={() => { setShowDialog(true); setConnContextMenu(null); }}
              >
                新建会话
              </button>
              <div className="border-t border-surface-border my-1" />
              <button
                className="w-full text-left px-3 py-1.5 text-sm hover:bg-surface-lighter text-gray-200"
                onClick={() => { setEditingConn({ groupId: connContextMenu.groupId, hostId: connContextMenu.hostId }); setShowDialog(true); setConnContextMenu(null); }}
              >
                编辑属性
              </button>
              <button
                className="w-full text-left px-3 py-1.5 text-sm hover:bg-surface-lighter text-accent-red"
                onClick={async () => {
                  const { groupId, hostId } = connContextMenu;
                  try {
                    await invoke('delete_connection', { groupId, hostId });
                    await loadConnections();
                  } catch (e) { setError(`删除失败: ${e}`); }
                  setConnContextMenu(null);
                }}
              >
                删除
              </button>
            </div>
          </>
        )}

        {/* System info — local or remote */}
        <SystemInfoPanel
          sessionId={activeSshSessionId || 'local'}
          hostIp={activeSshSessionId ? activeTab?.sshParams?.host : '本机'}
          onOpenProcessManager={activeSshSessionId ? openProcessManager : undefined}
        />
      </aside>

      {/* Sidebar resize handle */}
      <div
        className="w-1 flex-shrink-0 cursor-col-resize hover:bg-accent-cyan/30 active:bg-accent-cyan/50"
        onMouseDown={(e) => {
          e.preventDefault();
          sidebarDragging.current = true;
          const startX = e.clientX;
          const startW = sidebarWidth;
          const onMove = (ev: MouseEvent) => {
            if (!sidebarDragging.current) return;
            const newW = Math.max(150, Math.min(500, startW + ev.clientX - startX));
            setSidebarWidth(newW);
          };
          const onUp = () => {
            sidebarDragging.current = false;
            document.removeEventListener('mousemove', onMove);
            document.removeEventListener('mouseup', onUp);
          };
          document.addEventListener('mousemove', onMove);
          document.addEventListener('mouseup', onUp);
        }}
      />

      {/* Right: Main content */}
      <main className="flex-1 flex flex-col min-w-0 min-h-0 overflow-hidden">
        {/* Tab bar */}
        <div className="h-9 bg-surface-light border-b border-surface-border flex items-center px-1 gap-1 overflow-x-auto"
          onContextMenu={(e) => e.preventDefault()}
        >
          {tabs.map((tab) => (
            <div
              key={tab.id}
              onClick={() => {
                setActiveTabId(tab.id);
                const tree = splitTrees[tab.id];
                if (tree) {
                  const termIds = collectTerminalIds(tree);
                  if (termIds.length > 0) setFocusedTerminalId(termIds[0]);
                }
              }}
              onContextMenu={(e) => {
                e.preventDefault();
                e.stopPropagation();
                setTabContextMenu({ x: e.clientX, y: e.clientY, tabId: tab.id });
              }}
              className={`px-3 py-1 rounded text-sm cursor-pointer flex items-center gap-1 flex-shrink-0 border ${
                activeTabId === tab.id
                  ? 'bg-surface text-accent-cyan border-surface-border'
                  : 'text-gray-400 hover:text-gray-200 border-transparent hover:border-surface-border'
              }`}
            >
              {tab.type === 'ssh' && (
                <span className="text-accent-green text-xs">{'●'}</span>
              )}
              {tab.type === 'process' && (
                <span className="text-accent-cyan text-xs">{'◉'}</span>
              )}
              {tab.type === 'serial' && (
                <span className="text-accent-yellow text-xs">{'●'}</span>
              )}
              {tab.type === 'recording' && (
                <span className="text-accent-cyan text-xs">{'▶'}</span>
              )}
              {tab.label}
              <span
                onClick={(e) => {
                  e.stopPropagation();
                  closeTab(tab.id);
                }}
                className="ml-1 text-gray-500 hover:text-white cursor-pointer"
                title="关闭"
              >
                {'×'}
              </span>
            </div>
          ))}
          <button
            onClick={() => setShowNewTab(true)}
            className="px-2 py-1 text-gray-500 hover:text-white text-sm flex-shrink-0"
            title="新建标签页"
          >
            <IconPlus size={14} />
          </button>
          <div className="flex-1" />
          <button
            onClick={() => setShowBatchCommand(true)}
            className="px-1.5 py-1 text-gray-500 hover:text-accent-cyan flex-shrink-0"
            title="批量命令"
          >
            <IconBatchCmd size={14} />
          </button>
          <button
            onClick={() => setShowTunnelManager(true)}
            className="px-1.5 py-1 text-gray-500 hover:text-accent-cyan flex-shrink-0"
            title="SSH 隧道"
          >
            <IconTunnel size={14} />
          </button>
          <button
            onClick={() => setShowShortcutSettings(true)}
            className="px-1.5 py-1 text-gray-500 hover:text-accent-cyan flex-shrink-0"
            title="快捷键设置"
          >
            <IconSettings size={14} />
          </button>
          {import.meta.env.DEV && (
            <button
              onClick={() => setShowLogPanel(prev => !prev)}
              className={`px-1.5 py-1 flex-shrink-0 ${showLogPanel ? 'text-accent-cyan' : 'text-gray-500 hover:text-accent-cyan'}`}
              title="调试日志"
            >
              <IconLog size={14} />
            </button>
          )}
        </div>

        {/* Tab context menu */}
        {tabContextMenu && (
          <>
            <div className="fixed inset-0 z-40" onClick={() => setTabContextMenu(null)} onContextMenu={(e) => { e.preventDefault(); setTabContextMenu(null); }} />
            <div
              className="fixed z-50 bg-surface-light border border-surface-border rounded shadow-lg py-1 min-w-[160px]"
              style={{ left: tabContextMenu.x, top: tabContextMenu.y }}
            >
              <button
                className="w-full text-left px-3 py-1.5 text-sm hover:bg-surface-lighter text-gray-200"
                onClick={() => {
                  const tab = tabs.find(t => t.id === tabContextMenu.tabId);
                  setRenameTab({ tabId: tabContextMenu.tabId, name: tab?.label || '' });
                  setTabContextMenu(null);
                }}
              >
                重命名
              </button>
              <button
                className="w-full text-left px-3 py-1.5 text-sm hover:bg-surface-lighter text-gray-200"
                onClick={() => { duplicateTab(tabContextMenu.tabId); setTabContextMenu(null); }}
              >
                复制标签页
              </button>
              {(() => {
                const t = tabs.find(t => t.id === tabContextMenu.tabId);
                return t?.type === 'ssh' && t.sshParams ? (
                  <button
                    className="w-full text-left px-3 py-1.5 text-sm hover:bg-surface-lighter text-accent-cyan"
                    onClick={async () => {
                      const tab = t;
                      const sp = tab.sshParams!;
                      setTabContextMenu(null);
                      log('重连', `开始重连 ${sp.host}:${sp.port}, authMethod=${sp.authMethod}, password=${sp.password ? '有' : '无'}`);

                      // 如果密码为空且认证方式是密码，先从 keyring 取
                      let pw = sp.password;
                      if (!pw && sp.authMethod === 'password') {
                        try {
                          const stored = await invoke<string | null>('retrieve_password', {
                            user: sp.user, host: sp.host, port: sp.port,
                          });
                          if (stored) pw = stored;
                          log('重连', `从 keyring 获取密码: ${stored ? '成功' : '失败'}`);
                        } catch (e) {
                          log('重连', `keyring 查询异常: ${e}`);
                        }
                      }

                      // 清除报错和传输弹窗
                      setError(null);
                      setGlobalTransfers({});

                      // 关闭旧终端
                      log('重连', `关闭旧终端 ${tab.id}`);
                      invoke('close_terminal', { id: tab.id }).catch(() => {});

                      try {
                        log('重连', `发起 SSH 连接...`);
                        const newId = await sshConnect({
                          host: sp.host, port: sp.port, user: sp.user,
                          password: pw, authMethod: sp.authMethod,
                          keyPath: sp.keyPath, proxyJump: sp.proxyJump || null,
                          label: tab.label,
                        });
                        log('重连', `连接成功, newId=${newId}`);

                        setTabs(prev => prev.map(x => x.id === tab.id ? { ...x, id: newId, sshParams: { ...sp, password: pw } } : x));
                        setSplitTrees(prev => {
                          const next = { ...prev };
                          delete next[tab.id];
                          next[newId] = { type: 'terminal' as const, terminalId: newId };
                          return next;
                        });
                        setActiveTabId(newId);
                        setFocusedTerminalId(newId);
                        setTimeout(() => invoke('terminal_resize', { id: newId, cols: 120, rows: 36 }).catch(() => {}), 300);
                        startMonitorAndSftp(newId, sp.host, sp.port, sp.user, pw, sp.authMethod, sp.keyPath);
                      } catch (e) {
                        log('重连', `失败: ${e}`);
                        setError(`重连失败: ${e}`);
                      }
                    }}
                  >
                    重新连接
                  </button>
                ) : null;
              })()}
              <div className="border-t border-surface-border my-1" />
              <button
                className="w-full text-left px-3 py-1.5 text-sm hover:bg-surface-lighter text-gray-200"
                onClick={() => { closeTab(tabContextMenu.tabId); setTabContextMenu(null); }}
              >
                关闭
              </button>
              <button
                className="w-full text-left px-3 py-1.5 text-sm hover:bg-surface-lighter text-gray-200"
                onClick={() => { closeOtherTabs(tabContextMenu.tabId); setTabContextMenu(null); }}
              >
                关闭其他
              </button>
              <div className="border-t border-surface-border my-1" />
              <button
                className="w-full text-left px-3 py-1.5 text-sm hover:bg-surface-lighter text-gray-200"
                onClick={() => { openLocalTerminal(); setTabContextMenu(null); }}
              >
                新建本地终端
              </button>
            </div>
          </>
        )}

        {/* Rename tab dialog */}
        {renameTab && (
          <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/40" onClick={() => setRenameTab(null)}>
            <div className="bg-surface-light border border-surface-border rounded-lg p-4 min-w-[300px] shadow-xl" onClick={(e) => e.stopPropagation()}>
              <div className="text-sm text-gray-200 mb-3">重命名标签页</div>
              <input
                autoFocus
                defaultValue={renameTab.name}
                onKeyDown={(e) => {
                  if (e.key === 'Enter') {
                    const val = e.currentTarget.value.trim();
                    if (val) {
                      setTabs(prev => prev.map(t => t.id === renameTab.tabId ? { ...t, label: val } : t));
                      setRenameTab(null);
                    }
                  }
                  if (e.key === 'Escape') setRenameTab(null);
                }}
                className="w-full bg-surface border border-surface-border rounded px-2 py-1 text-sm text-gray-200 outline-none focus:border-accent-cyan"
              />
              <div className="flex justify-end gap-2 mt-3">
                <button onClick={() => setRenameTab(null)} className="px-3 py-1 text-xs text-gray-400 hover:text-white">取消</button>
                <button
                  onClick={() => {
                    const input = document.querySelector<HTMLInputElement>('.bg-surface-light input');
                    const val = input?.value.trim();
                    if (val) {
                      setTabs(prev => prev.map(t => t.id === renameTab.tabId ? { ...t, label: val } : t));
                      setRenameTab(null);
                    }
                  }}
                  className="px-3 py-1 text-xs bg-accent-cyan/20 text-accent-cyan rounded hover:bg-accent-cyan/30"
                >确定</button>
              </div>
            </div>
          </div>
        )}

        {/* Error banner */}
        {error && (
          <div className="px-3 py-2 bg-accent-red/10 border-b border-accent-red/30 text-accent-red text-xs flex items-center justify-between">
            <span>{error}</span>
            <button onClick={() => setError(null)} className="hover:text-white ml-2">
              {'×'}
            </button>
          </div>
        )}

        {/* Terminal area — isolated from flex to prevent xterm canvas from blowing layout */}
        <div className="flex-1 bg-black relative" style={{ minHeight: 0, minWidth: 0, overflow: 'hidden' }}>
          {tabs.length === 0 ? (
            <div className="p-4 text-gray-500 font-mono text-sm">
              暂无终端。点击 + 新建一个。
            </div>
          ) : (
            tabs.map((tab) =>
              tab.type === 'process' ? (
                <div
                  key={tab.id}
                  style={{
                    display: tab.id === activeTabId ? 'flex' : 'none',
                    position: 'absolute',
                    inset: 0,
                    overflow: 'hidden',
                  }}
                >
                  <ProcessTable
                    sshParams={tab.sshParams!}
                    hostLabel={tab.label}
                  />
                </div>
              ) : tab.type === 'recording' ? (
                <div
                  key={tab.id}
                  style={{
                    display: tab.id === activeTabId ? 'flex' : 'none',
                    position: 'absolute',
                    inset: 0,
                    overflow: 'hidden',
                  }}
                >
                  <RecordingPlayer
                    filePath={tab.recordingPath!}
                    onClose={() => closeTab(tab.id)}
                  />
                </div>
              ) : splitTrees[tab.id] ? (
                <div
                  key={tab.id}
                  style={{
                    display: tab.id === activeTabId ? 'block' : 'none',
                    position: 'absolute',
                    inset: 0,
                    overflow: 'hidden',
                  }}
                >
                  <SplitContainer
                    node={splitTrees[tab.id]}
                    isActive={tab.id === activeTabId}
                    activeTerminalId={focusedTerminalId}
                    onSplit={(termId, dir) => handleSplit(tab.id, termId, dir)}
                    onClose={(termId) => handleClosePane(tab.id, termId)}
                    onFocusTerminal={setFocusedTerminalId}
                    onOpenRecording={openRecordingTab}
                  />
                  {dragOverTerminal && tab.id === activeTabId && tab.type === 'ssh' && (
                    <div style={{
                      position: 'absolute', inset: 0, zIndex: 30,
                      display: 'flex', alignItems: 'center', justifyContent: 'center',
                      background: 'rgba(0,212,255,0.08)',
                      border: '2px dashed #00d4ff',
                      borderRadius: '4px',
                      pointerEvents: 'none',
                    }}>
                      <span style={{ color: '#00d4ff', fontSize: '14px', fontWeight: 600 }}>
                        释放文件上传到远程服务器
                      </span>
                    </div>
                  )}
                  {reconnecting[tab.id] && (
                    <div style={{
                      position: 'absolute', inset: 0, zIndex: 20,
                      display: 'flex', flexDirection: 'column', alignItems: 'center', justifyContent: 'center',
                      background: 'rgba(13,17,23,0.9)',
                    }}>
                      <div style={{ color: '#00d4ff', fontSize: '14px', marginBottom: '8px' }}>
                        {reconnecting[tab.id].status}
                      </div>
                      {reconnecting[tab.id].attempts > 5 && (
                        <div style={{ display: 'flex', gap: '8px', marginBottom: '8px' }}>
                          <button
                            onClick={() => attemptReconnect(tab, 1)}
                            style={{
                              background: '#00d4ff', color: '#0d1117', border: 'none',
                              borderRadius: '4px', padding: '4px 16px', cursor: 'pointer', fontSize: '13px',
                            }}
                          >重试</button>
                          <button
                            onClick={() => {
                              setReconnecting(prev => { const n = { ...prev }; delete n[tab.id]; return n; });
                              closeTab(tab.id);
                            }}
                            style={{
                              background: '#30363d', color: '#e6edf3', border: 'none',
                              borderRadius: '4px', padding: '4px 16px', cursor: 'pointer', fontSize: '13px',
                            }}
                          >关闭</button>
                        </div>
                      )}
                      <button
                        onClick={() => {
                          setReconnecting(prev => { const n = { ...prev }; delete n[tab.id]; return n; });
                          closeTab(tab.id);
                        }}
                        style={{
                          color: '#8b949e', fontSize: '12px', marginTop: '4px', cursor: 'pointer',
                          background: 'none', border: 'none',
                        }}
                      >
                        取消重连
                      </button>
                    </div>
                  )}
                </div>
              ) : (
                <div
                  key={tab.id}
                  style={{
                    display: tab.id === activeTabId ? 'block' : 'none',
                    position: 'absolute',
                    inset: 0,
                    overflow: 'hidden',
                  }}
                >
                  <TerminalPane
                    terminalId={tab.id}
                    isActive={tab.id === activeTabId}
                    onOpenRecording={openRecordingTab}
                  />
                  {reconnecting[tab.id] && (
                    <div style={{
                      position: 'absolute', inset: 0, zIndex: 20,
                      display: 'flex', flexDirection: 'column', alignItems: 'center', justifyContent: 'center',
                      background: 'rgba(13,17,23,0.9)',
                    }}>
                      <div style={{ color: '#00d4ff', fontSize: '14px', marginBottom: '8px' }}>
                        {reconnecting[tab.id].status}
                      </div>
                      {reconnecting[tab.id].attempts > 5 && (
                        <div style={{ display: 'flex', gap: '8px', marginBottom: '8px' }}>
                          <button
                            onClick={() => attemptReconnect(tab, 1)}
                            style={{
                              background: '#00d4ff', color: '#0d1117', border: 'none',
                              borderRadius: '4px', padding: '4px 16px', cursor: 'pointer', fontSize: '13px',
                            }}
                          >重试</button>
                          <button
                            onClick={() => {
                              setReconnecting(prev => { const n = { ...prev }; delete n[tab.id]; return n; });
                              closeTab(tab.id);
                            }}
                            style={{
                              background: '#30363d', color: '#e6edf3', border: 'none',
                              borderRadius: '4px', padding: '4px 16px', cursor: 'pointer', fontSize: '13px',
                            }}
                          >关闭</button>
                        </div>
                      )}
                      <button
                        onClick={() => {
                          setReconnecting(prev => { const n = { ...prev }; delete n[tab.id]; return n; });
                          closeTab(tab.id);
                        }}
                        style={{
                          color: '#8b949e', fontSize: '12px', marginTop: '4px', cursor: 'pointer',
                          background: 'none', border: 'none',
                        }}
                      >
                        取消重连
                      </button>
                    </div>
                  )}
                </div>
              )
            )
          )}
        </div>

        {/* Command input bar */}
        <CommandInputBar
          terminalId={focusedTerminalId || activeTabId}
        />

        {/* File browser: drag handle + toggle + panel */}
        {fileBrowserOpen && (
          <div
            className="h-1 flex-shrink-0 cursor-row-resize hover:bg-accent-cyan/30 active:bg-accent-cyan/50"
            onMouseDown={(e) => {
              e.preventDefault();
              fbDragging.current = true;
              const startY = e.clientY;
              const startH = fileBrowserHeight;
              const onMove = (ev: MouseEvent) => {
                if (!fbDragging.current) return;
                const newH = Math.max(100, Math.min(600, startH - (ev.clientY - startY)));
                setFileBrowserHeight(newH);
              };
              const onUp = () => {
                fbDragging.current = false;
                document.removeEventListener('mousemove', onMove);
                document.removeEventListener('mouseup', onUp);
              };
              document.addEventListener('mousemove', onMove);
              document.addEventListener('mouseup', onUp);
            }}
          />
        )}
        <div
          className="h-6 border-t border-surface-border bg-surface-light flex items-center justify-center cursor-pointer hover:bg-surface-lighter select-none flex-shrink-0"
          onClick={() => setFileBrowserOpen(!fileBrowserOpen)}
        >
          <span className="text-xs text-gray-500">{fileBrowserOpen ? '▼ 隐藏文件管理器' : '▲ 显示文件管理器'}</span>
        </div>
        {fileBrowserOpen && (
          <div
            className="border-t border-surface-border bg-surface-light flex flex-col file-browser-panel flex-shrink-0"
            style={{ height: `${fileBrowserHeight}px` }}
          >
            <FileBrowser sessionId={activeSshSessionId} activeTerminalId={activeTabId} sshUser={activeTab?.sshParams?.user} sftpReady={sftpReady} onRemotePathChange={(p) => { if (activeSshSessionId) currentRemotePathRef.current = { sid: activeSshSessionId, path: p }; }} />
          </div>
        )}
      </main>

      {/* New tab selector */}
      {showNewTab && (
        <NewTabSelector
          onClose={() => setShowNewTab(false)}
          onOpenShell={openShellTerminal}
          onConnectSSH={(groupId, hostId) => { setShowNewTab(false); handleConnectExisting(groupId, hostId); }}
          onNewSSH={() => { setShowNewTab(false); setShowDialog(true); }}
          onOpenSerial={openSerialTerminal}
          connections={connections}
        />
      )}

      {/* Connection dialog */}
      {showDialog && (
        <ConnectionDialog
          onClose={() => { setShowDialog(false); setEditingConn(null); }}
          onConnect={(params) => {
            if (editingConn) {
              // Edit mode: save config and reload, don't connect
              invoke('save_connection', {
                groupId: params.groupId,
                groupLabel: params.groupLabel,
                groupColor: params.groupColor,
                hostId: params.hostId,
                config: {
                  label: params.label,
                  host: params.host,
                  port: params.port,
                  user: params.user,
                  auth: params.authMethod,
                  key_path: params.keyPath,
                  charset: 'UTF-8',
                  proxy_jump: params.proxyJump,
                },
              }).then(() => {
                if (params.password && params.authMethod === 'keyring') {
                  invoke('store_password', { user: params.user, host: params.host, port: params.port, password: params.password }).catch(() => {});
                }
                loadConnections();
              }).catch(e => setError(`保存失败: ${e}`));
              setShowDialog(false);
              setEditingConn(null);
            } else {
              handleConnect(params);
            }
          }}
          onSaveOnly={(params) => {
            invoke('save_connection', {
              groupId: params.groupId, groupLabel: params.groupLabel, groupColor: params.groupColor,
              hostId: params.hostId,
              config: { label: params.label, host: params.host, port: params.port, user: params.user, auth: params.authMethod, key_path: params.keyPath, charset: 'UTF-8', proxy_jump: params.proxyJump },
            }).then(() => {
              if (params.password && params.authMethod === 'keyring') {
                invoke('store_password', { user: params.user, host: params.host, port: params.port, password: params.password }).catch(() => {});
              }
              loadConnections();
            }).catch(e => setError(`保存失败: ${e}`));
            setShowDialog(false);
          }}
          editData={editingConn ? (() => {
            const g = connections.groups[editingConn.groupId];
            const h = g?.hosts[editingConn.hostId];
            return h ? { groupId: editingConn.groupId, hostId: editingConn.hostId, host: h } : undefined;
          })() : undefined}
        />
      )}

      {/* Password prompt */}
      {passwordPrompt && (
        <PasswordPrompt
          hostLabel={passwordPrompt.hostLabel}
          onSubmit={handlePasswordSubmit}
          onCancel={() => setPasswordPrompt(null)}
        />
      )}

      {/* SSH Key Manager */}
      {showSshKeyManager && (
        <SshKeyManager onClose={() => setShowSshKeyManager(false)} />
      )}

      {/* Batch Command */}
      {showBatchCommand && (
        <BatchCommand onClose={() => setShowBatchCommand(false)} tabs={tabs} />
      )}

      {/* Shortcut Settings */}
      {showShortcutSettings && (
        <ShortcutSettings onClose={() => setShowShortcutSettings(false)} />
      )}

      {/* Tunnel Manager */}
      {showTunnelManager && (
        <TunnelManager onClose={() => setShowTunnelManager(false)} connections={connections} />
      )}

      {/* 右上角传输进度浮窗 */}
      {activeTransfers.length > 0 && (
        transferPanelVisible ? (
          <div className="fixed top-12 right-4 w-72 bg-surface-light border border-surface-border rounded-lg shadow-xl z-50 overflow-hidden">
            <div className="flex items-center justify-between px-3 py-1.5 border-b border-surface-border bg-surface">
              <span className="text-xs text-gray-400">文件传输 ({activeTransfers.length})</span>
              <button onClick={() => setTransferPanelVisible(false)} className="text-gray-500 hover:text-white text-xs">—</button>
            </div>
            <div className="max-h-48 overflow-y-auto">
              {activeTransfers.map((t, i) => {
                const pct = t.total > 0 ? Math.round((t.bytes / t.total) * 100) : 0;
                const done = t.bytes >= t.total && t.total > 0;
                const sizeStr = t.total >= 1048576
                  ? `${(t.bytes / 1048576).toFixed(1)}/${(t.total / 1048576).toFixed(1)}M`
                  : `${(t.bytes / 1024).toFixed(0)}/${(t.total / 1024).toFixed(0)}K`;
                // 速率格式化
                const spd = t.speed >= 1048576
                  ? `${(t.speed / 1048576).toFixed(1)} MB/s`
                  : t.speed >= 1024
                    ? `${(t.speed / 1024).toFixed(0)} KB/s`
                    : `${Math.max(0, Math.round(t.speed))} B/s`;
                return (
                  <div key={i} className="px-3 py-1.5 border-b border-surface-border/30 last:border-b-0">
                    <div className="flex items-center gap-1.5 text-[11px] mb-1">
                      <span className={t.direction === 'download' ? 'text-accent-cyan' : 'text-accent-green'}>
                        {t.direction === 'download' ? '↓' : '↑'}
                      </span>
                      <span className="text-gray-300 truncate flex-1 min-w-0">{t.filename}</span>
                      {!done && (
                        <button
                          onClick={() => {
                            const cancelKey = `${t.direction}-${t.filename}`;
                            invoke('cancel_transfer', { transferKey: cancelKey }).catch(() => {});
                            setGlobalTransfers(prev => { const n = { ...prev }; delete n[cancelKey]; return n; });
                          }}
                          className="text-gray-500 hover:text-accent-red flex-shrink-0 ml-1"
                          title="取消传输"
                        ><IconClose size={11} /></button>
                      )}
                    </div>
                    {/* 目标目录（拖拽上传时显示） */}
                    {t.target && (
                      <div className="text-[10px] text-gray-500 truncate mb-0.5" title={t.target}>→ {t.target}</div>
                    )}
                    <div className="h-1.5 bg-surface rounded-full overflow-hidden">
                      <div
                        className={`h-full rounded-full transition-all duration-300 ${done ? 'bg-accent-green' : t.direction === 'download' ? 'bg-accent-cyan' : 'bg-accent-green'}`}
                        style={{ width: `${pct}%` }}
                      />
                    </div>
                    {/* 大小 / 速率 / 百分比 */}
                    <div className="flex items-center justify-between text-[10px] text-gray-500 mt-0.5">
                      <span>{done ? '完成' : sizeStr}</span>
                      <span>{done ? '' : `${spd} · ${pct}%`}</span>
                    </div>
                  </div>
                );
              })}
            </div>
          </div>
        ) : (
          <button
            onClick={() => setTransferPanelVisible(true)}
            className="fixed top-12 right-4 z-50 px-3 py-1.5 bg-surface-light border border-surface-border rounded-lg shadow-lg text-xs text-accent-cyan hover:bg-surface-lighter"
          >
            ↑↓ 传输中 ({activeTransfers.length})
          </button>
        )
      )}

      {/* 日志面板 */}
      {showLogPanel && (
        <div className="fixed bottom-0 right-0 w-[600px] h-[300px] bg-surface-light border border-surface-border rounded-tl-lg shadow-xl z-50 flex flex-col">
          <div className="flex items-center justify-between px-3 py-1 border-b border-surface-border">
            <span className="text-xs text-gray-400">调试日志</span>
            <div className="flex gap-2">
              <button onClick={() => navigator.clipboard.writeText(getLogText())} className="text-[10px] text-gray-500 hover:text-white">复制</button>
              <button onClick={() => setShowLogPanel(false)} className="text-gray-500 hover:text-white text-sm">×</button>
            </div>
          </div>
          <pre className="flex-1 overflow-auto p-2 text-[10px] text-gray-400 font-mono whitespace-pre-wrap">{getLogText()}</pre>
        </div>
      )}
    </div>
  );
}

export default App;
