import { useEffect, useRef, useState, useCallback } from 'react';
import { Terminal } from '@xterm/xterm';
import { FitAddon } from '@xterm/addon-fit';
import { invoke } from '@tauri-apps/api/core';
import '@xterm/xterm/css/xterm.css';

interface RecordingPlayerProps {
  filePath: string;
  onClose: () => void;
}

interface AsciicastHeader {
  version: number;
  width: number;
  height: number;
  timestamp?: number;
  title?: string;
}

interface AsciicastEvent {
  time: number;
  data: string;
}

function parseAsciicast(content: string): { header: AsciicastHeader; events: AsciicastEvent[] } {
  const lines = content.split('\n').filter(l => l.trim());
  const header: AsciicastHeader = JSON.parse(lines[0]);
  const events: AsciicastEvent[] = [];
  for (let i = 1; i < lines.length; i++) {
    try {
      const arr = JSON.parse(lines[i]);
      if (Array.isArray(arr) && arr.length >= 3 && arr[1] === 'o') {
        events.push({ time: arr[0], data: arr[2] });
      }
    } catch {
      // skip malformed lines
    }
  }
  return { header, events };
}

function formatTime(seconds: number): string {
  const m = Math.floor(seconds / 60);
  const s = Math.floor(seconds % 60);
  return `${m}:${s.toString().padStart(2, '0')}`;
}

