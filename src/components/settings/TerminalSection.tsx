import { useState, useEffect } from 'react';
import { TERMINAL_THEMES } from '../../themes';
import { ThemePreview } from './ThemePreview';
import { getTerminalFont, getTerminalFontSize } from '../Settings';

const FONT_OPTIONS = [
  'JetBrains Mono', 'Fira Code', 'Source Code Pro',
  'Ubuntu Mono', 'DejaVu Sans Mono', 'Liberation Mono', 'Noto Sans Mono',
  'Cascadia Code', 'Consolas', 'Courier New',
  'SF Mono', 'Menlo', 'Monaco', 'monospace',
];

interface Props {
  onApply: () => void;
}

export function TerminalSection({ onApply }: Props) {
  const [theme, setTheme] = useState(() => localStorage.getItem('guishell_terminal_theme') || '暗色默认');
  const [useSystemFont, setUseSystemFont] = useState(() => localStorage.getItem('guishell_use_system_font') === 'true');
  const [font, setFont] = useState(getTerminalFont);
  const [fontSize, setFontSize] = useState(getTerminalFontSize);
  const [search, setSearch] = useState('');

  const allThemeNames = Object.keys(TERMINAL_THEMES);
  const filteredNames = search
    ? allThemeNames.filter(n => n.toLowerCase().includes(search.toLowerCase()))
    : allThemeNames;

  useEffect(() => {
    const timer = setTimeout(() => {
      localStorage.setItem('guishell_terminal_theme', theme);
      localStorage.setItem('guishell_use_system_font', String(useSystemFont));
      localStorage.setItem('guishell_terminal_font', useSystemFont ? 'monospace' : font);
      localStorage.setItem('guishell_terminal_fontsize', String(fontSize));
      onApply();
    }, 150);
    return () => clearTimeout(timer);
  }, [theme, useSystemFont, font, fontSize, onApply]);

  return (
    <div className="space-y-6">
      <h2 className="text-lg font-semibold text-gray-200">终端</h2>

      {/* 当前配色 */}
      <div>
        <div className="flex items-center justify-between mb-2">
          <span className="text-sm text-gray-400">当前配色方案: <span className="text-accent-cyan">{theme}</span></span>
          <span className="text-xs text-gray-500">{allThemeNames.length} 套</span>
        </div>
        <ThemePreview
          name={theme}
          theme={TERMINAL_THEMES[theme] || TERMINAL_THEMES['暗色默认']}
          selected={true}
          onClick={() => {}}
        />
      </div>

      {/* 搜索 */}
      <input
        value={search}
        onChange={e => setSearch(e.target.value)}
        placeholder="搜索配色方案..."
        className="w-full bg-surface border border-surface-border rounded px-3 py-1.5 text-sm text-gray-200 outline-none focus:border-accent-cyan"
      />

      {/* 配色列表 */}
      <div className="space-y-3 max-h-[500px] overflow-auto">
        {filteredNames.map(name => (
          <ThemePreview
            key={name}
            name={name}
            theme={TERMINAL_THEMES[name]}
            selected={theme === name}
            onClick={() => setTheme(name)}
          />
        ))}
      </div>

      {/* 字体设置 */}
      <div className="border-t border-surface-border pt-4 space-y-4">
        <label className="flex items-center gap-2 cursor-pointer">
          <input
            type="checkbox"
            checked={useSystemFont}
            onChange={e => setUseSystemFont(e.target.checked)}
            className="accent-accent-cyan"
          />
          <span className="text-sm text-gray-300">使用系统字体</span>
        </label>

        <div style={{ opacity: useSystemFont ? 0.4 : 1, pointerEvents: useSystemFont ? 'none' : 'auto' }}
             className="space-y-3">
          <div>
            <label className="text-sm text-gray-400 block mb-1">终端字体</label>
            <select
              value={font}
              onChange={e => setFont(e.target.value)}
              className="w-full bg-surface border border-surface-border rounded px-3 py-1.5 text-sm text-gray-200 outline-none focus:border-accent-cyan max-w-xs"
            >
              {FONT_OPTIONS.map(f => <option key={f} value={f}>{f}</option>)}
            </select>
          </div>
          <div>
            <label className="text-sm text-gray-400 block mb-1">字号 (8-48)</label>
            <div className="flex items-center gap-2">
              <button onClick={() => setFontSize(s => Math.max(8, s - 1))}
                className="w-7 h-7 flex items-center justify-center border border-surface-border rounded text-gray-300 hover:bg-surface-lighter">−</button>
              <span className="text-sm text-gray-200 w-8 text-center">{fontSize}</span>
              <button onClick={() => setFontSize(s => Math.min(48, s + 1))}
                className="w-7 h-7 flex items-center justify-center border border-surface-border rounded text-gray-300 hover:bg-surface-lighter">+</button>
              <input type="range" min={8} max={48} value={fontSize}
                onChange={e => setFontSize(parseInt(e.target.value))} className="flex-1 max-w-[200px]" />
            </div>
          </div>
        </div>
      </div>
    </div>
  );
}
