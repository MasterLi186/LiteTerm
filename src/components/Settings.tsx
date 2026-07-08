import { useState, useEffect } from 'react';

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

const FONT_SIZE_OPTIONS = [11, 12, 13, 14, 15, 16, 17, 18, 20, 22, 24];

export function getTerminalFont(): string {
  return localStorage.getItem('guishell_terminal_font') || 'Ubuntu Mono';
}

export function getTerminalFontSize(): number {
  return parseInt(localStorage.getItem('guishell_terminal_fontsize') || '15') || 15;
}

export function getTerminalFontFamily(): string {
  const primary = getTerminalFont();
  return `'${primary}', 'DejaVu Sans Mono', 'Liberation Mono', 'Noto Sans Mono', monospace`;
}

interface Props {
  onClose: () => void;
  onApply: () => void;
}

export function SettingsPanel({ onClose, onApply }: Props) {
  const [font, setFont] = useState(getTerminalFont);
  const [fontSize, setFontSize] = useState(getTerminalFontSize);
  const [theme, setTheme] = useState(() => localStorage.getItem('guishell_terminal_theme') || '暗色默认');

  // 读取当前主题列表(从 TerminalPane 导出太耦合,直接硬编码名字)
  const themeNames = ['暗色默认', 'AdventureTime', 'Monokai', 'Solarized Dark', 'Dracula', 'One Dark', '浅色'];

  function handleApply() {
    localStorage.setItem('guishell_terminal_font', font);
    localStorage.setItem('guishell_terminal_fontsize', String(fontSize));
    localStorage.setItem('guishell_terminal_theme', theme);
    onApply();
  }

  useEffect(() => {
    // 实时预览
    handleApply();
  }, [font, fontSize, theme]);

  return (
    <div className="fixed inset-0 bg-black/60 flex items-center justify-center z-50" onClick={onClose}>
      <div className="bg-surface-light border border-surface-border rounded-lg shadow-xl w-[480px] max-h-[80vh] overflow-auto" onClick={e => e.stopPropagation()}>
        {/* Header */}
        <div className="flex items-center justify-between px-5 py-3 border-b border-surface-border">
          <h2 className="text-base font-semibold text-gray-200">系统设置</h2>
          <span onClick={onClose} className="text-gray-500 hover:text-white cursor-pointer text-lg">×</span>
        </div>

        <div className="p-5 space-y-5">
          {/* 终端字体 */}
          <div>
            <label className="text-sm text-gray-400 block mb-1.5">终端字体</label>
            <select
              value={font}
              onChange={e => setFont(e.target.value)}
              className="w-full bg-surface border border-surface-border rounded px-3 py-1.5 text-sm text-gray-200 outline-none focus:border-accent-cyan"
            >
              {FONT_OPTIONS.map(f => (
                <option key={f} value={f} style={{ fontFamily: f }}>{f}</option>
              ))}
            </select>
          </div>

          {/* 字号 */}
          <div>
            <label className="text-sm text-gray-400 block mb-1.5">字号</label>
            <div className="flex items-center gap-3">
              <input
                type="range"
                min={11}
                max={24}
                value={fontSize}
                onChange={e => setFontSize(parseInt(e.target.value))}
                className="flex-1"
              />
              <span className="text-sm text-gray-200 w-8 text-right">{fontSize}</span>
            </div>
          </div>

          {/* 终端主题 */}
          <div>
            <label className="text-sm text-gray-400 block mb-1.5">终端主题</label>
            <div className="grid grid-cols-3 gap-2">
              {themeNames.map(name => (
                <button
                  key={name}
                  onClick={() => setTheme(name)}
                  className={`px-3 py-1.5 text-xs rounded border ${
                    theme === name
                      ? 'border-accent-cyan text-accent-cyan bg-accent-cyan/10'
                      : 'border-surface-border text-gray-400 hover:text-gray-200 hover:border-gray-500'
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
                fontFamily: `'${font}', monospace`,
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
