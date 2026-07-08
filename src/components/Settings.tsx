import { useState, useEffect } from 'react';
import { TERMINAL_THEMES } from '../themes';

// 跨平台等宽字体:Linux / Windows / macOS 各有的 + 通用安装的
const FONT_OPTIONS = [
  // 通用(用户可能安装)
  'JetBrains Mono',
  'Fira Code',
  'Source Code Pro',
  // Linux
  'Ubuntu Mono',
  'DejaVu Sans Mono',
  'Liberation Mono',
  'Noto Sans Mono',
  // Windows
  'Cascadia Code',
  'Consolas',
  'Courier New',
  // macOS
  'SF Mono',
  'Menlo',
  'Monaco',
  // 兜底
  'monospace',
];


export function getTerminalFont(): string {
  return localStorage.getItem('guishell_terminal_font') || 'Ubuntu Mono';
}

export function getTerminalFontSize(): number {
  return parseInt(localStorage.getItem('guishell_terminal_fontsize') || '15') || 15;
}

export function getTerminalFontFamily(): string {
  if (localStorage.getItem('guishell_use_system_font') === 'true') return 'monospace';
  const primary = getTerminalFont();
  return `'${primary}', 'DejaVu Sans Mono', 'Liberation Mono', 'Noto Sans Mono', monospace`;
}

interface Props {
  onClose: () => void;
  onApply: () => void;
}

