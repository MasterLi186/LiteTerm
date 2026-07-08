import { useEffect, useState, useCallback, useRef } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import type { FileEntry } from '../../types';
import { log } from '../../utils/logger';
import { IconSearch, IconRefresh, IconFolderUp, IconFolder, IconFile } from '../Icons';

// --- Utilities ---

function formatSize(bytes: number): string {
  if (bytes < 1024) return `${bytes}B`;
  if (bytes < 1048576) return `${(bytes / 1024).toFixed(1)}K`;
  if (bytes < 1073741824) return `${(bytes / 1048576).toFixed(1)}M`;
  return `${(bytes / 1073741824).toFixed(1)}G`;
}

function formatTime(epoch: number): string {
  if (!epoch) return '';
  const d = new Date(epoch * 1000);
  const month = String(d.getMonth() + 1).padStart(2, '0');
  const day = String(d.getDate()).padStart(2, '0');
  const hour = String(d.getHours()).padStart(2, '0');
  const min = String(d.getMinutes()).padStart(2, '0');
  return `${month}-${day} ${hour}:${min}`;
}

function getExtType(name: string): string {
  const dot = name.lastIndexOf('.');
  if (dot === -1) return '文件';
  return name.slice(dot + 1).toUpperCase();
}

function joinPath(base: string, name: string): string {
  if (name === '..') {
    const parts = base.replace(/\/+$/, '').split('/');
    parts.pop();
    return parts.length === 0 ? '/' : parts.join('/');
  }
  if (base === '/') return `/${name}`;
  return `${base.replace(/\/+$/, '')}/${name}`;
}

function baseName(path: string): string {
  const parts = path.replace(/\/+$/, '').split('/');
  return parts[parts.length - 1] || path;
}

// --- Context Menu ---

interface ContextMenuItem {
  label: string;
  onClick: () => void;
  disabled?: boolean;
  separator?: boolean;
}

function ContextMenu({ x, y, onClose, items }: {
  x: number;
  y: number;
  onClose: () => void;
  items: ContextMenuItem[];
}) {
  useEffect(() => {
    const handleClick = () => onClose();
    const handleKeyDown = (e: KeyboardEvent) => {
      if (e.key === 'Escape') onClose();
    };
    document.addEventListener('click', handleClick);
    document.addEventListener('keydown', handleKeyDown);
    return () => {
      document.removeEventListener('click', handleClick);
      document.removeEventListener('keydown', handleKeyDown);
    };
  }, [onClose]);

  return (
    <div
      className="fixed z-50 bg-surface-light border border-surface-border rounded shadow-lg py-1 min-w-[160px]"
      style={{ left: x, top: y }}
    >
      {items.map((item, i) =>
        item.separator ? (
          <div key={i} className="border-t border-surface-border my-1" />
        ) : (
          <button
            key={i}
            onClick={(e) => {
              e.stopPropagation();
              item.onClick();
              onClose();
            }}
            disabled={item.disabled}
            className="w-full text-left px-3 py-1.5 text-sm hover:bg-surface-lighter disabled:opacity-40 text-gray-200"
          >
            {item.label}
          </button>
        )
      )}
    </div>
  );
}

// --- Rename Dialog ---

function RenameDialog({ name, onConfirm, onCancel }: {
  name: string;
  onConfirm: (newName: string) => void;
  onCancel: () => void;
}) {
  const [value, setValue] = useState(name);
  const inputRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    inputRef.current?.select();
  }, []);

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/40" onClick={onCancel}>
      <div className="bg-surface-light border border-surface-border rounded-lg p-4 min-w-[300px] shadow-xl" onClick={(e) => e.stopPropagation()}>
        <div className="text-sm text-gray-200 mb-3">重命名</div>
        <input
          ref={inputRef}
          value={value}
          onChange={(e) => setValue(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === 'Enter' && value.trim()) onConfirm(value.trim());
            if (e.key === 'Escape') onCancel();
          }}
          className="w-full bg-surface border border-surface-border rounded px-2 py-1 text-sm text-gray-200 outline-none focus:border-accent-cyan"
        />
        <div className="flex justify-end gap-2 mt-3">
          <button onClick={onCancel} className="px-3 py-1 text-xs text-gray-400 hover:text-white">取消</button>
          <button
            onClick={() => value.trim() && onConfirm(value.trim())}
            className="px-3 py-1 text-xs bg-accent-cyan/20 text-accent-cyan rounded hover:bg-accent-cyan/30"
          >确定</button>
        </div>
      </div>
    </div>
  );
}

// --- Sorting ---
type SortKey = 'name' | 'size' | 'mtime';
type SortDir = 'asc' | 'desc';

