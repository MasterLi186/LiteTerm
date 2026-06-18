import { useEffect, useRef, useState } from 'react';
import { Terminal } from '@xterm/xterm';
import { FitAddon } from '@xterm/addon-fit';
import { SearchAddon } from '@xterm/addon-search';
import { WebLinksAddon } from '@xterm/addon-web-links';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { save, open } from '@tauri-apps/plugin-dialog';
import { open as shellOpen } from '@tauri-apps/plugin-shell';
import * as Zmodem from 'zmodem.js';
import '@xterm/xterm/css/xterm.css';
import type { ITheme } from '@xterm/xterm';

// ---- Terminal Themes ----
const TERMINAL_THEMES: Record<string, ITheme> = {
  '暗色默认': {
    background: '#0d1117', foreground: '#e6edf3', cursor: '#00d4ff',
    selectionBackground: '#264f78',
    black: '#484f58', red: '#ff7b72', green: '#3fb950', yellow: '#d29922',
    blue: '#58a6ff', magenta: '#bc8cff', cyan: '#39d353', white: '#b1bac4',
    brightBlack: '#6e7681', brightRed: '#ffa198', brightGreen: '#56d364',
    brightYellow: '#e3b341', brightBlue: '#79c0ff', brightMagenta: '#d2a8ff',
    brightCyan: '#56d364', brightWhite: '#f0f6fc',
  },
  'Monokai': {
    background: '#272822', foreground: '#f8f8f2', cursor: '#f8f8f0',
    selectionBackground: '#49483e',
    black: '#272822', red: '#f92672', green: '#a6e22e', yellow: '#f4bf75',
    blue: '#66d9ef', magenta: '#ae81ff', cyan: '#a1efe4', white: '#f8f8f2',
    brightBlack: '#75715e', brightRed: '#f92672', brightGreen: '#a6e22e',
    brightYellow: '#f4bf75', brightBlue: '#66d9ef', brightMagenta: '#ae81ff',
    brightCyan: '#a1efe4', brightWhite: '#f9f8f5',
  },
  'Solarized Dark': {
    background: '#002b36', foreground: '#839496', cursor: '#93a1a1',
    selectionBackground: '#073642',
    black: '#073642', red: '#dc322f', green: '#859900', yellow: '#b58900',
    blue: '#268bd2', magenta: '#d33682', cyan: '#2aa198', white: '#eee8d5',
    brightBlack: '#586e75', brightRed: '#cb4b16', brightGreen: '#586e75',
    brightYellow: '#657b83', brightBlue: '#839496', brightMagenta: '#6c71c4',
    brightCyan: '#93a1a1', brightWhite: '#fdf6e3',
  },
  'Dracula': {
    background: '#282a36', foreground: '#f8f8f2', cursor: '#f8f8f2',
    selectionBackground: '#44475a',
    black: '#21222c', red: '#ff5555', green: '#50fa7b', yellow: '#f1fa8c',
    blue: '#bd93f9', magenta: '#ff79c6', cyan: '#8be9fd', white: '#f8f8f2',
    brightBlack: '#6272a4', brightRed: '#ff6e6e', brightGreen: '#69ff94',
    brightYellow: '#ffffa5', brightBlue: '#d6acff', brightMagenta: '#ff92df',
    brightCyan: '#a4ffff', brightWhite: '#ffffff',
  },
  'One Dark': {
    background: '#282c34', foreground: '#abb2bf', cursor: '#528bff',
    selectionBackground: '#3e4451',
    black: '#282c34', red: '#e06c75', green: '#98c379', yellow: '#e5c07b',
    blue: '#61afef', magenta: '#c678dd', cyan: '#56b6c2', white: '#abb2bf',
    brightBlack: '#5c6370', brightRed: '#e06c75', brightGreen: '#98c379',
    brightYellow: '#e5c07b', brightBlue: '#61afef', brightMagenta: '#c678dd',
    brightCyan: '#56b6c2', brightWhite: '#ffffff',
  },
  '浅色': {
    background: '#ffffff', foreground: '#383a42', cursor: '#526eff',
    selectionBackground: '#d7d7ff',
    black: '#383a42', red: '#e45649', green: '#50a14f', yellow: '#c18401',
    blue: '#4078f2', magenta: '#a626a4', cyan: '#0184bc', white: '#a0a1a7',
    brightBlack: '#696c77', brightRed: '#e45649', brightGreen: '#50a14f',
    brightYellow: '#c18401', brightBlue: '#4078f2', brightMagenta: '#a626a4',
    brightCyan: '#0184bc', brightWhite: '#ffffff',
  },
};

