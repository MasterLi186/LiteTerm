import { useEffect, useRef, useState } from 'react';
import { Terminal } from '@xterm/xterm';
import { FitAddon } from '@xterm/addon-fit';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import * as Zmodem from 'zmodem.js';
import '@xterm/xterm/css/xterm.css';

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

/** Handle a detected ZMODEM session (receive or send). */
async function handleZmodemDetection(
  detection: any,
  terminalId: string,
  setZmodemTransfer: React.Dispatch<React.SetStateAction<{
    filename: string;
    bytesReceived: number;
    totalSize: number;
    status: 'receiving' | 'complete';
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

        try {
          await invoke('save_file', {
            path: `~/Downloads/${filename}`,
            data: Array.from(merged),
          });
        } catch (e) {
          console.error('Failed to save ZMODEM file:', e);
        }

        setZmodemTransfer({ filename, bytesReceived: totalLen, totalSize, status: 'complete' });
      });
    });

    session.on('session_end', () => {
      setTimeout(() => setZmodemTransfer(null), 3000);
    });

    session.start();
  } else {
    // Send session — not yet implemented, just abort gracefully
    session.close();
  }
}

interface Props {
  terminalId: string;
  isActive: boolean;
  onSplit?: (direction: 'horizontal' | 'vertical') => void;
  onClosePane?: () => void;
  onFocus?: () => void;
}

export function TerminalPane({ terminalId, isActive, onSplit, onClosePane, onFocus }: Props) {
  const wrapperRef = useRef<HTMLDivElement>(null);
  const containerRef = useRef<HTMLDivElement>(null);
  const termRef = useRef<Terminal | null>(null);
  const fitRef = useRef<FitAddon | null>(null);
  const [contextMenu, setContextMenu] = useState<{ x: number; y: number } | null>(null);
  const [zmodemTransfer, setZmodemTransfer] = useState<{
    filename: string;
    bytesReceived: number;
    totalSize: number;
    status: 'receiving' | 'complete';
  } | null>(null);

  useEffect(() => {
    if (!containerRef.current || !wrapperRef.current || termRef.current) return;

    const term = new Terminal({
      cursorBlink: true,
      fontSize: 14,
      fontFamily: "'DejaVu Sans Mono', 'Liberation Mono', 'Noto Sans Mono', monospace",
      theme: {
        background: '#0d1117',
        foreground: '#e6edf3',
        cursor: '#00d4ff',
        selectionBackground: '#264f78',
        black: '#484f58',
        red: '#ff7b72',
        green: '#3fb950',
        yellow: '#d29922',
        blue: '#58a6ff',
        magenta: '#bc8cff',
        cyan: '#39d353',
        white: '#b1bac4',
        brightBlack: '#6e7681',
        brightRed: '#ffa198',
        brightGreen: '#56d364',
        brightYellow: '#e3b341',
        brightBlue: '#79c0ff',
        brightMagenta: '#d2a8ff',
        brightCyan: '#56d364',
        brightWhite: '#f0f6fc',
      },
    });

    const fitAddon = new FitAddon();
    term.loadAddon(fitAddon);
    term.open(containerRef.current);
    termRef.current = term;
    fitRef.current = fitAddon;

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
        term.write(new Uint8Array(octets));
      },
      sender: (octets: number[]) => {
        invoke('terminal_write', { id: terminalId, data: Array.from(octets) });
      },
      on_retract: () => {
        // Detection was retracted (not actually ZMODEM)
      },
      on_detect: (detection: any) => {
        handleZmodemDetection(detection, terminalId, setZmodemTransfer);
      },
    });

    // Listen for output from Tauri — pass through ZMODEM sentry
    const unlisten = listen<{ id: string; data: number[] }>('terminal-output', (event) => {
      if (event.payload.id === terminalId) {
        const data = new Uint8Array(event.payload.data);
        try {
          sentry.consume(data);
        } catch (e) {
          // If sentry fails (e.g. after abort), fall back to direct write
          term.write(data);
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
      term.dispose();
      termRef.current = null;
      fitRef.current = null;
    };
  }, [terminalId]);

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

  const contextMenuItems: ContextMenuItem[] = [
    { label: '复制', onClick: handleCopy },
    { label: '粘贴', onClick: handlePaste },
    { label: '全选', onClick: handleSelectAll },
    { label: '清屏', onClick: handleClear },
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
              <span style={{ color: '#3fb950' }}>{'✓'} {'完成'}</span>
            )}
          </div>
        </div>
      )}
    </div>
  );
}