function sortFiles(files: FileEntry[], key: SortKey, dir: SortDir): FileEntry[] {
  return [...files].sort((a, b) => {
    if (a.is_dir !== b.is_dir) return a.is_dir ? -1 : 1;
    const d = dir === 'asc' ? 1 : -1;
    switch (key) {
      case 'name': return a.name.localeCompare(b.name) * d;
      case 'size': return (a.size - b.size) * d;
      case 'mtime': return (a.mtime - b.mtime) * d;
      default: return 0;
    }
  });
}

// --- Transfer Queue Types ---

interface TransferItem {
  id: string;
  filename: string;
  direction: 'download' | 'upload';
  bytesTransferred: number;
  totalBytes: number;
  status: 'active' | 'done' | 'error';
  error?: string;
}

// --- FilePane: reusable for both local and remote ---

interface FilePaneProps {
  side: 'local' | 'remote';
  title: string;
  path: string;
  files: FileEntry[];
  error: string | null;
  loading: boolean;
  showHidden: boolean;
  selectedFile: string | null;
  onPathChange: (path: string) => void;
  onRefresh: () => void;
  onSelectFile: (name: string | null) => void;
  onDoubleClick: (entry: FileEntry) => void;
  onContextMenu: (e: React.MouseEvent, entry: FileEntry) => void;
}