/** Get the current theme name from localStorage */
function getTerminalThemeName(): string {
  return localStorage.getItem('guishell_terminal_theme') || '暗色默认';
}

/** Get the current theme object */
function getTerminalTheme(): ITheme {
  return TERMINAL_THEMES[getTerminalThemeName()] || TERMINAL_THEMES['暗色默认'];
}

// Global event for theme change notification
const themeChangeListeners = new Set<() => void>();
function notifyThemeChange() {
  themeChangeListeners.forEach(fn => fn());
}

interface ContextMenuItem {
  label: string;
  onClick: () => void;
  disabled?: boolean;
  separator?: boolean;
  submenu?: ContextMenuItem[];
}

function ContextMenu({ x, y, onClose, items }: {
  x: number;
  y: number;
  onClose: () => void;
  items: ContextMenuItem[];
}) {
  const [hoveredSubmenu, setHoveredSubmenu] = useState<number | null>(null);

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
        ) : item.submenu ? (
          <div
            key={i}
            className="relative"
            onMouseEnter={() => setHoveredSubmenu(i)}
            onMouseLeave={() => setHoveredSubmenu(null)}
          >
            <button
              className="w-full text-left px-3 py-1.5 text-sm hover:bg-surface-lighter text-gray-200 flex items-center justify-between"
            >
              {item.label}
              <span className="text-xs text-gray-500 ml-2">{'▶'}</span>
            </button>
            {hoveredSubmenu === i && (
              <div
                className="absolute left-full top-0 bg-surface-light border border-surface-border rounded shadow-lg py-1 min-w-[140px]"
                style={{ marginLeft: '2px' }}
              >
                {item.submenu.map((sub, j) => (
                  <button
                    key={j}
                    onClick={(e) => {
                      e.stopPropagation();
                      sub.onClick();
                      onClose();
                    }}
                    className="w-full text-left px-3 py-1.5 text-sm hover:bg-surface-lighter text-gray-200"
                  >
                    {sub.label}
                  </button>
                ))}
              </div>
            )}
          </div>
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

/** Handle a detected ZMODEM session (receive or send). */
async function handleZmodemDetection(
  detection: any,
  terminalId: string,
  setZmodemTransfer: React.Dispatch<React.SetStateAction<{
    filename: string;
    bytesReceived: number;
    totalSize: number;
    status: 'receiving' | 'sending' | 'complete';
  } | null>>,
  pendingUploadFiles?: Array<{ name: string; data: Uint8Array }>,
) {
  const session = detection.confirm();
  const role: string = session.type;

  if (role === 'receive') {
    setZmodemTransfer({ filename: '', bytesReceived: 0, totalSize: 0, status: 'receiving' });

    session.on('offer', (offer: any) => {
      const details = offer.get_details();
      const filename: string = details.name;
      const totalSize: number = details.size || 0;

      setZmodemTransfer({ filename, bytesReceived: 0, totalSize, status: 'receiving' });

      // Accept with spool mode — collect all data, then save
      offer.on('input', (payload: number[]) => {
        setZmodemTransfer((prev) =>
          prev ? { ...prev, bytesReceived: prev.bytesReceived + payload.length } : null,
        );
      });

      offer.accept().then(async (payloads: Uint8Array[]) => {
        // Merge all payloads into one array for saving
        const totalLen = payloads.reduce((sum, p) => sum + p.length, 0);
        const merged = new Uint8Array(totalLen);
        let offset = 0;
        for (const p of payloads) {
          merged.set(p, offset);
          offset += p.length;
        }

        // 弹出保存对话框让用户选择保存位置
        try {
          const savePath = await save({
            title: '保存下载文件',
            defaultPath: filename,
            filters: [{ name: '所有文件', extensions: ['*'] }],
          });
          if (savePath) {
            await invoke('save_file', { path: savePath, data: Array.from(merged) });
          }
        } catch (e) {
          console.error('ZMODEM 保存失败:', e);
        }

        setZmodemTransfer({ filename, bytesReceived: totalLen, totalSize, status: 'complete' });
        // 完成后 5 秒自动关闭进度条
        setTimeout(() => setZmodemTransfer(null), 5000);
      });
    });

    session.on('session_end', () => {
      setTimeout(() => setZmodemTransfer(null), 3000);
    });

    // 兜底：30 秒后强制关闭进度条（防止 session_end 没触发）
    setTimeout(() => setZmodemTransfer(null), 30000);

    session.start();
  } else if (role === 'send' && pendingUploadFiles && pendingUploadFiles.length > 0) {
    // Send session — upload files via ZMODEM (triggered by rz)
    for (const file of pendingUploadFiles) {
      setZmodemTransfer({ filename: file.name, bytesReceived: 0, totalSize: file.data.length, status: 'sending' });
      try {
        const xfer = await session.send_offer({ name: file.name, size: file.data.length });
        if (xfer) {
          const CHUNK = 8192;
          for (let i = 0; i < file.data.length; i += CHUNK) {
            const end = Math.min(i + CHUNK, file.data.length);
            await xfer.send(Array.from(file.data.subarray(i, end)));
            setZmodemTransfer(prev => prev ? { ...prev, bytesReceived: end } : null);
          }
          await xfer.end();
        }
      } catch (e) {
        console.error('ZMODEM 上传失败:', e);
      }
    }
    setZmodemTransfer(prev => prev ? { ...prev, status: 'complete' } : null);
    setTimeout(() => setZmodemTransfer(null), 5000);
    session.close();
  } else {
    session.close();
  }
}

interface Props {
  terminalId: string;
  isActive: boolean;
  onSplit?: (direction: 'horizontal' | 'vertical') => void;
  onClosePane?: () => void;
  onFocus?: () => void;
  onOpenRecording?: (filePath: string) => void;
  onRegisterUpload?: (upload: (files: Array<{ name: string; data: Uint8Array }>) => void) => void;
}

export function TerminalPane({ terminalId, isActive, onSplit, onClosePane, onFocus, onOpenRecording, onRegisterUpload }: Props) {
  const wrapperRef = useRef<HTMLDivElement>(null);
  const containerRef = useRef<HTMLDivElement>(null);
  const termRef = useRef<Terminal | null>(null);
  const fitRef = useRef<FitAddon | null>(null);
  const searchAddonRef = useRef<SearchAddon | null>(null);
  const [contextMenu, setContextMenu] = useState<{ x: number; y: number } | null>(null);
  const [logging, setLogging] = useState(false);
  const logBufferRef = useRef<string[]>([]);
  const logFileNameRef = useRef<string>('');
  const [recording, setRecording] = useState(false);
  const isRecordingRef = useRef(false);
  const [searchVisible, setSearchVisible] = useState(false);
  const [searchTerm, setSearchTerm] = useState('');
  const searchInputRef = useRef<HTMLInputElement>(null);
  const [zmodemTransfer, setZmodemTransfer] = useState<{
    filename: string;
    bytesReceived: number;
    totalSize: number;
    status: 'receiving' | 'sending' | 'complete';
  } | null>(null);
  const pendingUploadRef = useRef<Array<{ name: string; data: Uint8Array }>>([]);

  useEffect(() => {
    if (!containerRef.current || !wrapperRef.current || termRef.current) return;

    const term = new Terminal({
      cursorBlink: true,
      rightClickSelectsWord: true,
      fontSize: 14,
      fontFamily: "'DejaVu Sans Mono', 'Liberation Mono', 'Noto Sans Mono', monospace",
      theme: getTerminalTheme(),
    });

    const fitAddon = new FitAddon();
    const searchAddon = new SearchAddon();
    const webLinksAddon = new WebLinksAddon((_event, uri) => {
      shellOpen(uri).catch(() => {});
    });
    term.loadAddon(fitAddon);
    term.loadAddon(searchAddon);
    term.loadAddon(webLinksAddon);
    term.open(containerRef.current);
    termRef.current = term;
    fitRef.current = fitAddon;
    searchAddonRef.current = searchAddon;

    // Listen for theme changes from other panes / context menu
    const onThemeChange = () => {
      if (termRef.current) {
        termRef.current.options.theme = getTerminalTheme();
      }
    };
    themeChangeListeners.add(onThemeChange);

    // Force fit: read size from wrapper, write to container, fit, refresh
    const forceFit = () => {
      const w = wrapperRef.current;
      const c = containerRef.current;
      if (!w || !c || !fitRef.current || !termRef.current) return false;
      const rect = w.getBoundingClientRect();
      if (rect.width < 10 || rect.height < 10) return false;
      c.style.width = `${Math.floor(rect.width)}px`;
      c.style.height = `${Math.floor(rect.height)}px`;
      try {
        fitRef.current.fit();
        termRef.current.refresh(0, termRef.current.rows - 1);
      } catch (_) {}
      return true;
    };

    // Retry initial fit until layout is ready
    let initAttempt = 0;
    const tryInitFit = () => {
      initAttempt++;
      if (!forceFit() && initAttempt < 30) {
        setTimeout(tryInitFit, 100);
      }
    };
    requestAnimationFrame(tryInitFit);

    // Ctrl+Shift+F to toggle search bar
    term.attachCustomKeyEventHandler((e: KeyboardEvent) => {
      if (e.ctrlKey && e.shiftKey && e.key === 'F' && e.type === 'keydown') {
        setSearchVisible(prev => !prev);
        return false; // prevent terminal from receiving this key
      }
      return true;
    });

    // Select-to-copy: auto copy selection to clipboard
    term.onSelectionChange(() => {
      const sel = term.getSelection();
      if (sel) {
        navigator.clipboard.writeText(sel).catch(() => {});
      }
    });

    // Middle-click paste (mouseup only, prevent double-paste)
    const handleMiddleClick = (e: MouseEvent) => {
      if (e.button === 1) {
        e.preventDefault();
        e.stopPropagation();
        navigator.clipboard.readText().then((text) => {
          if (text) {
            const bytes = Array.from(new TextEncoder().encode(text));
            invoke('terminal_write', { id: terminalId, data: bytes });
          }
        }).catch(() => {});
      }
    };
    containerRef.current.addEventListener('mouseup', handleMiddleClick);
    containerRef.current.addEventListener('mousedown', (e: MouseEvent) => {
      if (e.button === 1) e.preventDefault();
    });

    // User input -> Tauri
    term.onData((data) => {
      const bytes = Array.from(new TextEncoder().encode(data));
      invoke('terminal_write', { id: terminalId, data: bytes });
    });

    // Resize -> Tauri
    term.onResize(({ cols, rows }) => {
      invoke('terminal_resize', { id: terminalId, cols, rows });
    });

    // ZMODEM sentry: intercepts ZMODEM sessions from the data stream
    const sentry = new Zmodem.Sentry({
      to_terminal: (octets: number[]) => {
        const bytes = new Uint8Array(octets);
        term.write(bytes);
        if (logBufferRef.current.length > 0 || logFileNameRef.current) {
          const text = new TextDecoder().decode(bytes);
          logBufferRef.current.push(text);
        }
        if (isRecordingRef.current) {
          const text = new TextDecoder().decode(bytes);
          invoke('record_event', { terminalId, data: text }).catch(() => {});
        }
      },
      sender: (octets: number[]) => {
        invoke('terminal_write', { id: terminalId, data: Array.from(octets) });
      },
      on_retract: () => {
        // Detection was retracted (not actually ZMODEM)
      },
      on_detect: (detection: any) => {
        const files = pendingUploadRef.current.length > 0 ? [...pendingUploadRef.current] : undefined;
        pendingUploadRef.current = [];
        handleZmodemDetection(detection, terminalId, setZmodemTransfer, files);
      },
    });

    // 注册上传函数：外部调用时设置 pending files 并发送 rz 命令
    if (onRegisterUpload) {
      onRegisterUpload((files) => {
        pendingUploadRef.current = files;
        const rzCmd = 'rz\n';
        const bytes = Array.from(new TextEncoder().encode(rzCmd));
        invoke('terminal_write', { id: terminalId, data: bytes });
      });
    }

    // Listen for output from Tauri — pass through ZMODEM sentry
    const unlisten = listen<{ id: string; data: number[] }>('terminal-output', (event) => {
      if (event.payload.id === terminalId) {
        const data = new Uint8Array(event.payload.data);
        try {
          sentry.consume(data);
        } catch (e) {
          term.write(data);
          if (logFileNameRef.current) {
            logBufferRef.current.push(new TextDecoder().decode(data));
          }
          if (isRecordingRef.current) {
            invoke('record_event', { terminalId, data: new TextDecoder().decode(data) }).catch(() => {});
          }
        }
      }
    });

    // Resize handler: multi-stage fit to handle window animation
    let resizeTimer: ReturnType<typeof setTimeout> | null = null;
    let resizeTimers: ReturnType<typeof setTimeout>[] = [];
    const doFit = () => {
      // Clear any pending fits
      resizeTimers.forEach(t => clearTimeout(t));
      resizeTimers = [];
      // Fit at multiple delays to catch window animation completion
      for (const delay of [50, 150, 400]) {
        const t = setTimeout(() => {
          requestAnimationFrame(forceFit);
        }, delay);
        resizeTimers.push(t);
      }
    };

    const observer = new ResizeObserver(doFit);
    if (wrapperRef.current) {
      observer.observe(wrapperRef.current);
    }
    window.addEventListener('resize', doFit);

    return () => {
      unlisten.then(fn => fn());
      observer.disconnect();
      window.removeEventListener('resize', doFit);
      resizeTimers.forEach(t => clearTimeout(t));
      themeChangeListeners.delete(onThemeChange);
      term.dispose();
      termRef.current = null;
      fitRef.current = null;
      searchAddonRef.current = null;
    };
  }, [terminalId]);

  // Focus search input when search bar becomes visible
  useEffect(() => {
    if (searchVisible && searchInputRef.current) {
      searchInputRef.current.focus();
    }
  }, [searchVisible]);

  // Refit when tab becomes active
  useEffect(() => {
    if (isActive && fitRef.current && termRef.current) {
      const syncAndFit = () => {
        const w = wrapperRef.current;
        const c = containerRef.current;
        if (w && c && fitRef.current && termRef.current) {
          const { width, height } = w.getBoundingClientRect();
          if (width > 10 && height > 10) {
            c.style.width = `${Math.floor(width)}px`;
            c.style.height = `${Math.floor(height)}px`;
            try {
              fitRef.current.fit();
              termRef.current.refresh(0, termRef.current.rows - 1);
            } catch (_) {}
          }
        }
      };
      requestAnimationFrame(syncAndFit);
      const t = setTimeout(syncAndFit, 200);
      return () => clearTimeout(t);
    }
  }, [isActive]);


  function handleContextMenu(e: React.MouseEvent) {
    e.preventDefault();
    e.stopPropagation();
    setContextMenu({ x: e.clientX, y: e.clientY });
  }

  async function handleCopy() {
    const term = termRef.current;
    if (!term) return;
    const selection = term.getSelection();
    if (selection) {
      try { await navigator.clipboard.writeText(selection); } catch (_) {}
    }
  }

  async function handlePaste() {
    try {
      const text = await navigator.clipboard.readText();
      if (text) {
        const bytes = Array.from(new TextEncoder().encode(text));
        invoke('terminal_write', { id: terminalId, data: bytes });
      }
    } catch (_) {}
  }

  function handleSelectAll() {
    termRef.current?.selectAll();
  }

  function handleClear() {
    termRef.current?.clear();
  }

  async function handleStartLog() {
    const now = new Date();
    const ts = `${now.getFullYear()}${String(now.getMonth()+1).padStart(2,'0')}${String(now.getDate()).padStart(2,'0')}_${String(now.getHours()).padStart(2,'0')}${String(now.getMinutes()).padStart(2,'0')}${String(now.getSeconds()).padStart(2,'0')}`;
    const defaultName = `terminal_${ts}.log`;

    const filePath = await save({
      title: '选择日志保存位置',
      defaultPath: defaultName,
      filters: [{ name: '日志文件', extensions: ['log', 'txt'] }],
    });

    if (!filePath) return;

    logFileNameRef.current = filePath;
    logBufferRef.current = [];
    setLogging(true);
  }

  async function handleStopLog() {
    const content = logBufferRef.current.join('');
    const filename = logFileNameRef.current;
    logFileNameRef.current = '';
    logBufferRef.current = [];
    setLogging(false);

    if (content && filename) {
      const clean = content.replace(/\x1b\[[0-9;]*[a-zA-Z]/g, '').replace(/\x1b\][^\x07]*\x07/g, '');
      const bytes = Array.from(new TextEncoder().encode(clean));
      try {
        await invoke('save_file', { path: filename, data: bytes });
      } catch (_) {}
    }
  }

  async function handleStartRecording() {
    const now = new Date();
    const ts = `${now.getFullYear()}${String(now.getMonth()+1).padStart(2,'0')}${String(now.getDate()).padStart(2,'0')}_${String(now.getHours()).padStart(2,'0')}${String(now.getMinutes()).padStart(2,'0')}${String(now.getSeconds()).padStart(2,'0')}`;
    const defaultName = `terminal_${ts}.cast`;

    const filePath = await save({
      title: '选择录屏保存位置',
      defaultPath: defaultName,
      filters: [{ name: '录屏文件', extensions: ['cast'] }],
    });

    if (!filePath) return;

    const term = termRef.current;
    const cols = term?.cols || 80;
    const rows = term?.rows || 24;

    try {
      await invoke('start_recording', {
        terminalId,
        filePath,
        width: cols,
        height: rows,
      });
      isRecordingRef.current = true;
      setRecording(true);
    } catch (_) {}
  }

  async function handleStopRecording() {
    isRecordingRef.current = false;
    setRecording(false);
    try {
      await invoke('stop_recording', { terminalId });
    } catch (_) {}
  }

  async function handleOpenRecording() {
    const filePath = await open({
      title: '选择录屏文件',
      multiple: false,
      filters: [{ name: '录屏文件', extensions: ['cast'] }],
    });
    if (filePath && onOpenRecording) {
      onOpenRecording(filePath as string);
    }
  }

  function handleChangeTheme(themeName: string) {
    localStorage.setItem('guishell_terminal_theme', themeName);
    notifyThemeChange();
  }

  function handleSearchNext() {
    if (searchAddonRef.current && searchTerm) {
      searchAddonRef.current.findNext(searchTerm);
    }
  }

  function handleSearchPrevious() {
    if (searchAddonRef.current && searchTerm) {
      searchAddonRef.current.findPrevious(searchTerm);
    }
  }

  const currentThemeName = getTerminalThemeName();

  const themeSubmenuItems: ContextMenuItem[] = Object.keys(TERMINAL_THEMES).map(name => ({
    label: name === currentThemeName ? `✓ ${name}` : `   ${name}`,
    onClick: () => handleChangeTheme(name),
  }));

  const contextMenuItems: ContextMenuItem[] = [
    { label: '复制', onClick: handleCopy },
    { label: '粘贴', onClick: handlePaste },
    { label: '全选', onClick: handleSelectAll },
    { label: '清屏', onClick: handleClear },
    { label: '', onClick: () => {}, separator: true },
    { label: '搜索 (Ctrl+Shift+F)', onClick: () => setSearchVisible(true) },
    { label: '', onClick: () => {}, separator: true },
    { label: '终端主题', onClick: () => {}, submenu: themeSubmenuItems },
    { label: '', onClick: () => {}, separator: true },
    logging
      ? { label: '⏹ 停止录制日志', onClick: handleStopLog }
      : { label: '⏺ 开始录制日志', onClick: handleStartLog },
    recording
      ? { label: '⏹ 停止录屏', onClick: handleStopRecording }
      : { label: '⏺ 开始录屏', onClick: handleStartRecording },
    { label: '▶ 回放录屏', onClick: handleOpenRecording },
    { label: '', onClick: () => {}, separator: true },
    { label: '水平分屏', onClick: () => onSplit?.('horizontal') },
    { label: '垂直分屏', onClick: () => onSplit?.('vertical') },
    ...(onClosePane ? [{ label: '关闭面板', onClick: onClosePane }] : []),
  ];

  // When inside a SplitContainer (onFocus is provided), use relative sizing
  // instead of absolute positioning to work with flex layout
  const inSplit = !!onFocus;

  return (
    <div
      ref={wrapperRef}
      style={inSplit ? {
        width: '100%',
        height: '100%',
        overflow: 'hidden',
        position: 'relative',
      } : {
        position: 'absolute',
        top: 0,
        left: 0,
        right: 0,
        bottom: 0,
        display: isActive ? 'block' : 'none',
        overflow: 'hidden',
      }}
      onContextMenu={handleContextMenu}
      onClick={() => onFocus?.()}
    >
      <div
        ref={containerRef}
        style={{
          width: '100%',
          height: '100%',
          overflow: 'hidden',
        }}
      />
      {logging && !recording && (
        <div style={{
          position: 'absolute', top: 8, right: 8, zIndex: 10,
          display: 'flex', alignItems: 'center', gap: '6px',
          background: 'rgba(248,81,73,0.15)', border: '1px solid rgba(248,81,73,0.3)',
          borderRadius: '4px', padding: '3px 10px', fontSize: '11px',
          cursor: 'pointer',
        }} onClick={handleStopLog}>
          <span style={{ color: '#f85149', animation: 'pulse 1.5s infinite' }}>●</span>
          <span style={{ color: '#f85149' }}>录制中</span>
          <span style={{ color: '#8b949e' }}>点击停止</span>
        </div>
      )}
      {recording && (
        <div style={{
          position: 'absolute', top: 8, right: 8, zIndex: 10,
          display: 'flex', alignItems: 'center', gap: '6px',
          background: 'rgba(0,212,255,0.15)', border: '1px solid rgba(0,212,255,0.3)',
          borderRadius: '4px', padding: '3px 10px', fontSize: '11px',
          cursor: 'pointer',
        }} onClick={handleStopRecording}>
          <span style={{ color: '#00d4ff', animation: 'pulse 1.5s infinite' }}>●</span>
          <span style={{ color: '#00d4ff' }}>录屏中</span>
          <span style={{ color: '#8b949e' }}>点击停止</span>
        </div>
      )}
      {searchVisible && (
        <div style={{
          position: 'absolute', top: 8, right: 8, zIndex: 20,
          display: 'flex', alignItems: 'center', gap: '4px',
          background: 'rgba(22,27,34,0.95)', border: '1px solid #30363d',
          borderRadius: '6px', padding: '4px 8px', fontSize: '12px',
          boxShadow: '0 4px 12px rgba(0,0,0,0.4)',
        }}>
          <input
            ref={searchInputRef}
            type="text"
            value={searchTerm}
            onChange={(e) => setSearchTerm(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === 'Enter') {
                if (e.shiftKey) handleSearchPrevious();
                else handleSearchNext();
              }
              if (e.key === 'Escape') {
                setSearchVisible(false);
                setSearchTerm('');
              }
            }}
            placeholder="搜索..."
            style={{
              background: '#0d1117', border: '1px solid #30363d', borderRadius: '4px',
              padding: '3px 8px', color: '#e6edf3', outline: 'none', width: '180px',
              fontSize: '12px',
            }}
          />
          <button
            onClick={handleSearchPrevious}
            title="上一个 (Shift+Enter)"
            style={{
              background: 'transparent', border: '1px solid #30363d', borderRadius: '4px',
              color: '#8b949e', padding: '3px 8px', cursor: 'pointer', fontSize: '11px',
              whiteSpace: 'nowrap',
            }}
          >上一个</button>
          <button
            onClick={handleSearchNext}
            title="下一个 (Enter)"
            style={{
              background: 'transparent', border: '1px solid #30363d', borderRadius: '4px',
              color: '#8b949e', padding: '3px 8px', cursor: 'pointer', fontSize: '11px',
              whiteSpace: 'nowrap',
            }}
          >下一个</button>
          <button
            onClick={() => { setSearchVisible(false); setSearchTerm(''); }}
            title="关闭 (Esc)"
            style={{
              background: 'transparent', border: 'none', color: '#8b949e',
              cursor: 'pointer', fontSize: '14px', padding: '0 4px', lineHeight: 1,
            }}
          >{'×'}</button>
        </div>
      )}
      {contextMenu && (
        <ContextMenu
          x={contextMenu.x}
          y={contextMenu.y}
          onClose={() => setContextMenu(null)}
          items={contextMenuItems}
        />
      )}
      {zmodemTransfer && (
        <div style={{
          position: 'absolute', bottom: 0, left: 0, right: 0,
          background: 'rgba(22,27,34,0.95)', padding: '8px 16px',
          borderTop: '1px solid #30363d', zIndex: 10,
        }}>
          <div style={{ display: 'flex', alignItems: 'center', gap: '8px', fontSize: '12px' }}>
            <span style={{ color: '#00d4ff', fontWeight: 'bold' }}>ZMODEM</span>
            <span style={{ color: zmodemTransfer.status === 'sending' ? '#3fb950' : '#00d4ff', fontSize: '11px' }}>
              {zmodemTransfer.status === 'sending' ? '↑' : '↓'}
            </span>
            <span style={{ color: '#e6edf3' }}>
              {zmodemTransfer.filename || '等待传输...'}
            </span>
            {zmodemTransfer.totalSize > 0 && (
              <>
                <div style={{
                  flex: 1, height: '4px', background: '#21262d',
                  borderRadius: '2px', overflow: 'hidden',
                }}>
                  <div style={{
                    height: '100%', borderRadius: '2px',
                    width: `${(zmodemTransfer.bytesReceived / zmodemTransfer.totalSize * 100).toFixed(0)}%`,
                    background: '#00d4ff', transition: 'width 0.3s',
                  }} />
                </div>
                <span style={{ color: '#8b949e' }}>
                  {(zmodemTransfer.bytesReceived / 1024 / 1024).toFixed(1)}M / {(zmodemTransfer.totalSize / 1024 / 1024).toFixed(1)}M
                </span>
              </>
            )}
            {zmodemTransfer.status === 'complete' && (
              <span style={{ color: '#3fb950' }}>{'✓ 完成'}</span>
            )}
            <button
              onClick={() => setZmodemTransfer(null)}
              style={{ color: '#8b949e', background: 'none', border: 'none', cursor: 'pointer', fontSize: '14px', padding: '0 4px', marginLeft: '4px' }}
            >{'×'}</button>
          </div>
        </div>
      )}
    </div>
  );
}
