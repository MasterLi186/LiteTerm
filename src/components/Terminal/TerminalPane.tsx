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
import { TERMINAL_THEMES } from '../../themes';
import { getTerminalFontFamily } from '../Settings';

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
    document.addEventListener('mousedown', handleClick);
    document.addEventListener('keydown', handleKeyDown);
    return () => {
      document.removeEventListener('click', handleClick);
      document.removeEventListener('mousedown', handleClick);
      document.removeEventListener('keydown', handleKeyDown);
    };
  }, [onClose]);

  return (
    <div
      className="fixed z-50 bg-surface-light border border-surface-border rounded shadow-lg py-1 min-w-[160px]"
      onMouseDown={(e) => e.stopPropagation()}
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
          appLog('ZM', 'ZMODEM 保存失败: ' + String(e));
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
  const isBashRef = useRef(true);
  const isActiveRef = useRef(isActive);
  useEffect(() => { isActiveRef.current = isActive; }, [isActive]); // zsh/fish 自带补全,LiteTerm 只在 bash 下启用

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
      const cursorTop = buf.cursorY * cellH;
      // 默认在光标上方弹出(不挡终端输出),上方不够才向下
      const y = cursorTop >= popupHeight
        ? cursorTop - popupHeight
        : (buf.cursorY + 1) * cellH;
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
      fontSize: (() => { try { return parseInt(localStorage.getItem('guishell_terminal_fontsize') || '15') || 15; } catch { return 15; } })(),
      scrollback: 10000,
      fontFamily: getTerminalFontFamily(),
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

    // wrapper 背景跟随终端主题(消除 xterm canvas 底部间隙)
    const syncWrapperBg = () => {
      const bg = getTerminalTheme().background;
      if (bg && wrapperRef.current) wrapperRef.current.style.backgroundColor = bg;
    };
    syncWrapperBg();

    // Listen for theme changes from other panes / context menu
    const onThemeChange = () => {
      if (termRef.current) {
        termRef.current.options.theme = getTerminalTheme();
      }
      syncWrapperBg();
    };
    themeChangeListeners.add(onThemeChange);

    // 设置面板改了字体/字号/主题后:焦点终端立刻应用,其余延迟 100ms 错开
    const applySettings = () => {
      if (!termRef.current) return;
      termRef.current.options.fontFamily = getTerminalFontFamily();
      termRef.current.options.fontSize = parseInt(localStorage.getItem('guishell_terminal_fontsize') || '15') || 15;
      termRef.current.options.theme = getTerminalTheme();
      syncWrapperBg();
      if (fitRef.current) fitRef.current.fit();
    };
    const onSettingsChanged = () => {
      if (isActiveRef.current) {
        applySettings();
      } else {
        setTimeout(applySettings, 100);
      }
    };
    window.addEventListener('terminal-settings-changed', onSettingsChanged);

    // Force fit: read size from wrapper, write to container, fit, refresh
    const forceFit = (source?: string) => {
      const w = wrapperRef.current;
      const c = containerRef.current;
      const t = termRef.current;
      if (!w || !c || !fitRef.current || !t) {
        appLog('PTY', `fit(${source}): SKIP w=${!!w} c=${!!c} fit=${!!fitRef.current} t=${!!t}`);
        return false;
      }
      const rect = w.getBoundingClientRect();
      if (rect.width < 10 || rect.height < 10) {
        appLog('PTY', `fit(${source}): SKIP rect=${rect.width.toFixed(1)}x${rect.height.toFixed(1)} 太小`);
        return false;
      }
      const beforeCols = t.cols, beforeRows = t.rows;
      c.style.width = `${Math.floor(rect.width)}px`;
      c.style.height = `${Math.floor(rect.height)}px`;
      try {
        fitRef.current.fit();
      } catch (e) {
        appLog('PTY', `fit(${source}): fitAddon.fit() 异常: ${e}`);
        return false;
      }
      const afterCols = t.cols, afterRows = t.rows;
      // 计算 fitAddon 用的字符单元格大小
      const cellW = afterCols > 0 ? (Math.floor(rect.width) / afterCols) : 0;
      const cellH = afterRows > 0 ? (Math.floor(rect.height) / afterRows) : 0;
      // 读取 xterm 内部渲染器的实际测量值(如果可访问)
      let measuredCellW = 0, measuredCellH = 0;
      try {
        const dims = (t as any)._core._renderService.dimensions;
        measuredCellW = dims?.css?.cell?.width || 0;
        measuredCellH = dims?.css?.cell?.height || 0;
      } catch (_) {}
      const font = t.options.fontFamily || '?';
      const fontSize = t.options.fontSize || 0;
      appLog('PTY', `fit(${source}): ${beforeCols}x${beforeRows} → ${afterCols}x${afterRows} | wrapper=${Math.floor(rect.width)}x${Math.floor(rect.height)} | cellCalc=${cellW.toFixed(2)}x${cellH.toFixed(2)} cellReal=${measuredCellW.toFixed(2)}x${measuredCellH.toFixed(2)} | font="${font}" size=${fontSize}`);
      try { t.refresh(0, afterRows - 1); } catch (_) {}
      return true;
    };

    // 等字体加载完再 fit(字体没加载完时 fitAddon 的字符宽高测量不准 → cols/rows 错)
    document.fonts.ready.then(() => {
      requestAnimationFrame(() => forceFit('fonts.ready'));
    });
    // 兜底:字体加载事件不触发时(某些 WebView),重试直到布局就绪
    let initAttempt = 0;
    const tryInitFit = () => {
      initAttempt++;
      if (!forceFit('init#' + initAttempt) && initAttempt < 30) {
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
      // 点击终端画布关闭补全弹窗(对标 WindTerm)
      closeAutocomplete();
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
    // IME 状态机：完整处理 compositionstart/end + 回显消耗
    // 解决 WebView2 + 搜狗/微软拼音等 IME 的双重输入问题
    // 核心：compositionend 发送文本后,后续 onData 的回显用缓冲区累积匹配消耗
    const sendInput = (data: string) => {
      const bytes = Array.from(new TextEncoder().encode(data));
      invoke('terminal_write', { id: terminalId, data: bytes });
    };

    // IME 状态: idle(正常) → composing(组合中) → consuming(消耗回显)
    // idle=正常 composing=组合中 consuming=消耗回显 draining=排水(吞掉xterm延迟输出)
    let imeState: 'idle' | 'composing' | 'consuming' | 'draining' = 'idle';
    let imeComposedText = '';   // compositionend 发送的文本
    let imeEchoBuffer = '';     // 累积的 onData 数据,用于匹配回显
    let imeConsumeTimer: ReturnType<typeof setTimeout> | null = null;
    let imeDrainUntil = 0;      // draining 阶段截止时间

    const imeReset = () => {
      if (imeConsumeTimer) { clearTimeout(imeConsumeTimer); imeConsumeTimer = null; }
      imeState = 'idle';
      imeComposedText = '';
      imeEchoBuffer = '';
      imeDrainUntil = 0;
    };

    // 回显消耗完毕后进入 drain 阶段,吞掉 xterm setTimeout(0) 的延迟重放
    const imeDrain = () => {
      if (imeConsumeTimer) { clearTimeout(imeConsumeTimer); imeConsumeTimer = null; }
      imeState = 'draining';
      imeComposedText = '';
      imeEchoBuffer = '';
      imeDrainUntil = performance.now() + 50;
      // failsafe: 50ms 后强制回 idle(即使没有 onData 触发)
      imeConsumeTimer = setTimeout(() => {
        if (imeState === 'draining') {
          imeState = 'idle';
          appLog('IME', 'drain 超时回 idle');
        }
        imeConsumeTimer = null;
      }, 60);
      appLog('IME', '进入 drain 阶段 50ms');
    };

    const imeTextarea = term.element?.querySelector('.xterm-helper-textarea') as HTMLTextAreaElement | null;
    if (imeTextarea) {
      imeTextarea.addEventListener('compositionstart', () => {
        if (imeConsumeTimer) { clearTimeout(imeConsumeTimer); imeConsumeTimer = null; }
        imeState = 'composing';
        imeEchoBuffer = '';
        appLog('IME', 'compositionstart');
      });

      imeTextarea.addEventListener('compositionend', (ev: Event) => {
        const text = (ev as CompositionEvent).data;
        appLog('IME', 'compositionend: “' + text + '”');
        if (text) {
          sendInput(text);
          imeComposedText = text;
          imeEchoBuffer = '';
          imeState = 'consuming';
          // 安全超时:500ms 后回显仍未消耗完则重置(防卡死)
          imeConsumeTimer = setTimeout(() => {
            appLog('IME', '回显超时重置 (expected=”' + imeComposedText + '”, got=”' + imeEchoBuffer + '”)');
            imeReset();
          }, 500);
        } else {
          // 空 compositionend(Esc 取消等),走 drain 吞掉可能的延迟重放
          imeDrain();
        }
      });

    }

    term.onData((data) => {
      // 状态: composing → 丢弃所有 onData(组合期间 xterm 产出的预编辑回显)
      if (imeState === 'composing') {
        appLog('IME', '组合中丢弃: “' + data.slice(0, 20) + '”');
        return;
      }

      // 状态: draining → 吞掉 xterm setTimeout(0) 的延迟重放(50ms内)
      if (imeState === 'draining') {
        if (performance.now() < imeDrainUntil) {
          appLog('IME', 'drain 丢弃: “' + data.slice(0, 20) + '”');
          return;
        }
        appLog('IME', 'drain 结束,恢复 idle');
        imeState = 'idle';
        // 继续到下面正常处理
      }

      // 状态: consuming → 累积缓冲区匹配回显
      if (imeState === 'consuming') {
        imeEchoBuffer += data;

        if (imeEchoBuffer === imeComposedText) {
          // 完全匹配:回显消耗完毕,进入 drain 吞掉 xterm 延迟重放
          appLog('IME', '回显完全消耗: “' + imeComposedText + '”');
          imeDrain();
          return;
        }

        if (imeComposedText.startsWith(imeEchoBuffer)) {
          // 部分匹配:继续累积
          appLog('IME', '回显部分匹配: “' + imeEchoBuffer + '” / “' + imeComposedText + '”');
          return;
        }

        if (imeEchoBuffer.startsWith(imeComposedText)) {
          // 缓冲区超出回显:消耗回显部分,放行新输入后进入 drain
          const remainder = imeEchoBuffer.slice(imeComposedText.length);
          appLog('IME', '回显消耗+放行: echo=”' + imeComposedText + '”, new=”' + remainder + '”');
          imeDrain();
          if (remainder) {
            sendInput(remainder);
          }
          return;
        }

        // 不匹配:回显格式非预期,fail-open 放行(避免吞掉合法输入)
        appLog('IME', '回显不匹配,放行: expected=”' + imeComposedText + '”, buffer=”' + imeEchoBuffer + '”');
        const flushed = imeEchoBuffer;
        imeDrain();
        sendInput(flushed);
        return;
      }

      // 状态: idle → 正常处理

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
        // 每次命令执行后,等 PTY 回传数据后 forceFit 纠正尺寸
        pendingFit = true;
        // clear/reset 需要额外强制重建渲染纹理(WebView2 canvas 缓存问题)
        if (cmd === 'clear' || cmd === 'reset') {
          pendingRepaint = true;
        }
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
      appLog('PTY', `onResize → terminal_resize: ${cols}x${rows}`);
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
    let pendingFit = false;
    let pendingRepaint = false;
    let pendingFitTimer: ReturnType<typeof setTimeout> | null = null;
    const unlisten = listen<{ id: string; data: number[] }>('terminal-output', (event) => {
      if (disposed) return;
      if (event.payload.id === terminalId) {
        // 命令执行后等 PTY 数据到达,重新计算 rows + 同步尺寸
        if (pendingFit) {
          pendingFit = false;
          if (pendingFitTimer) clearTimeout(pendingFitTimer);
          pendingFitTimer = setTimeout(() => {
            if (!disposed) forceFit('cmd');
            pendingFitTimer = null;
          }, 50);
        }
        // clear/reset: 强制重建渲染纹理 + resize cycle 刷新 WebView2 canvas 缓存
        if (pendingRepaint) {
          pendingRepaint = false;
          setTimeout(() => {
            if (disposed || !termRef.current) return;
            const t = termRef.current;
            appLog('PTY', 'clear 后强制重绘: clearTextureAtlas + resize cycle');
            try { t.clearTextureAtlas(); } catch (_) {}
            // resize cycle: 临时缩小1行再恢复,强制 xterm 完全重绘 viewport
            const cols = t.cols, rows = t.rows;
            try {
              t.resize(cols, rows - 1);
              t.resize(cols, rows);
            } catch (_) {}
            t.refresh(0, rows - 1);
          }, 100);
        }
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
    let resizeTimers: ReturnType<typeof setTimeout>[] = [];
    const doFit = (triggerSource?: string) => {
      closeAutocomplete();
      resizeTimers.forEach(t => clearTimeout(t));
      resizeTimers = [];
      const src = triggerSource || 'resize';
      appLog('PTY', `doFit 触发(${src}), 将在 50/150/400ms 后 forceFit`);
      for (const delay of [50, 150, 400]) {
        const t = setTimeout(() => {
          requestAnimationFrame(() => forceFit(`${src}@${delay}ms`));
        }, delay);
        resizeTimers.push(t);
      }
    };

    const observer = new ResizeObserver(() => doFit('ResizeObserver'));
    if (wrapperRef.current) {
      observer.observe(wrapperRef.current);
    }
    const onWindowResize = () => doFit('window.resize');
    window.addEventListener('resize', onWindowResize);

    return () => {
      disposed = true; // 同步屏蔽:新 listener 注册前旧回调即刻失效,不等 unlisten resolve
      imeReset();
      if (pendingFitTimer) clearTimeout(pendingFitTimer);
      unlisten.then(fn => fn());
      observer.disconnect();
      window.removeEventListener('resize', onWindowResize);
      resizeTimers.forEach(t => clearTimeout(t));
      themeChangeListeners.delete(onThemeChange);
      window.removeEventListener('terminal-settings-changed', onSettingsChanged);
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
