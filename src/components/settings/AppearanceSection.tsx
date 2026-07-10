import { useState, useEffect } from 'react';
import { invoke } from '@tauri-apps/api/core';

interface Props { onApply: () => void; }

export function AppearanceSection({ onApply }: Props) {
  const [sidebarWidth, setSidebarWidth] = useState(220);
  const [fileBrowserHeight, setFileBrowserHeight] = useState(200);
  const [showSidebar, setShowSidebar] = useState(true);
  const [showFileBrowser, setShowFileBrowser] = useState(true);
  const [loaded, setLoaded] = useState(false);

  useEffect(() => {
    invoke<any>('get_settings').then(s => {
      const a = s.appearance || {};
      setSidebarWidth(a.sidebar_width ?? 220);
      setFileBrowserHeight(a.file_browser_height ?? 200);
      setShowSidebar(a.show_sidebar ?? true);
      setShowFileBrowser(a.show_file_browser ?? true);
      setLoaded(true);
    }).catch(() => setLoaded(true));
  }, []);

  useEffect(() => {
    if (!loaded) return;
    const timer = setTimeout(() => {
      invoke('update_settings', { patch: { appearance: {
        sidebar_width: sidebarWidth, file_browser_height: fileBrowserHeight,
        show_sidebar: showSidebar, show_file_browser: showFileBrowser,
      }}}).catch(() => {});
      onApply();
    }, 300);
    return () => clearTimeout(timer);
  }, [sidebarWidth, fileBrowserHeight, showSidebar, showFileBrowser, loaded, onApply]);

  return (
    <div className="space-y-6">
      <h2 className="text-lg font-semibold text-gray-200">外观</h2>
      <div className="space-y-4">
        <label className="flex items-center gap-2 cursor-pointer">
          <input type="checkbox" checked={showSidebar} onChange={e => setShowSidebar(e.target.checked)} className="accent-accent-cyan" />
          <span className="text-sm text-gray-300">显示侧边栏</span>
        </label>
        <div>
          <label className="text-sm text-gray-400 block mb-1">侧边栏宽度 ({sidebarWidth}px)</label>
          <input type="range" min={100} max={400} value={sidebarWidth} onChange={e => setSidebarWidth(parseInt(e.target.value))} className="w-full max-w-xs" />
        </div>
        <label className="flex items-center gap-2 cursor-pointer">
          <input type="checkbox" checked={showFileBrowser} onChange={e => setShowFileBrowser(e.target.checked)} className="accent-accent-cyan" />
          <span className="text-sm text-gray-300">显示文件管理器</span>
        </label>
        <div>
          <label className="text-sm text-gray-400 block mb-1">文件管理器高度 ({fileBrowserHeight}px)</label>
          <input type="range" min={100} max={500} value={fileBrowserHeight} onChange={e => setFileBrowserHeight(parseInt(e.target.value))} className="w-full max-w-xs" />
        </div>
      </div>
    </div>
  );
}