function FilePane({
  side, title, path, files, error, loading, showHidden,
  selectedFile, onPathChange, onRefresh, onSelectFile, onDoubleClick, onContextMenu,
}: FilePaneProps) {
  const [pathInput, setPathInput] = useState(path);
  const [sortKey, setSortKey] = useState<SortKey>('name');
  const [sortDir, setSortDir] = useState<SortDir>('asc');
  const [searchVisible, setSearchVisible] = useState(false);
  const [searchQuery, setSearchQuery] = useState('');
  const searchInputRef = useRef<HTMLInputElement>(null);
  const paneRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    setPathInput(path);
  }, [path]);

  useEffect(() => {
    if (searchVisible && searchInputRef.current) {
      searchInputRef.current.focus();
    }
  }, [searchVisible]);

  useEffect(() => {
    const el = paneRef.current;
    if (!el) return;
    const handleKeyDown = (e: KeyboardEvent) => {
      if (e.ctrlKey && e.key === 'f') {
        e.preventDefault();
        setSearchVisible(prev => !prev);
      }
      if (e.key === 'Escape' && searchVisible) {
        setSearchVisible(false);
        setSearchQuery('');
      }
    };
    el.addEventListener('keydown', handleKeyDown);
    return () => el.removeEventListener('keydown', handleKeyDown);
  }, [searchVisible]);

  function handleSort(key: SortKey) {
    if (sortKey === key) {
      setSortDir(sortDir === 'asc' ? 'desc' : 'asc');
    } else {
      setSortKey(key);
      setSortDir('asc');
    }
  }

  function handleGoUp() {
    onPathChange(joinPath(path, '..'));
  }

  function handlePathSubmit() {
    onPathChange(pathInput);
  }

  const hiddenFiltered = showHidden ? files : files.filter(f => !f.name.startsWith('.'));
  const visibleFiles = searchQuery
    ? hiddenFiltered.filter(f => f.name.toLowerCase().includes(searchQuery.toLowerCase()))
    : hiddenFiltered;
  const sorted = sortFiles(visibleFiles, sortKey, sortDir);

  const sortIndicator = (key: SortKey) => {
    if (sortKey !== key) return '';
    return sortDir === 'asc' ? ' ↑' : ' ↓';
  };

  return (
    <div className="flex-1 flex flex-col min-w-0 min-h-0" ref={paneRef} tabIndex={-1}>
      {/* Pane header */}
      <div className="flex items-center gap-1 px-2 py-1 border-b border-surface-border bg-surface-light flex-shrink-0">
        <span className={`text-[10px] font-semibold ${side === 'local' ? 'text-gray-400' : 'text-accent-green'}`}>
          {title}
        </span>
        <div className="flex-1" />
        <button
          onClick={() => { setSearchVisible(prev => !prev); if (searchVisible) setSearchQuery(''); }}
          className={`text-[11px] px-1 rounded ${searchVisible ? 'text-accent-cyan' : 'text-gray-500 hover:text-gray-300'}`}
          title="搜索 (Ctrl+F)"
        ><IconSearch size={12} /></button>
      </div>
      {/* Search bar */}
      {searchVisible && (
        <div className="flex items-center gap-1 px-1 py-0.5 border-b border-surface-border bg-surface flex-shrink-0">
          <input
            ref={searchInputRef}
            type="text"
            value={searchQuery}
            onChange={(e) => setSearchQuery(e.target.value)}
            onKeyDown={(e) => { if (e.key === 'Escape') { setSearchVisible(false); setSearchQuery(''); } }}
            placeholder="搜索文件名..."
            className="flex-1 bg-surface-light border border-surface-border rounded px-1.5 py-0.5 text-[11px] text-gray-300 outline-none focus:border-accent-cyan min-w-0"
          />
          {searchQuery && (
            <span className="text-[10px] text-gray-500">{visibleFiles.length} 项</span>
          )}
          <button
            onClick={() => { setSearchVisible(false); setSearchQuery(''); }}
            className="text-[11px] text-gray-500 hover:text-white px-0.5"
          >×</button>
        </div>
      )}
      {/* Path bar */}
      <div className="flex items-center gap-1 px-1 py-0.5 border-b border-surface-border flex-shrink-0">
        <input
          type="text"
          value={pathInput}
          onChange={(e) => setPathInput(e.target.value)}
          onKeyDown={(e) => { if (e.key === 'Enter') handlePathSubmit(); }}
          className="flex-1 bg-surface border border-surface-border rounded px-1.5 py-0.5 text-[11px] text-gray-300 outline-none focus:border-accent-cyan min-w-0"
        />
        <button onClick={onRefresh} className="text-gray-400 hover:text-white px-0.5" title="刷新"><IconRefresh size={12} /></button>
        <button onClick={handleGoUp} className="text-gray-400 hover:text-white px-0.5" title="上级目录"><IconFolderUp size={12} /></button>
      </div>
      {/* Error */}
      {error && (
        <div className="px-2 py-0.5 text-[10px] text-accent-red bg-accent-red/5 truncate flex-shrink-0">{error}</div>
      )}
      {/* Column headers */}
      <div className="flex text-[10px] text-gray-500 border-b border-surface-border bg-surface-light px-2 py-0.5 flex-shrink-0">
        <span className="flex-1 cursor-pointer hover:text-gray-300" onClick={() => handleSort('name')}>
          文件名{sortIndicator('name')}
        </span>
        <span className="w-14 text-right cursor-pointer hover:text-gray-300" onClick={() => handleSort('size')}>
          大小{sortIndicator('size')}
        </span>
        <span className="w-12 text-gray-500">类型</span>
        <span className="w-20 cursor-pointer hover:text-gray-300" onClick={() => handleSort('mtime')}>
          修改时间{sortIndicator('mtime')}
        </span>
      </div>
      {/* ".." row */}
      <div
        className="flex text-[11px] px-2 py-0.5 hover:bg-surface-lighter cursor-pointer text-gray-400 flex-shrink-0"
        onDoubleClick={handleGoUp}
      >
        <span className="flex-1">..</span>
      </div>
      {/* File list */}
      <div className="flex-1 overflow-y-auto">
        {loading && files.length === 0 && (
          <div className="px-2 py-2 text-[10px] text-gray-500 text-center">加载中...</div>
        )}
        {sorted.map((f) => (
          <div
            key={f.name}
            className={`flex text-[11px] px-2 py-0.5 cursor-pointer ${
              selectedFile === f.name ? 'bg-accent-cyan/10 text-accent-cyan' : 'hover:bg-surface-lighter text-gray-200'
            }`}
            onClick={() => onSelectFile(f.name)}
            onDoubleClick={() => onDoubleClick(f)}
            onContextMenu={(e) => {
              e.preventDefault();
              e.stopPropagation();
              onSelectFile(f.name);
              onContextMenu(e, f);
            }}
          >
            <span className="flex-1 truncate">
              <span className="inline-flex mr-1 align-text-bottom">{f.is_dir ? <IconFolder size={12} className="text-accent-yellow" /> : <IconFile size={12} className="text-gray-500" />}</span>{f.name}
            </span>
            <span className="w-14 text-right text-gray-500">{f.is_dir ? '' : formatSize(f.size)}</span>
            <span className="w-12 text-gray-500">{f.is_dir ? '文件夹' : getExtType(f.name)}</span>
            <span className="w-20 text-gray-500">{formatTime(f.mtime)}</span>
          </div>
        ))}
      </div>
    </div>
  );
}

// 按 sessionId+path 缓存远端文件列表,切换标签时零延迟
const remoteFileCache = new Map<string, FileEntry[]>();

// --- 主组件 ---

interface Props {
  sessionId: string | null;
  activeTerminalId?: string | null;
  sshUser?: string;
  sftpReady?: number;
  // 当前远程路径变化时回调通知父组件（用于拖拽上传的 fallback 目录）
  onRemotePathChange?: (path: string) => void;
}