export function RecordingPlayer({ filePath, onClose }: RecordingPlayerProps) {
  const termRef = useRef<HTMLDivElement>(null);
  const terminalRef = useRef<Terminal | null>(null);
  const fitAddonRef = useRef<FitAddon | null>(null);
  const timersRef = useRef<ReturnType<typeof setTimeout>[]>([]);
  const eventsRef = useRef<AsciicastEvent[]>([]);
  const headerRef = useRef<AsciicastHeader | null>(null);

  const [playing, setPlaying] = useState(false);
  const [speed, setSpeed] = useState(1);
  const [currentTime, setCurrentTime] = useState(0);
  const [totalTime, setTotalTime] = useState(0);
  const [loaded, setLoaded] = useState(false);
  const [error, setError] = useState<string | null>(null);
  // Track how far we've played so we can restart from a clean state
  const playIndexRef = useRef(0);
  const playStartWallRef = useRef(0);
  const playStartTimeRef = useRef(0);
  const rafRef = useRef<number>(0);
  const speedRef = useRef(speed);

  // Keep speedRef in sync
  useEffect(() => { speedRef.current = speed; }, [speed]);

  // Load file and parse
  useEffect(() => {
    let cancelled = false;
    (async () => {
      try {
        const content = await invoke<string>('read_text_file', { path: filePath });
        if (cancelled) return;
        const { header, events } = parseAsciicast(content);
        headerRef.current = header;
        eventsRef.current = events;
        const total = events.length > 0 ? events[events.length - 1].time : 0;
        setTotalTime(total);
        setLoaded(true);
      } catch (e: any) {
        if (!cancelled) setError(String(e));
      }
    })();
    return () => { cancelled = true; };
  }, [filePath]);

  // Initialize terminal once loaded
  useEffect(() => {
    if (!loaded || !termRef.current || !headerRef.current) return;

    const header = headerRef.current;
    const term = new Terminal({
      cols: header.width,
      rows: header.height,
      cursorBlink: false,
      disableStdin: true,
      theme: {
        background: '#0d1117',
        foreground: '#e6edf3',
        cursor: '#00d4ff',
        selectionBackground: '#264f78',
        black: '#484f58', red: '#ff7b72', green: '#3fb950', yellow: '#d29922',
        blue: '#58a6ff', magenta: '#bc8cff', cyan: '#39d353', white: '#b1bac4',
        brightBlack: '#6e7681', brightRed: '#ffa198', brightGreen: '#56d364',
        brightYellow: '#e3b341', brightBlue: '#79c0ff', brightMagenta: '#d2a8ff',
        brightCyan: '#56d364', brightWhite: '#f0f6fc',
      },
      scrollback: 5000,
    });

    const fitAddon = new FitAddon();
    term.loadAddon(fitAddon);
    term.open(termRef.current);

    terminalRef.current = term;
    fitAddonRef.current = fitAddon;

    // Fit after a small delay to let the DOM settle
    setTimeout(() => {
      try { fitAddon.fit(); } catch { /* ignore */ }
    }, 50);

    return () => {
      term.dispose();
      terminalRef.current = null;
      fitAddonRef.current = null;
    };
  }, [loaded]);

  // Handle window resize
  useEffect(() => {
    if (!fitAddonRef.current) return;
    const handleResize = () => {
      try { fitAddonRef.current?.fit(); } catch { /* ignore */ }
    };
    window.addEventListener('resize', handleResize);
    return () => window.removeEventListener('resize', handleResize);
  }, [loaded]);

  const clearTimers = useCallback(() => {
    timersRef.current.forEach(clearTimeout);
    timersRef.current = [];
    cancelAnimationFrame(rafRef.current);
  }, []);

  const resetTerminal = useCallback(() => {
    const term = terminalRef.current;
    if (term) {
      term.reset();
    }
    playIndexRef.current = 0;
    setCurrentTime(0);
  }, []);

  const startPlayback = useCallback(() => {
    const term = terminalRef.current;
    const events = eventsRef.current;
    if (!term || events.length === 0) return;

    clearTimers();

    const startIndex = playIndexRef.current;
    const startEventTime = startIndex < events.length ? events[startIndex].time : 0;
    playStartWallRef.current = performance.now();
    playStartTimeRef.current = startEventTime;

    // Schedule events
    for (let i = startIndex; i < events.length; i++) {
      const delay = (events[i].time - startEventTime) / speedRef.current * 1000;
      const idx = i;
      const timer = setTimeout(() => {
        term.write(events[idx].data);
        playIndexRef.current = idx + 1;
        // If this was the last event, stop
        if (idx === events.length - 1) {
          setPlaying(false);
          cancelAnimationFrame(rafRef.current);
          setCurrentTime(events[idx].time);
        }
      }, delay);
      timersRef.current.push(timer);
    }

    // Progress updater via rAF
    const updateProgress = () => {
      const elapsed = (performance.now() - playStartWallRef.current) / 1000 * speedRef.current;
      const ct = Math.min(playStartTimeRef.current + elapsed, totalTime);
      setCurrentTime(ct);
      if (ct < totalTime) {
        rafRef.current = requestAnimationFrame(updateProgress);
      }
    };
    rafRef.current = requestAnimationFrame(updateProgress);

    setPlaying(true);
  }, [clearTimers, totalTime]);

  const pausePlayback = useCallback(() => {
    clearTimers();
    setPlaying(false);
  }, [clearTimers]);

  const handlePlayPause = useCallback(() => {
    if (playing) {
      pausePlayback();
    } else {
      // If at the end, restart
      if (playIndexRef.current >= eventsRef.current.length) {
        resetTerminal();
      }
      startPlayback();
    }
  }, [playing, pausePlayback, startPlayback, resetTerminal]);

  const handleRestart = useCallback(() => {
    pausePlayback();
    resetTerminal();
  }, [pausePlayback, resetTerminal]);

  const handleSpeedChange = useCallback((newSpeed: number) => {
    setSpeed(newSpeed);
    if (playing) {
      // Reschedule with new speed: pause then resume
      clearTimers();
      // speedRef gets updated via the useEffect, but we need it now
      speedRef.current = newSpeed;
      startPlayback();
    }
  }, [playing, clearTimers, startPlayback]);

  const handleProgressClick = useCallback((e: React.MouseEvent<HTMLDivElement>) => {
    const rect = e.currentTarget.getBoundingClientRect();
    const ratio = Math.max(0, Math.min(1, (e.clientX - rect.left) / rect.width));
    const targetTime = ratio * totalTime;

    const wasPlaying = playing;
    clearTimers();

    // Reset terminal and replay all events up to targetTime synchronously
    const term = terminalRef.current;
    const events = eventsRef.current;
    if (!term) return;

    term.reset();
    let idx = 0;
    for (; idx < events.length; idx++) {
      if (events[idx].time > targetTime) break;
      term.write(events[idx].data);
    }
    playIndexRef.current = idx;
    setCurrentTime(targetTime);

    if (wasPlaying) {
      // speedRef is already current
      startPlayback();
    } else {
      setPlaying(false);
    }
  }, [totalTime, playing, clearTimers, startPlayback]);

  // Cleanup on unmount
  useEffect(() => {
    return () => {
      clearTimers();
    };
  }, [clearTimers]);

  if (error) {
    return (
      <div style={{
        width: '100%', height: '100%', display: 'flex', flexDirection: 'column',
        alignItems: 'center', justifyContent: 'center',
        background: '#0d1117', color: '#e6edf3',
      }}>
        <div style={{ color: '#ff7b72', marginBottom: 16 }}>加载失败: {error}</div>
        <button onClick={onClose} style={closeBtnStyle}>关闭</button>
      </div>
    );
  }

  if (!loaded) {
    return (
      <div style={{
        width: '100%', height: '100%', display: 'flex',
        alignItems: 'center', justifyContent: 'center',
        background: '#0d1117', color: '#e6edf3',
      }}>
        加载中...
      </div>
    );
  }

  const progressRatio = totalTime > 0 ? currentTime / totalTime : 0;

  return (
    <div style={{
      width: '100%', height: '100%', position: 'relative',
      display: 'flex', flexDirection: 'column',
      background: '#0d1117', color: '#e6edf3',
    }}>
      {/* Close button */}
      <button
        onClick={onClose}
        style={{
          position: 'absolute', top: 8, right: 8, zIndex: 10,
          background: 'transparent', border: 'none', color: '#e6edf3',
          fontSize: 18, cursor: 'pointer', padding: '2px 8px',
          borderRadius: 4,
        }}
        title="关闭"
      >
        ✕
      </button>

      {/* Terminal area */}
      <div style={{ flex: 1, overflow: 'hidden', padding: '8px 8px 0 8px' }}>
        <div ref={termRef} style={{ width: '100%', height: '100%' }} />
      </div>

      {/* Controls bar */}
      <div style={{
        height: 40, minHeight: 40,
        display: 'flex', alignItems: 'center', gap: 10,
        padding: '0 12px',
        borderTop: '1px solid #30363d',
        background: '#0d1117',
        userSelect: 'none',
      }}>
        {/* Play/Pause */}
        <button onClick={handlePlayPause} style={controlBtnStyle}>
          {playing ? '暂停' : '播放'}
        </button>

        {/* Restart */}
        <button onClick={handleRestart} style={controlBtnStyle}>
          重播
        </button>

        {/* Speed selector */}
        <div style={{ display: 'flex', gap: 2 }}>
          {[1, 2, 5, 10].map(s => (
            <button
              key={s}
              onClick={() => handleSpeedChange(s)}
              style={{
                ...speedBtnStyle,
                background: speed === s ? '#00d4ff' : 'transparent',
                color: speed === s ? '#0d1117' : '#e6edf3',
              }}
            >
              {s}x
            </button>
          ))}
        </div>

        {/* Progress bar */}
        <div
          onClick={handleProgressClick}
          style={{
            flex: 1, height: 6, borderRadius: 3,
            background: '#30363d', cursor: 'pointer',
            position: 'relative', margin: '0 4px',
          }}
        >
          <div style={{
            width: `${progressRatio * 100}%`,
            height: '100%', borderRadius: 3,
            background: '#00d4ff',
            transition: playing ? 'none' : 'width 0.1s',
          }} />
        </div>

        {/* Time display */}
        <span style={{ fontSize: 12, fontFamily: 'monospace', whiteSpace: 'nowrap', color: '#8b949e' }}>
          {formatTime(currentTime)} / {formatTime(totalTime)}
        </span>
      </div>
    </div>
  );
}

const controlBtnStyle: React.CSSProperties = {
  background: 'transparent',
  border: '1px solid #30363d',
  color: '#e6edf3',
  borderRadius: 4,
  padding: '2px 10px',
  cursor: 'pointer',
  fontSize: 13,
  lineHeight: '24px',
};

const speedBtnStyle: React.CSSProperties = {
  border: '1px solid #30363d',
  borderRadius: 4,
  padding: '2px 8px',
  cursor: 'pointer',
  fontSize: 12,
  lineHeight: '20px',
};

const closeBtnStyle: React.CSSProperties = {
  background: '#30363d',
  border: 'none',
  color: '#e6edf3',
  borderRadius: 4,
  padding: '6px 16px',
  cursor: 'pointer',
  fontSize: 14,
};
