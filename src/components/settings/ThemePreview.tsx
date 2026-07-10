import type { ITheme } from '@xterm/xterm';

interface Props {
  name: string;
  theme: ITheme;
  selected: boolean;
  onClick: () => void;
}

export function ThemePreview({ name, theme, selected, onClick }: Props) {
  const colors = [
    theme.black, theme.red, theme.green, theme.yellow,
    theme.blue, theme.magenta, theme.cyan, theme.white,
    theme.brightBlack, theme.brightRed,
  ];

  return (
    <div
      onClick={onClick}
      className={`cursor-pointer rounded-lg overflow-hidden border-2 transition-colors ${
        selected ? 'border-accent-cyan' : 'border-transparent hover:border-surface-border'
      }`}
    >
      <div className="flex items-center justify-between px-3 py-1.5 bg-surface-light">
        <div className="flex items-center gap-2">
          {selected && <span className="text-accent-cyan text-sm">✓</span>}
          <span className={`text-sm ${selected ? 'text-accent-cyan' : 'text-gray-300'}`}>{name}</span>
        </div>
        <div className="flex gap-0.5">
          {colors.map((c, i) => (
            <span key={i} className="w-2.5 h-2.5 rounded-full inline-block" style={{ backgroundColor: c || '#888' }} />
          ))}
        </div>
      </div>
      <div style={{
        backgroundColor: theme.background || '#000',
        fontFamily: "'Ubuntu Mono', 'DejaVu Sans Mono', monospace",
        fontSize: '12px', lineHeight: 1.4, padding: '6px 10px',
      }}>
        <div>
          <span style={{ color: theme.green || '#0f0' }}>john</span>
          <span style={{ color: theme.foreground || '#fff' }}>@</span>
          <span style={{ color: theme.blue || '#00f' }}>doe-pc</span>
          <span style={{ color: theme.foreground || '#fff' }}>$ </span>
          <span style={{ color: theme.foreground || '#fff' }}>ls</span>
        </div>
        <div>
          <span style={{ color: theme.foreground || '#fff' }}>-rwxr-xr-x 1 root </span>
          <span style={{ color: theme.green || '#0f0' }}>Documents</span>
        </div>
        <div>
          <span style={{ color: theme.foreground || '#fff' }}>-rwxr-xr-x 1 root </span>
          <span style={{ backgroundColor: theme.yellow || '#ff0', color: theme.black || '#000' }}>Downloads</span>
        </div>
        <div>
          <span style={{ color: theme.foreground || '#fff' }}>-rwxr-xr-x 1 root </span>
          <span style={{ color: theme.cyan || '#0ff' }}>Pictures</span>
        </div>
        <div>
          <span style={{ color: theme.foreground || '#fff' }}>-rwxr-xr-x 1 root </span>
          <span style={{ color: theme.magenta || '#f0f' }}>Music</span>
        </div>
      </div>
    </div>
  );
}