export function FileBrowser({ sessionId, activeTerminalId, sshUser, sftpReady, onRemotePathChange }: Props) {
  const [activeTab, setActiveTab] = useState<'file' | 'cmd'>('file');
  const [showHidden, setShowHidden] = useState(false);

  // Local pane state
  const [localPath, setLocalPath] = useState('~/Downloads');
  const [localFiles, setLocalFiles] = useState<FileEntry[]>([]);
  const [localError, setLocalError] = useState<string | null>(null);
  const [localLoading, setLocalLoading] = useState(false);
  const [localSelected, setLocalSelected] = useState<string | null>(null);

  // Remote pane state — per-session path cache
  const sessionPathsRef = useRef<Record<string, string>>({});
  const getSessionPath = (sid: string | null) => {
    if (!sid) return '/home';
    return sessionPathsRef.current[sid] || (sshUser ? (sshUser === 'root' ? '/root' : `/home/${sshUser}`) : '/home');
  };
  const [remotePath, setRemotePathRaw] = useState(() => getSessionPath(sessionId));
  const setRemotePath = (path: string) => {
    setRemotePathRaw(path);
    if (sessionId) sessionPathsRef.current[sessionId] = path;
    // 通知父组件当前远程路径变化
    if (onRemotePathChange) onRemotePathChange(path);
  };
  const [remoteFiles, setRemoteFiles] = useState<FileEntry[]>([]);
  const [remoteError, setRemoteError] = useState<string | null>(null);
  const [remoteLoading, setRemoteLoading] = useState(false);
  const [remoteSelected, setRemoteSelected] = useState<string | null>(null);

  // Context menu
  const [contextMenu, setContextMenu] = useState<{
    x: number; y: number; side: 'local' | 'remote'; entry: FileEntry;
  } | null>(null);

  // Rename dialog
  const [renameDialog, setRenameDialog] = useState<{
    side: 'local' | 'remote'; entry: FileEntry;
  } | null>(null);

  // Transfer queue
  const [transfers, setTransfers] = useState<TransferItem[]>([]);

  // --- Load functions ---

  const loadLocalFiles = useCallback(async (path: string) => {
    log('FileBrowser', `loadLocalFiles: ${path}`);
    setLocalLoading(true);
    setLocalError(null);
    try {
      const entries = await invoke<FileEntry[]>('list_local_dir', { path });
      log('FileBrowser', `loadLocalFiles OK: ${entries.length} entries`);
      setLocalFiles(entries);
    } catch (e) {
      log('FileBrowser', `loadLocalFiles ERROR: ${e}`);
      setLocalError(`${e}`);
      setLocalFiles([]);
    }
    setLocalLoading(false);
  }, []);

  const remoteFilesRef = useRef<FileEntry[]>([]);
  const loadingSessionRef = useRef<string | null>(null);
  const loadRemoteFiles = useCallback(async (path: string) => {
    if (!sessionId) return;
    // 防重入：仅阻止同一 session 的并发请求，不同 session 互不影响
    if (loadingSessionRef.current === sessionId) return;
    loadingSessionRef.current = sessionId;
    const thisSession = sessionId;
    const cacheKey = `${thisSession}:${path}`;
    log('FileBrowser', `loadRemoteFiles: session=${sessionId}, path=${path}`);

    // 先从缓存恢复(切换标签时零延迟)
    const cached = remoteFileCache.get(cacheKey);
    if (cached) {
      log('FileBrowser', `缓存命中: ${cacheKey}, ${cached.length} entries`);
      setRemoteFiles(cached);
      remoteFilesRef.current = cached;
    }

    if (!cached && remoteFilesRef.current.length === 0) setRemoteLoading(true);
    setRemoteError(null);

    for (let attempt = 0; attempt < 3; attempt++) {
      if (loadingSessionRef.current !== thisSession) return;
      try {
        const entries = await invoke<FileEntry[]>('sftp_list_dir', { sessionId: thisSession, path });
        if (loadingSessionRef.current !== thisSession) return;
        log('FileBrowser', `loadRemoteFiles OK: ${entries.length} entries (attempt ${attempt})`);
        setRemoteFiles(entries);
        remoteFilesRef.current = entries;
        remoteFileCache.delete(cacheKey);
        remoteFileCache.set(cacheKey, entries);
        if (remoteFileCache.size > 200) {
          const oldest = remoteFileCache.keys().next().value;
          if (oldest) remoteFileCache.delete(oldest);
        }
        setRemoteLoading(false);
        loadingSessionRef.current = null;
        return;
      } catch (e) {
        log('FileBrowser', `loadRemoteFiles attempt ${attempt} FAILED: ${e}`);
        if (attempt < 2) {
          await new Promise(r => setTimeout(r, 2000 * (attempt + 1)));
        }
      }
    }

    if (loadingSessionRef.current !== thisSession) return;
    if (remoteFilesRef.current.length > 0) {
      setRemoteLoading(false);
      loadingSessionRef.current = null;
      return;
    }

    setRemoteError('无法读取远程目录，请手动输入路径');
    setRemoteFiles([]);
    remoteFilesRef.current = [];
    setRemoteLoading(false);
    loadingSessionRef.current = null;
  }, [sessionId]);

  // session 切换时恢复该 session 的路径缓存，首次则查询远端 home
  const prevSessionIdRef = useRef<string | null>(null);
  useEffect(() => {
    loadingSessionRef.current = null;
    log('FileBrowser', `session 切换: prev=${prevSessionIdRef.current} → new=${sessionId}`);
    if (sessionId && sessionId !== prevSessionIdRef.current) {
      prevSessionIdRef.current = sessionId;
      if (sessionPathsRef.current[sessionId]) {
        const cached = sessionPathsRef.current[sessionId];
        log('FileBrowser', `路径缓存命中: ${sessionId} → ${cached}`);
        setRemotePathRaw(cached);
        if (onRemotePathChange) onRemotePathChange(cached);
      } else {
        const fallback = sshUser ? (sshUser === 'root' ? '/root' : `/home/${sshUser}`) : '/home';
        log('FileBrowser', `首次连接 ${sessionId}, fallback=${fallback}, 等 SFTP 就绪后查询 $HOME`);
        setRemotePathRaw(fallback);
        sessionPathsRef.current[sessionId] = fallback;
        if (onRemotePathChange) onRemotePathChange(fallback);
      }
    }
    if (!sessionId) {
      prevSessionIdRef.current = null;
    }
  }, [sessionId, sshUser]);

  // Initial load
  useEffect(() => {
    loadLocalFiles(localPath);
  }, [localPath]);

  // sftpReady 变化时说明 SFTP session 刚建立成功,此时才查 $HOME + 加载文件
  useEffect(() => {
    if (sessionId && sftpReady) {
      // 首次:查询真实 home 目录
      if (!sessionPathsRef.current[sessionId] || sessionPathsRef.current[sessionId].startsWith('/home')) {
        invoke<string>('sftp_exec', { sessionId, command: 'echo $HOME' })
          .then(home => {
            if (home && home.startsWith('/')) {
              log('FileBrowser', `SFTP 就绪,远端 home (${sessionId}): ${home}`);
              sessionPathsRef.current[sessionId] = home;
              setRemotePath(home);
            }
          })
          .catch(() => {});
      }
      loadRemoteFiles(remotePath);
    } else if (!sessionId) {
      setRemoteFiles([]);
      remoteFilesRef.current = [];
      setRemoteError(null);
    }
  }, [remotePath, sessionId, sftpReady]);

  // Listen for transfer progress events
  useEffect(() => {
    const unlisten = listen<{
      filename: string;
      bytes_transferred: number;
      total_bytes: number;
      direction: string;
    }>('transfer-progress', (event) => {
      const { filename, bytes_transferred, total_bytes, direction } = event.payload;
      setTransfers((prev) => {
        const existing = prev.find(t => t.filename === filename && t.direction === direction);
        if (existing) {
          return prev.map(t =>
            t.filename === filename && t.direction === direction
              ? { ...t, bytesTransferred: bytes_transferred, totalBytes: total_bytes, status: bytes_transferred >= total_bytes ? 'done' as const : 'active' as const }
              : t
          );
        }
        return [...prev, {
          id: `${Date.now()}-${filename}`,
          filename,
          direction: direction as 'download' | 'upload',
          bytesTransferred: bytes_transferred,
          totalBytes: total_bytes,
          status: bytes_transferred >= total_bytes ? 'done' as const : 'active' as const,
        }];
      });
    });
    return () => { unlisten.then(fn => fn()); };
  }, []);

  // Auto-remove completed transfers after 3s
  useEffect(() => {
    const done = transfers.filter(t => t.status === 'done' || t.status === 'error');
    if (done.length === 0) return;
    const timer = setTimeout(() => {
      setTransfers(prev => prev.filter(t => t.status === 'active'));
    }, 3000);
    return () => clearTimeout(timer);
  }, [transfers]);

  // --- Handlers ---

  function handleLocalPathChange(path: string) {
    setLocalPath(path);
    setLocalSelected(null);
  }

  function handleRemotePathChange(path: string) {
    setRemotePath(path);
    setRemoteSelected(null);
  }

  function handleLocalDoubleClick(entry: FileEntry) {
    if (entry.is_dir) {
      handleLocalPathChange(joinPath(localPath, entry.name));
    } else if (sessionId) {
      // Upload file to remote
      handleUpload(entry);
    }
  }

  function handleRemoteDoubleClick(entry: FileEntry) {
    if (entry.is_dir) {
      handleRemotePathChange(joinPath(remotePath, entry.name));
    } else {
      // Download file to local
      handleDownload(entry);
    }
  }

  async function handleDownload(entry: FileEntry) {
    if (!sessionId) return;
    const remoteFilePath = joinPath(remotePath, entry.name);
    const localFilePath = joinPath(localPath, entry.name);
    log('FileBrowser', `handleDownload: remote=${remoteFilePath}, local=${localFilePath}, session=${sessionId}`);
    const transferId = `${Date.now()}-${entry.name}`;
    setTransfers(prev => [...prev, {
      id: transferId,
      filename: entry.name,
      direction: 'download',
      bytesTransferred: 0,
      totalBytes: entry.size,
      status: 'active',
    }]);
    try {
      await invoke('sftp_download', {
        sessionId,
        remotePath: remoteFilePath,
        localPath: localFilePath,
      });
      log('FileBrowser', `handleDownload OK: ${entry.name}`);
      loadLocalFiles(localPath);
    } catch (e) {
      log('FileBrowser', `handleDownload ERROR: ${e}`);
      setTransfers(prev => prev.map(t =>
        t.id === transferId ? { ...t, status: 'error' as const, error: `${e}` } : t
      ));
    }
  }

  async function handleUpload(entry: FileEntry) {
    if (!sessionId) return;
    const localFilePath = joinPath(localPath, entry.name);
    const remoteFilePath = joinPath(remotePath, entry.name);
    log('FileBrowser', `handleUpload: local=${localFilePath}, remote=${remoteFilePath}, session=${sessionId}`);
    const transferId = `${Date.now()}-${entry.name}`;
    setTransfers(prev => [...prev, {
      id: transferId,
      filename: entry.name,
      direction: 'upload',
      bytesTransferred: 0,
      totalBytes: entry.size,
      status: 'active',
    }]);
    try {
      await invoke('sftp_upload', {
        sessionId,
        localPath: localFilePath,
        remotePath: remoteFilePath,
      });
      log('FileBrowser', `handleUpload OK: ${entry.name}`);
      loadRemoteFiles(remotePath);
    } catch (e) {
      log('FileBrowser', `handleUpload ERROR: ${e}`);
      setTransfers(prev => prev.map(t =>
        t.id === transferId ? { ...t, status: 'error' as const, error: `${e}` } : t
      ));
    }
  }

  async function uploadByPath(localAbsPath: string) {
    if (!sessionId) return;
    const filename = localAbsPath.replace(/\\/g, '/').split('/').pop() || 'file';
    const remoteFilePath = joinPath(remotePath, filename);
    log('FileBrowser', `uploadByPath: local=${localAbsPath}, remote=${remoteFilePath}`);
    const transferId = `${Date.now()}-${filename}`;
    setTransfers(prev => [...prev, {
      id: transferId,
      filename,
      direction: 'upload' as const,
      bytesTransferred: 0,
      totalBytes: 0,
      status: 'active' as const,
    }]);
    try {
      await invoke('sftp_upload', {
        sessionId,
        localPath: localAbsPath,
        remotePath: remoteFilePath,
      });
      log('FileBrowser', `uploadByPath OK: ${filename}`);
      loadRemoteFiles(remotePath);
    } catch (e) {
      log('FileBrowser', `uploadByPath ERROR: ${e}`);
      setTransfers(prev => prev.map(t =>
        t.id === transferId ? { ...t, status: 'error' as const, error: `${e}` } : t
      ));
    }
  }

  async function handleDelete(side: 'local' | 'remote', entry: FileEntry) {
    const name = entry.name;
    if (side === 'local') {
      const fullPath = joinPath(localPath, name);
      try {
        await invoke('local_delete', { path: fullPath });
        loadLocalFiles(localPath);
      } catch (e) {
        setLocalError(`删除失败: ${e}`);
      }
    } else {
      if (!sessionId) return;
      const fullPath = joinPath(remotePath, name);
      try {
        await invoke('sftp_delete', { sessionId, path: fullPath, isDir: entry.is_dir });
        loadRemoteFiles(remotePath);
      } catch (e) {
        setRemoteError(`删除失败: ${e}`);
      }
    }
  }

  async function handleRename(side: 'local' | 'remote', entry: FileEntry, newName: string) {
    if (side === 'remote') {
      if (!sessionId) return;
      const oldPath = joinPath(remotePath, entry.name);
      const newPath = joinPath(remotePath, newName);
      try {
        await invoke('sftp_rename', { sessionId, oldPath, newPath });
        loadRemoteFiles(remotePath);
      } catch (e) {
        setRemoteError(`重命名失败: ${e}`);
      }
    } else {
      const oldPath = joinPath(localPath, entry.name);
      const newPath = joinPath(localPath, newName);
      try {
        await invoke('local_rename', { oldPath, newPath });
        loadLocalFiles(localPath);
      } catch (e) {
        setLocalError(`重命名失败: ${e}`);
      }
    }
    setRenameDialog(null);
  }

  function handleContextMenuOpen(e: React.MouseEvent, entry: FileEntry, side: 'local' | 'remote') {
    setContextMenu({ x: e.clientX, y: e.clientY, side, entry });
  }

  function getContextMenuItems(): ContextMenuItem[] {
    if (!contextMenu) return [];
    const { side, entry } = contextMenu;

    if (side === 'remote') {
      return [
        {
          label: `下载到本地 (${baseName(localPath)})`,
          onClick: () => handleDownload(entry),
          disabled: entry.is_dir,
        },
        { label: '', onClick: () => {}, separator: true },
        {
          label: '重命名',
          onClick: () => setRenameDialog({ side: 'remote', entry }),
        },
        {
          label: '删除',
          onClick: () => handleDelete('remote', entry),
        },
      ];
    } else {
      return [
        {
          label: `上传到远程 (${baseName(remotePath)})`,
          onClick: () => handleUpload(entry),
          disabled: !sessionId || entry.is_dir,
        },
        { label: '', onClick: () => {}, separator: true },
        {
          label: '重命名',
          onClick: () => setRenameDialog({ side: 'local', entry }),
        },
        {
          label: '删除',
          onClick: () => handleDelete('local', entry),
        },
      ];
    }
  }

  return (
    <div className="flex flex-col h-full">
      {/* Toolbar */}
      <div className="flex items-center gap-1 px-2 py-1 border-b border-surface-border bg-surface-light flex-shrink-0">
        <button
          onClick={() => setActiveTab('file')}
          className={`px-2 py-0.5 text-xs rounded ${
            activeTab === 'file' ? 'bg-surface-lighter text-gray-200' : 'text-gray-500 hover:text-gray-300'
          }`}
        >
          文件
        </button>
        <button
          onClick={() => setActiveTab('cmd')}
          className={`px-2 py-0.5 text-xs rounded ${
            activeTab === 'cmd' ? 'bg-surface-lighter text-gray-200' : 'text-gray-500 hover:text-gray-300'
          }`}
        >
          命令
        </button>
        <div className="flex-1" />
        <button
          onClick={() => setShowHidden(!showHidden)}
          className={`px-2 py-0.5 text-[10px] rounded mr-1 ${showHidden ? 'bg-accent-cyan/15 text-accent-cyan' : 'text-gray-500 hover:text-gray-300'}`}
          title={showHidden ? '隐藏隐藏文件' : '显示隐藏文件'}
        >
          {showHidden ? '隐藏.' : '显示.'}
        </button>
        {sessionId ? (
          <span className="text-[10px] text-accent-green mr-1">已连接</span>
        ) : (
          <span className="text-[10px] text-gray-500 mr-1">未连接</span>
        )}
      </div>

      {activeTab === 'file' ? (
        <div className="flex-1 flex flex-col min-h-0">
          {/* Dual pane */}
          <div className="flex-1 flex min-h-0">
            {/* Left: Local */}
            <FilePane
              side="local"
              title="本地文件"
              path={localPath}
              files={localFiles}
              error={localError}
              loading={localLoading}
              showHidden={showHidden}
              selectedFile={localSelected}
              onPathChange={handleLocalPathChange}
              onRefresh={() => loadLocalFiles(localPath)}
              onSelectFile={setLocalSelected}
              onDoubleClick={handleLocalDoubleClick}
              onContextMenu={(e, entry) => handleContextMenuOpen(e, entry, 'local')}
            />
            {/* Divider */}
            <div className="w-px bg-surface-border flex-shrink-0" />
            {/* Right: Remote */}
            {sessionId ? (
              <FilePane
                side="remote"
                title="远程文件"
                path={remotePath}
                files={remoteFiles}
                error={remoteError}
                loading={remoteLoading}
                showHidden={showHidden}
                selectedFile={remoteSelected}
                onPathChange={handleRemotePathChange}
                onRefresh={() => loadRemoteFiles(remotePath)}
                onSelectFile={setRemoteSelected}
                onDoubleClick={handleRemoteDoubleClick}
                onContextMenu={(e, entry) => handleContextMenuOpen(e, entry, 'remote')}
              />
            ) : (
              <div className="flex-1 flex items-center justify-center text-xs text-gray-500">
                未连接远程主机 — 连接SSH后显示远程文件
              </div>
            )}
          </div>
          {/* 传输进度统一显示在右上角浮窗，此处不再渲染底部进度条 */}
        </div>
      ) : (
        <CommandSnippets activeTerminalId={activeTerminalId || null} />
      )}

      {/* Context menu */}
      {contextMenu && (
        <ContextMenu
          x={contextMenu.x}
          y={contextMenu.y}
          onClose={() => setContextMenu(null)}
          items={getContextMenuItems()}
        />
      )}

      {/* Rename dialog */}
      {renameDialog && (
        <RenameDialog
          name={renameDialog.entry.name}
          onConfirm={(newName) => handleRename(renameDialog.side, renameDialog.entry, newName)}
          onCancel={() => setRenameDialog(null)}
        />
      )}
    </div>
  );
}