export function SettingsPanel({ onClose, onApply }: Props) {
  const [useSystemFont, setUseSystemFont] = useState(() => localStorage.getItem('guishell_use_system_font') === 'true');
  const [font, setFont] = useState(getTerminalFont);
  const [fontSize, setFontSize] = useState(getTerminalFontSize);
  const [theme, setTheme] = useState(() => localStorage.getItem('guishell_terminal_theme') || '暗色默认');

  const allThemeNames = Object.keys(TERMINAL_THEMES);
  const [themeSearch, setThemeSearch] = useState('');
  const themeNames = themeSearch
    ? allThemeNames.filter(n => n.toLowerCase().includes(themeSearch.toLowerCase()))
    : allThemeNames;

  useEffect(() => {
    const timer = setTimeout(() => {
      localStorage.setItem('guishell_use_system_font', String(useSystemFont));
      localStorage.setItem('guishell_terminal_font', useSystemFont ? 'monospace' : font);
      localStorage.setItem('guishell_terminal_fontsize', String(fontSize));
      localStorage.setItem('guishell_terminal_theme', theme);
      onApply();
    }, 150);
    return () => clearTimeout(timer);
  }, [font, fontSize, theme, useSystemFont, onApply]);

  return (
    <div className="fixed inset-0 bg-black/60 flex items-center justify-center z-50" onClick={onClose}>
      <div className="bg-surface-light border border-surface-border rounded-lg shadow-xl w-[480px] max-h-[80vh] overflow-auto" onClick={e => e.stopPropagation()}>
        {/* Header */}
        <div className="flex items-center justify-between px-5 py-3 border-b border-surface-border">
          <h2 className="text-base font-semibold text-gray-200">系统设置</h2>
          <span onClick={onClose} className="text-gray-500 hover:text-white cursor-pointer text-lg">×</span>
        </div>

        <div className="p-5 space-y-5">
          {/* 使用系统字体 */}
          <label className="flex items-center gap-2 cursor-pointer">
            <input
              type="checkbox"
              checked={useSystemFont}
              onChange={e => setUseSystemFont(e.target.checked)}
              className="accent-accent-cyan"
            />
            <span className="text-sm text-gray-300">使用系统字体</span>
            <span className="text-xs text-gray-500">勾选后使用操作系统默认等宽字体</span>
          </label>

          {/* 终端字体 */}
          <div style={{ opacity: useSystemFont ? 0.4 : 1, pointerEvents: useSystemFont ? 'none' : 'auto' }}>
            <label className="text-sm text-gray-400 block mb-1.5">终端字体</label>
            <select
              value={font}
              onChange={e => setFont(e.target.value)}
              disabled={useSystemFont}
              className="w-full bg-surface border border-surface-border rounded px-3 py-1.5 text-sm text-gray-200 outline-none focus:border-accent-cyan disabled:opacity-50"
            >
              {FONT_OPTIONS.map(f => (
                <option key={f} value={f} style={{ fontFamily: f }}>{f}</option>
              ))}
            </select>
          </div>

          {/* 字号 */}
          <div style={{ opacity: useSystemFont ? 0.4 : 1, pointerEvents: useSystemFont ? 'none' : 'auto' }}>
            <label className="text-sm text-gray-400 block mb-1.5">字号 (8-48)</label>
            <div className="flex items-center gap-2">
              <button
                onClick={() => setFontSize(s => Math.max(8, s - 1))}
                className="w-7 h-7 flex items-center justify-center border border-surface-border rounded text-gray-300 hover:bg-surface-lighter text-lg"
              >−</button>
              <input
                type="text"
                defaultValue={fontSize}
                key={fontSize}
                onBlur={e => {
                  const v = parseInt(e.target.value);
                  if (isNaN(v) || v < 8 || v > 48) {
                    e.target.value = String(fontSize);
                    e.target.style.borderColor = '#f85149';
                    setTimeout(() => { e.target.style.borderColor = ''; }, 1000);
                  } else {
                    setFontSize(v);
                  }
                }}
                onKeyDown={e => { if (e.key === 'Enter') (e.target as HTMLInputElement).blur(); }}
                className="w-12 text-center bg-surface border border-surface-border rounded px-1 py-0.5 text-sm text-gray-200 outline-none focus:border-accent-cyan"
              />
              <button
                onClick={() => setFontSize(s => Math.min(48, s + 1))}
                className="w-7 h-7 flex items-center justify-center border border-surface-border rounded text-gray-300 hover:bg-surface-lighter text-lg"
              >+</button>
              <input
                type="range"
                min={8}
                max={48}
                value={fontSize}
                onChange={e => setFontSize(parseInt(e.target.value))}
                className="flex-1"
              />
            </div>
          </div>

          {/* 终端主题 */}
          <div>
            <label className="text-sm text-gray-400 block mb-1.5">终端配色 ({allThemeNames.length} 套)</label>
            <input
              value={themeSearch}
              onChange={e => setThemeSearch(e.target.value)}
              placeholder="搜索配色方案..."
              className="w-full bg-surface border border-surface-border rounded px-3 py-1.5 text-sm text-gray-200 outline-none focus:border-accent-cyan mb-2"
            />
            <div className="max-h-40 overflow-auto border border-surface-border rounded">
              {themeNames.map(name => (
                <button
                  key={name}
                  onClick={() => setTheme(name)}
                  className={`w-full text-left px-3 py-1 text-xs hover:bg-surface-lighter ${
                    theme === name
                      ? 'text-accent-cyan bg-accent-cyan/10'
                      : 'text-gray-400'
                  }`}
                >
                  {name}
                </button>
              ))}
            </div>
          </div>

          {/* 预览 */}
          <div>
            <label className="text-sm text-gray-400 block mb-1.5">预览</label>
            <div
              style={{
                fontFamily: useSystemFont ? 'monospace' : `'${font}', monospace`,
                fontSize: `${fontSize}px`,
                background: '#0d1117',
                border: '1px solid #30363d',
                borderRadius: '4px',
                padding: '8px 12px',
                lineHeight: 1.4,
              }}
            >
              <span style={{ color: '#3fb950' }}>user@host</span>
              <span style={{ color: '#8b949e' }}>:</span>
              <span style={{ color: '#58a6ff' }}>~/project</span>
              <span style={{ color: '#8b949e' }}>$ </span>
              <span style={{ color: '#e6edf3' }}>ls -la</span>
              <br />
              <span style={{ color: '#b1bac4' }}>drwxr-xr-x  12 user user 4096 </span>
              <span style={{ color: '#58a6ff' }}>src/</span>
              <br />
              <span style={{ color: '#b1bac4' }}>-rw-r--r--   1 user user 1234 </span>
              <span style={{ color: '#e6edf3' }}>README.md</span>
              <br />
              <span style={{ color: '#b1bac4' }}>-rwxr-xr-x   1 user user 5678 </span>
              <span style={{ color: '#3fb950' }}>build.sh</span>
            </div>
          </div>
        </div>

        {/* Footer */}
        <div className="flex justify-end gap-2 px-5 py-3 border-t border-surface-border">
          <button onClick={onClose} className="px-4 py-1.5 text-sm text-gray-400 hover:text-white">
            关闭
          </button>
        </div>
      </div>
    </div>
  );
}
