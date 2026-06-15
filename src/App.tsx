import { useEffect, useState, useRef, useCallback } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { save, open } from '@tauri-apps/plugin-dialog';
import { TerminalPane } from './components/Terminal/TerminalPane';
import { SplitContainer } from './components/Terminal/SplitContainer';
import { ConnectionDialog } from './components/ConnectionDialog';
import { SystemInfoPanel } from './components/Sidebar/SystemInfoPanel';
import { FileBrowser } from './components/FileManager/FileBrowser';
import { ProcessTable } from './components/ProcessManager/ProcessTable';
import { NewTabSelector } from './components/NewTabSelector';
import { SshKeyManager } from './components/SshKeyManager';
import type { Tab, ConnectionStore, AuthMethod, SplitNode } from './types';

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
  const [showHistory, setShowHistory] = useState(false);
  const [showSpeed, setShowSpeed] = useState(false);
  const [txSpeed, setTxSpeed] = useState(0);
  const [rxSpeed, setRxSpeed] = useState(0);
  const inputRef = useRef<HTMLInputElement>(null);
  const txBytesRef = useRef(0);
  const rxBytesRef = useRef(0);

  // Track terminal data throughput
  useEffect(() => {
    const unlisten = listen<{ id: string; data: number[] }>('terminal-output', (event) => {
      if (terminalId && event.payload.id === terminalId) {
        rxBytesRef.current += event.payload.data.length;
      }
    });
    const interval = setInterval(() => {
      setRxSpeed(rxBytesRef.current);
      setTxSpeed(txBytesRef.current);
      rxBytesRef.current = 0;
      txBytesRef.current = 0;
    }, 1000);
    return () => { unlisten.then(fn => fn()); clearInterval(interval); };
  }, [terminalId]);

  function sendCommand(cmd: string) {
    if (!cmd || !terminalId) return;
    invoke('terminal_write', { id: terminalId, data: Array.from(new TextEncoder().encode(cmd + '\n')) });
    txBytesRef.current += cmd.length + 1;
    // Add to history (dedupe, max 50)
    const updated = [cmd, ...history.filter(h => h !== cmd)].slice(0, 50);
    setHistory(updated);
    localStorage.setItem('guishell_cmd_history', JSON.stringify(updated));
  }

  function formatSpeed(bytes: number): string {
    if (bytes < 1024) return `${bytes}B/s`;
    if (bytes < 1048576) return `${(bytes / 1024).toFixed(1)}K/s`;
    return `${(bytes / 1048576).toFixed(1)}M/s`;
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
          }
          if (e.key === 'Escape') {
            setShowHistory(false);
          }
        }}
      />
      <button
        onClick={() => { setShowHistory(!showHistory); setShowSpeed(false); }}
        className={`text-xs px-2 py-0.5 border border-surface-border rounded flex-shrink-0 ${showHistory ? 'text-accent-cyan bg-accent-cyan/10' : 'text-gray-400 hover:text-white'}`}
      >
        历史
      </button>
      <button
        onClick={() => { setShowSpeed(!showSpeed); setShowHistory(false); }}
        className={`text-xs px-2 py-0.5 border border-surface-border rounded flex-shrink-0 ${showSpeed ? 'text-accent-cyan bg-accent-cyan/10' : 'text-gray-400 hover:text-white'}`}
      >
        速度
      </button>

      {/* History popup */}
      {showHistory && (
        <>
          <div className="fixed inset-0 z-30" onClick={() => setShowHistory(false)} />
          <div className="absolute bottom-9 right-16 z-40 bg-surface-light border border-surface-border rounded shadow-lg w-80 max-h-64 overflow-y-auto">
            <div className="flex items-center justify-between px-3 py-1.5 border-b border-surface-border">
              <span className="text-xs text-gray-400">命令历史 ({history.length})</span>
              {history.length > 0 && (
                <button
                  onClick={() => { setHistory([]); localStorage.removeItem('guishell_cmd_history'); }}
                  className="text-[10px] text-gray-500 hover:text-accent-red"
                >清空</button>
              )}
            </div>
            {history.length === 0 ? (
              <div className="px-3 py-4 text-xs text-gray-500 text-center">暂无历史记录</div>
            ) : (
              history.map((cmd, i) => (
                <button
                  key={i}
                  onClick={() => {
                    sendCommand(cmd);
                    if (inputRef.current) inputRef.current.value = '';
                    setShowHistory(false);
                  }}
                  className="w-full text-left px-3 py-1.5 text-xs text-gray-300 hover:bg-surface-lighter truncate font-mono border-b border-surface-border/30 last:border-b-0"
                >
                  {cmd}
                </button>
              ))
            )}
          </div>
        </>
      )}

      {/* Speed popup */}
      {showSpeed && (
        <>
          <div className="fixed inset-0 z-30" onClick={() => setShowSpeed(false)} />
          <div className="absolute bottom-9 right-2 z-40 bg-surface-light border border-surface-border rounded shadow-lg w-48 p-3">
            <div className="text-xs text-gray-400 mb-2">终端传输速率</div>
            <div className="flex justify-between text-xs mb-1">
              <span className="text-gray-500">↑ 发送</span>
              <span className="text-accent-green font-mono">{formatSpeed(txSpeed)}</span>
            </div>
            <div className="flex justify-between text-xs">
              <span className="text-gray-500">↓ 接收</span>
              <span className="text-accent-cyan font-mono">{formatSpeed(rxSpeed)}</span>
            </div>
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
                const id = await invoke<string>('ssh_connect', {
                  host: s.sshParams.host,
                  port: s.sshParams.port,
                  user: s.sshParams.user,
                  password: pw || null,
                  authMethod: s.sshParams.authMethod,
                  keyPath: s.sshParams.keyPath,
                  label: s.label,
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

  // Listen for terminal-closed events
  useEffect(() => {
    const unlisten = listen<{ id: string }>('terminal-closed', (event) => {
      setTabs((prev) => prev.filter((t) => t.id !== event.payload.id));
      setActiveTabId((prev) =>
        prev === event.payload.id ? null : prev
      );
    });
    return () => {
      unlisten.then((fn) => fn());
    };
  }, []);

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
    // Start monitor in background
    invoke('start_monitor', {
      sessionId,
      host,
      port,
      user,
      password: password || null,
      authMethod,
      keyPath: keyPath || null,
    }).catch((e) => console.error('Monitor start failed:', e));

    // Start SFTP session — await so file browser can use it
    try {
      await invoke('start_sftp_session', {
        sessionId,
        host,
        port,
        user,
        password: password || null,
        authMethod,
        keyPath: keyPath || null,
      });
    } catch (e) {
      console.error('SFTP session start failed:', e);
    }
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
        },
      });
      await loadConnections();

      const resolvedAuthMethod = params.authMethod === 'keyring' ? 'password' : params.authMethod;

      // Connect
      const id = await invoke<string>('ssh_connect', {
        host: params.host,
        port: params.port,
        user: params.user,
        password: params.password || null,
        authMethod: resolvedAuthMethod,
        keyPath: params.keyPath || null,
        label: params.label,
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
    hostConfig: { host: string; port: number; user: string; auth: AuthMethod; key_path: string; label: string },
    password: string | null,
  ) {
    setError(null);
    const resolvedAuthMethod = hostConfig.auth === 'keyring' ? 'password' : hostConfig.auth;
    try {
      const id = await invoke<string>('ssh_connect', {
        host: hostConfig.host,
        port: hostConfig.port,
        user: hostConfig.user,
        password,
        authMethod: resolvedAuthMethod,
        keyPath: hostConfig.key_path || null,
        label: hostConfig.label,
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
    const tab = tabs.find((t) => t.id === id);
    if (tab && tab.type !== 'process') {
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
        const newId = await invoke<string>('ssh_connect', {
          host: tab.sshParams.host,
          port: tab.sshParams.port,
          user: tab.sshParams.user,
          password: tab.sshParams.password,
          authMethod: tab.sshParams.authMethod,
          keyPath: tab.sshParams.keyPath,
          label: tab.label,
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
        newTerminalId = await invoke<string>('ssh_connect', {
          host: tab.sshParams.host,
          port: tab.sshParams.port,
          user: tab.sshParams.user,
          password: tab.sshParams.password,
          authMethod: tab.sshParams.authMethod,
          keyPath: tab.sshParams.keyPath,
          label: tab.label,
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
  const [sidebarConnectionsOpen, setSidebarConnectionsOpen] = useState(true);
  const [fileBrowserOpen, setFileBrowserOpen] = useState(true);
  const [tabContextMenu, setTabContextMenu] = useState<{ x: number; y: number; tabId: string } | null>(null);
  const [renameTab, setRenameTab] = useState<{ tabId: string; name: string } | null>(null);
  const [connContextMenu, setConnContextMenu] = useState<{ x: number; y: number; groupId: string; hostId: string } | null>(null);
  const [editingConn, setEditingConn] = useState<{ groupId: string; hostId: string } | null>(null);
  const [sidebarWidth, setSidebarWidth] = useState(240);
  const sidebarDragging = useRef(false);
  const [fileBrowserHeight, setFileBrowserHeight] = useState(256);
  const fbDragging = useRef(false);

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
              <span className="text-[10px]">{sidebarConnectionsOpen ? '▼' : '▶'}</span>
              <span>连接管理</span>
            </button>
            <div className="flex items-center gap-1">
              <button
                onClick={handleImportConfig}
                className="text-gray-500 hover:text-accent-cyan text-[10px] leading-none px-1"
                title="导入配置"
              >{'↓'}</button>
              <button
                onClick={handleExportConfig}
                className="text-gray-500 hover:text-accent-cyan text-[10px] leading-none px-1"
                title="导出配置"
              >{'↑'}</button>
              <button
                onClick={() => setShowSshKeyManager(true)}
                className="text-gray-500 hover:text-accent-cyan text-[10px] leading-none px-1"
                title="SSH 密钥管理"
              >{'⚷'}</button>
              <button
                onClick={() => setShowDialog(true)}
                className="text-accent-cyan hover:text-white text-base leading-none"
                title="新建连接"
              >
                +
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
            +
          </button>
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
                  />
                </div>
              ) : (
                <TerminalPane
                  key={tab.id}
                  terminalId={tab.id}
                  isActive={tab.id === activeTabId}
                />
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
            <FileBrowser sessionId={activeSshSessionId} activeTerminalId={activeTabId} sshUser={activeTab?.sshParams?.user} />
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
              config: { label: params.label, host: params.host, port: params.port, user: params.user, auth: params.authMethod, key_path: params.keyPath, charset: 'UTF-8' },
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
    </div>
  );
}

export default App;