/* ---- Command Snippets Component ---- */

interface Snippet {
  id: string;
  name: string;
  command: string;
}

function CommandSnippets({ activeTerminalId }: { activeTerminalId: string | null }) {
  const [snippets, setSnippets] = useState<Snippet[]>(() => {
    try {
      const saved = localStorage.getItem('guishell_snippets');
      return saved ? JSON.parse(saved) : [
        { id: '1', name: '查看磁盘', command: 'df -h' },
        { id: '2', name: '查看内存', command: 'free -h' },
        { id: '3', name: '查看进程', command: 'top -bn1 | head -20' },
        { id: '4', name: '网络连接', command: 'ss -tlnp' },
        { id: '5', name: '系统日志', command: 'tail -50 /var/log/syslog' },
      ];
    } catch { return []; }
  });
  const [newName, setNewName] = useState('');
  const [newCmd, setNewCmd] = useState('');
  const [showAdd, setShowAdd] = useState(false);

  function save(list: Snippet[]) {
    setSnippets(list);
    localStorage.setItem('guishell_snippets', JSON.stringify(list));
  }

  function runSnippet(cmd: string) {
    if (!activeTerminalId) return;
    invoke('terminal_write', {
      id: activeTerminalId,
      data: Array.from(new TextEncoder().encode(cmd + '\n')),
    });
  }

  function addSnippet() {
    if (!newName.trim() || !newCmd.trim()) return;
    const id = Date.now().toString();
    save([...snippets, { id, name: newName.trim(), command: newCmd.trim() }]);
    setNewName('');
    setNewCmd('');
    setShowAdd(false);
  }

  function deleteSnippet(id: string) {
    save(snippets.filter(s => s.id !== id));
  }

  return (
    <div className="flex-1 flex flex-col min-h-0 overflow-hidden">
      {/* Toolbar */}
      <div className="flex items-center px-2 py-1 border-b border-surface-border flex-shrink-0">
        <span className="text-xs text-gray-400">命令收藏 -- 点击直接发送到终端</span>
        <div className="flex-1" />
        <button
          onClick={() => setShowAdd(!showAdd)}
          className="text-xs text-accent-cyan hover:text-white px-2"
        >
          {showAdd ? '取消' : '+ 添加'}
        </button>
      </div>

      {/* Add form */}
      {showAdd && (
        <div className="flex items-center gap-2 px-2 py-1 border-b border-surface-border bg-surface flex-shrink-0">
          <input
            value={newName}
            onChange={(e) => setNewName(e.target.value)}
            placeholder="名称"
            className="w-24 bg-surface-light border border-surface-border rounded px-2 py-0.5 text-xs text-gray-200 outline-none"
          />
          <input
            value={newCmd}
            onChange={(e) => setNewCmd(e.target.value)}
            placeholder="命令"
            onKeyDown={(e) => e.key === 'Enter' && addSnippet()}
            className="flex-1 bg-surface-light border border-surface-border rounded px-2 py-0.5 text-xs text-gray-200 outline-none"
          />
          <button onClick={addSnippet} className="text-xs text-accent-cyan hover:text-white px-2">保存</button>
        </div>
      )}

      {/* Snippet list */}
      <div className="flex-1 overflow-y-auto">
        {snippets.map((s) => (
          <div
            key={s.id}
            className="flex items-center px-2 py-1 hover:bg-surface-lighter cursor-pointer group"
            onClick={() => runSnippet(s.command)}
          >
            <span className="text-xs text-accent-cyan w-24 flex-shrink-0 truncate">{s.name}</span>
            <span className="text-xs text-gray-400 flex-1 truncate font-mono">{s.command}</span>
            <button
              onClick={(e) => { e.stopPropagation(); deleteSnippet(s.id); }}
              className="text-xs text-gray-600 hover:text-accent-red opacity-0 group-hover:opacity-100 ml-2"
            >
              x
            </button>
          </div>
        ))}
        {snippets.length === 0 && (
          <div className="px-3 py-4 text-xs text-gray-500 text-center">暂无收藏命令，点击"添加"创建</div>
        )}
      </div>
    </div>
  );
}
