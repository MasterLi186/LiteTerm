import { useState } from 'react';
import { TerminalSection } from './TerminalSection';
import { AppearanceSection } from './AppearanceSection';
import { ShortcutsSection } from './ShortcutsSection';
import { SshSection } from './SshSection';
import { TransferSection } from './TransferSection';
import { ZmodemSection } from './ZmodemSection';
import { AboutSection } from './AboutSection';

const SECTIONS = [
  { key: 'terminal', label: '终端', icon: '⌨' },
  { key: 'appearance', label: '外观', icon: '🎨' },
  { key: 'shortcuts', label: '快捷键', icon: '⌘' },
  { key: 'ssh', label: 'SSH', icon: '🔒' },
  { key: 'transfer', label: '传输', icon: '📁' },
  { key: 'zmodem', label: 'ZMODEM', icon: '📡' },
  { key: 'about', label: '关于', icon: 'ℹ' },
] as const;

type SectionKey = typeof SECTIONS[number]['key'];

interface Props {
  onApply: () => void;
}

export function SettingsTab({ onApply }: Props) {
  const [activeSection, setActiveSection] = useState<SectionKey>('terminal');

  return (
    <div className="flex h-full bg-surface">
      <div className="w-[180px] border-r border-surface-border bg-surface-light flex flex-col py-2">
        {SECTIONS.map(s => (
          <button
            key={s.key}
            onClick={() => setActiveSection(s.key)}
            className={`flex items-center gap-2 px-4 py-2 text-sm text-left transition-colors ${
              activeSection === s.key
                ? 'text-accent-cyan bg-accent-cyan/10 border-r-2 border-accent-cyan'
                : 'text-gray-400 hover:text-gray-200 hover:bg-surface-lighter'
            }`}
          >
            <span className="text-base">{s.icon}</span>
            {s.label}
          </button>
        ))}
      </div>
      <div className="flex-1 overflow-auto p-6">
        {activeSection === 'terminal' && <TerminalSection onApply={onApply} />}
        {activeSection === 'appearance' && <AppearanceSection onApply={onApply} />}
        {activeSection === 'shortcuts' && <ShortcutsSection onApply={onApply} />}
        {activeSection === 'ssh' && <SshSection onApply={onApply} />}
        {activeSection === 'transfer' && <TransferSection onApply={onApply} />}
        {activeSection === 'zmodem' && <ZmodemSection onApply={onApply} />}
        {activeSection === 'about' && <AboutSection />}
      </div>
    </div>
  );
}
