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
import { log as appLog, getLogText } from '../../utils/logger';

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

/** Handle a detected ZMODEM session (receive only; send is handled by Rust zmodem_send). */
async function handleZmodemDetection(
  detection: any,
  terminalId: string,
  setZmodemTransfer: React.Dispatch<React.SetStateAction<{
    filename: string;
    bytesReceived: number;
    totalSize: number;
    status: 'receiving' | 'sending' | 'complete';
  } | null>>,
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
  } else {
    // Send session — handled by Rust backend (zmodem_send command)
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
}

export function TerminalPane({ terminalId, isActive, onSplit, onClosePane, onFocus, onOpenRecording }: Props) {
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

  // ---- 历史命令自动补全 ----
  const [acItems, setAcItems] = useState<string[]>([]);
  const [acIndex, setAcIndex] = useState(0);
  const [acPos, setAcPos] = useState<{ x: number; y: number } | null>(null);
  const currentLineRef = useRef('');
  const historyRef = useRef<string[]>([]);
  const acVisibleRef = useRef(false);
  const acItemsRef = useRef<string[]>([]);
  const acIndexRef = useRef(0);
  const acNavigatedRef = useRef(false);
  const isBashRef = useRef(true); // zsh/fish 自带补全,LiteTerm 只在 bash 下启用

  // 检测 shell 类型 + 加载历史
  useEffect(() => {
    function mergeHistory(output: string) {
      if (!output.trim()) return;
      const lines = output.trim().split('\n').filter(l => l && !l.startsWith('#') && l.length > 2);
      const unique = [...new Set([...lines.slice(-500), ...historyRef.current])];
      historyRef.current = unique;
      appLog('AC', 'mergeHistory: 新增' + lines.length + '行, 总计=' + unique.length);
    }

    function detectAndLoad() {
      appLog('AC', '开始加载历史, terminalId=' + terminalId);

      // 1. localStorage 已有的历史
      try {
        const saved = JSON.parse(localStorage.getItem('guishell_cmd_history') || '[]');
        historyRef.current = saved;
      } catch { /* ignore */ }

      // 2. 检测本地 shell 类型
      invoke('read_text_file', { path: '/proc/self/exe' }).catch(() => null);
      // 通过环境变量检测:读 /etc/passwd 当前用户默认 shell 或直接读 SHELL
      invoke('sftp_exec', { sessionId: '__detect_shell__', command: 'echo $SHELL' })
        .catch(() => null); // 本地终端没有 sftp session,忽略

      // 本地 shell 检测:读 SHELL 环境变量(后端)
      invoke('get_default_shell')
        .then((shell: unknown) => {
          if (typeof shell === 'string') {
            const name = shell.split('/').pop() || '';
            const bash = name === 'bash' || name === 'sh';
            isBashRef.current = bash;
            appLog('AC', '本地 shell 检测: SHELL=' + shell + ' isBash=' + bash);
          }
        })
        .catch(() => { isBashRef.current = true; }); // 检测失败默认启用

      // 3. 加载本地 bash 历史
      invoke('read_text_file', { path: '~/.bash_history' })
        .then((output: unknown) => {
          appLog('AC', 'read_text_file ~/.bash_history 成功, 长度=' + (typeof output === 'string' ? output.length : 'non-string'));
          if (typeof output === 'string') mergeHistory(output);
        })
        .catch((e) => { appLog('AC', 'read_text_file ~/.bash_history 失败: ' + e); });
    }

    detectAndLoad();

    // SSH 远端:延迟 3 秒检测 shell + 加载历史
    const timer = setTimeout(() => {
      // 检测远端 shell
      invoke('sftp_exec', { sessionId: terminalId, command: 'basename "$SHELL"' })
        .then((shell: unknown) => {
          if (typeof shell === 'string') {
            const name = shell.trim();
            const bash = name === 'bash' || name === 'sh';
            isBashRef.current = bash;
            appLog('AC', '远端 shell 检测: ' + name + ' isBash=' + bash);
          }
        })
        .catch(() => {});
      // 加载远端历史
      invoke('sftp_exec', { sessionId: terminalId, command: 'cat ~/.bash_history 2>/dev/null' })
        .then((output: unknown) => {
          appLog('AC', 'sftp_exec 远端历史成功, 长度=' + (typeof output === 'string' ? output.length : '?'));
          if (typeof output === 'string') mergeHistory(output);
        })
        .catch(() => {});
    }, 3000);

    return () => clearTimeout(timer);
  }, [terminalId]);

  function updateAutocomplete(line: string) {
    if (!isBashRef.current || line.length < 2) {
      setAcItems([]);
      setAcPos(null);
      acVisibleRef.current = false;
      return;
    }
    const matches = historyRef.current
      .filter(h => h.startsWith(line) && h !== line)
      .reduce((acc, h) => acc.includes(h) ? acc : [...acc, h], [] as string[])
      .slice(0, 6);
    appLog('AC', '匹配: line=' + JSON.stringify(line) + ' historySize=' + historyRef.current.length + ' matches=' + matches.length);
    if (matches.length === 0) {
      setAcItems([]);
      setAcPos(null);
      acVisibleRef.current = false;
      return;
    }
    // 计算弹出位置:光标下方
    const term = termRef.current;
    const wrapper = wrapperRef.current;
    if (term && wrapper) {
      const rowsEl = term.element?.querySelector('.xterm-rows');
      const rowsRect = rowsEl?.getBoundingClientRect();
      const wrapperRect = wrapper.getBoundingClientRect();
      const cellW = (rowsRect?.width || 800) / term.cols;
      const cellH = (rowsRect?.height || 400) / term.rows;
      const buf = term.buffer.active;
      const x = buf.cursorX * cellW;
      const popupHeight = Math.min(matches.length, 6) * 22 + 8;
      const cursorBottom = (buf.cursorY + 1) * cellH;
      const spaceBelow = wrapperRect.height - cursorBottom;
      // 下方空间不够就向上弹出
      const y = spaceBelow >= popupHeight
        ? cursorBottom
        : buf.cursorY * cellH - popupHeight;
      appLog('AC', '位置计算: cursorX=' + buf.cursorX + ' cursorY=' + buf.cursorY + ' cellW=' + cellW.toFixed(1) + ' cellH=' + cellH.toFixed(1) + ' → pos=(' + x.toFixed(0) + ',' + y.toFixed(0) + ')');
      setAcPos({ x, y });
    } else {
      appLog('AC', '位置计算失败: term=' + !!term + ' wrapper=' + !!wrapper);
    }
    setAcItems(matches);
    setAcIndex(-1); // -1 = 无高亮,用户必须按 ↑/↓ 才选中
    acItemsRef.current = matches;
    acIndexRef.current = -1;
    acNavigatedRef.current = false;
    acVisibleRef.current = true;
    appLog('AC', '弹出框已设置: items=' + matches.length + ' acVisibleRef=' + acVisibleRef.current);
  }

  function closeAutocomplete() {
    setAcItems([]);
    setAcPos(null);
    acVisibleRef.current = false;
    acItemsRef.current = [];
    acIndexRef.current = 0;
    acNavigatedRef.current = false;
  }

  function recordCommand(cmd: string) {
    if (!cmd || cmd.length < 2) return;
    const hist = historyRef.current;
    const idx = hist.indexOf(cmd);
    if (idx >= 0) hist.splice(idx, 1);
    hist.push(cmd);
    if (hist.length > 1000) hist.splice(0, hist.length - 1000);
    historyRef.current = hist;
    // 同步到 localStorage
    try {
      const saved: string[] = JSON.parse(localStorage.getItem('guishell_cmd_history') || '[]');
      const merged = [cmd, ...saved.filter(h => h !== cmd)].slice(0, 200);
      localStorage.setItem('guishell_cmd_history', JSON.stringify(merged));
    } catch { /* ignore */ }
  }

  useEffect(() => {
    if (!containerRef.current || !wrapperRef.current || termRef.current) return;

    const term = new Terminal({
      cursorBlink: true,
      rightClickSelectsWord: true,
      fontSize: 15,
      scrollback: 10000,
      fontFamily: "'Ubuntu Mono', 'DejaVu Sans Mono', 'Liberation Mono', 'Noto Sans Mono', monospace",
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

    // 文件路径链接检测:注册 xterm link provider,悬停时高亮+手型光标,点击用默认程序打开
    const pathRegex = /(?:~\/|\.\/|\.\.\/|\/)[^\s'"\[\](){}|<>;,`]+/g;
    term.registerLinkProvider({
      provideLinks(lineNumber: number, callback: (links: Array<{ range: { start: { x: number; y: number }; end: { x: number; y: number } }; text: string; activate: () => void }> | undefined) => void) {
        const bufLine = term.buffer.active.getLine(lineNumber);
        if (!bufLine) { callback(undefined); return; }
        const text = bufLine.translateToString(true);
        const links: Array<{ range: { start: { x: number; y: number }; end: { x: number; y: number } }; text: string; activate: () => void }> = [];
        let m;
        pathRegex.lastIndex = 0;
        while ((m = pathRegex.exec(text)) !== null) {
          let path = m[0].replace(/:\d+(?::\d+)?$/, ''); // 去尾部 :行号:列号
          const startX = m.index + 1; // xterm link 坐标从 1 开始
          const endX = m.index + path.length;
          links.push({
            range: { start: { x: startX, y: lineNumber + 1 }, end: { x: endX, y: lineNumber + 1 } },
            text: path,
            activate: () => {
              invoke('open_file_path', { id: terminalId, path }).catch(() => {});
            },
          });
        }
        callback(links.length > 0 ? links : undefined);
      },
    });

    // User input -> Tauri
    // WebKitGTK（Tauri 在 Linux 的 webview）下，xterm 自身的中文输入法(composition)处理
    // 会产出重复甚至错乱的 onData（实测“看看剧情”被发成“看看剧情剧情”/“剧情看看”）。
    // 改为：组合期间丢弃 xterm 的 onData，由 compositionend 直接取干净的最终文本(e.data)
    // 发一次；组合刚结束的极短窗口内丢弃 xterm 产出的 CJK 回声（已发过干净版）。纯 ASCII
    // 输入(含组合后紧接的英文/回车)始终放行，不会被误丢。
    const sendInput = (data: string) => {
      const bytes = Array.from(new TextEncoder().encode(data));
      invoke('terminal_write', { id: terminalId, data: bytes });
    };
    let imeComposing = false;
    let imeEndedAt = 0;
    let lastComposed = ''; // compositionend 已发的干净文本，用于识别 xterm 的重复回声
    const imeTextarea = term.element?.querySelector('.xterm-helper-textarea') as HTMLTextAreaElement | null;
    if (imeTextarea) {
      imeTextarea.addEventListener('compositionstart', () => { imeComposing = true; });
      imeTextarea.addEventListener('compositionend', (ev: Event) => {
        imeComposing = false;
        const text = (ev as CompositionEvent).data;
        if (text) {
          sendInput(text);
          lastComposed = text;
          imeEndedAt = performance.now(); // 仅在发了干净版后才开启丢弃窗口
        }
      });
    }
    term.onData((data) => {
      if (imeComposing) return;
      if (performance.now() - imeEndedAt < 80 && lastComposed && (/[^\x00-\x7f]/.test(data) || data.includes(lastComposed))) return;

      // ---- 自动补全键盘拦截(用 ref 同步读,避免 state 异步延迟) ----
      if (acVisibleRef.current) {
        if (data === '\x1b[A') {
          // 首次按 ↑:跳到最后一项;否则上移
          acIndexRef.current = acIndexRef.current <= 0
            ? acItemsRef.current.length - 1
            : acIndexRef.current - 1;
          acNavigatedRef.current = true;
          setAcIndex(acIndexRef.current);
          return;
        }
        if (data === '\x1b[B') {
          // 首次按 ↓:跳到第一项;否则下移
          acIndexRef.current = acIndexRef.current < 0
            ? 0
            : Math.min(acItemsRef.current.length - 1, acIndexRef.current + 1);
          acNavigatedRef.current = true;
          setAcIndex(acIndexRef.current);
          return;
        }
        if (data === '\r' && acNavigatedRef.current) {
          // 只有用户主动按过 ↑/↓ 选择后,Enter 才注入补全项;
          // 否则 Enter 就是正常执行用户输入的命令(下面的正常流程处理)
          const line = currentLineRef.current;
          const items = acItemsRef.current;
          const selected = items[Math.min(acIndexRef.current, items.length - 1)];
          if (selected && line.length > 0) {
            const bs = '\x7f'.repeat(line.length);
            sendInput(bs + selected);
            recordCommand(selected);
            currentLineRef.current = '';
            closeAutocomplete();
            return;
          }
        }
        if (data === '\x1b' || data === '\t') { closeAutocomplete(); return; }
      }

      // ---- 追踪当前输入行(重建命令行) ----
      if (data === '\r' || data === '\n') {
        const cmd = currentLineRef.current.trim();
        appLog('AC', '按键: Enter → recordCommand="' + cmd + '"');
        if (cmd) recordCommand(cmd);
        // 检测 shell 切换:用户输入 fish/zsh 时禁用补全,输入 bash/exit 时重新启用
        if (cmd === 'fish' || cmd === 'zsh') {
          isBashRef.current = false;
          appLog('AC', '检测到切换到 ' + cmd + ',禁用补全');
        } else if (cmd === 'bash' || cmd === 'exit') {
          isBashRef.current = true;
          appLog('AC', '检测到切换回 bash/exit,启用补全');
        }
        // adb shell 的 PTY 不继承外层终端尺寸,自动注入 stty 修正
        if (cmd.includes('adb') && cmd.includes('shell')) {
          const t = termRef.current;
          if (t) {
            setTimeout(() => {
              sendInput(`stty cols ${t.cols} rows ${t.rows}\r`);
              appLog('AC', `adb shell 自动修正尺寸: ${t.cols}x${t.rows}`);
            }, 800);
          }
        }
        currentLineRef.current = '';
        closeAutocomplete();
      } else if (data === '\x7f' || data === '\b') {
        currentLineRef.current = currentLineRef.current.slice(0, -1);
        appLog('AC', '按键: Backspace → line="' + currentLineRef.current + '"');
        updateAutocomplete(currentLineRef.current);
      } else if (data === '\x03' || data === '\x15' || data === '\x04') {
        appLog('AC', '按键: Ctrl+C/U/D → 重置');
        currentLineRef.current = '';
        closeAutocomplete();
      } else if (data.startsWith('\x1b')) {
        appLog('AC', '按键: 转义序列 → 关闭补全');
        closeAutocomplete();
      } else if (data.length === 1 && data >= ' ') {
        currentLineRef.current += data;
        appLog('AC', '按键: "' + data + '" → line="' + currentLineRef.current + '" → 调用 updateAutocomplete');
        updateAutocomplete(currentLineRef.current);
      } else if (data.length > 1 && !/[\x00-\x1f]/.test(data)) {
        currentLineRef.current += data;
        appLog('AC', '按键: 多字节"' + data + '" → 关闭补全');
        closeAutocomplete();
      }

      sendInput(data);
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
      on_retract: () => {},
      on_detect: (detection: any) => {
        handleZmodemDetection(detection, terminalId, setZmodemTransfer);
      },
    });

    // Listen for output from Tauri — pass through ZMODEM sentry
    // disposed 标记:同步屏蔽回调,避免 unlisten(异步 Promise)未 resolve 前新旧 listener 短暂共存
    let disposed = false;
    const unlisten = listen<{ id: string; data: number[] }>('terminal-output', (event) => {
      if (disposed) return;
      if (event.payload.id === terminalId) {
        const data = new Uint8Array(event.payload.data);
        try {
          sentry.consume(data);
        } catch (e: any) {
          const msg = typeof e === 'object' && e.message ? e.message : String(e);
          appLog('ZM', 'sentry.consume error: ' + msg);
          try { (sentry as any)._zsession = null; } catch (_) {}
          try { (sentry as any)._parsed_session = null; } catch (_) {}
        }
      }
    });

    // Resize handler: multi-stage fit to handle window animation
    let resizeTimer: ReturnType<typeof setTimeout> | null = null;
    let resizeTimers: ReturnType<typeof setTimeout>[] = [];
    const doFit = () => {
      closeAutocomplete();
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
      disposed = true; // 同步屏蔽:新 listener 注册前旧回调即刻失效,不等 unlisten resolve
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
    // 清屏:发 Ctrl+L 给 shell,清除可见屏幕但保留回溯历史(等价 clear / Ctrl+L)
    invoke('terminal_write', { id: terminalId, data: [0x0c] });
  }

  function handleClearScrollback() {
    // 清空缓存:清空 xterm 整个缓冲(含回溯历史)释放内存,仅保留当前提示行
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
    { label: '清空缓存', onClick: handleClearScrollback },
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
      {/* 历史命令自动补全弹出框 */}
      {acItems.length > 0 && acPos && (
        <div
          style={{
            position: 'absolute',
            left: acPos.x,
            top: acPos.y,
            zIndex: 45,
            background: '#1c2128',
            border: '1px solid #30363d',
            borderRadius: '4px',
            boxShadow: '0 4px 12px rgba(0,0,0,0.5)',
            minWidth: '200px',
            maxWidth: '500px',
            maxHeight: '180px',
            overflowY: 'auto',
            padding: '2px 0',
          }}
          onMouseDown={(e) => e.preventDefault()}
        >
          {acItems.map((item, i) => (
            <div
              key={i}
              onMouseDown={(e) => {
                e.preventDefault();
                const line = currentLineRef.current;
                if (line.length > 0) {
                  const bs = '\x7f'.repeat(line.length);
                  const bytes = Array.from(new TextEncoder().encode(bs + item));
                  invoke('terminal_write', { id: terminalId, data: bytes });
                  recordCommand(item);
                  currentLineRef.current = '';
                  closeAutocomplete();
                }
              }}
              style={{
                padding: '3px 10px',
                fontSize: '12px',
                fontFamily: 'monospace',
                color: i === acIndex ? '#fff' : '#b1bac4',
                background: i === acIndex ? '#264f78' : 'transparent',
                cursor: 'pointer',
                whiteSpace: 'nowrap',
                overflow: 'hidden',
                textOverflow: 'ellipsis',
              }}
            >
              {item}
            </div>
          ))}
        </div>
      )}
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
