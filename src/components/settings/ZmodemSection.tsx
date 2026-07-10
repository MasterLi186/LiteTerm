import { useState, useEffect } from 'react';
import { invoke } from '@tauri-apps/api/core';

interface Props { onApply: () => void; }

export function ZmodemSection({ onApply }: Props) {
  const [enabled, setEnabled] = useState(true);
  const [autoDetect, setAutoDetect] = useState(true);
  const [downloadDir, setDownloadDir] = useState('~/Downloads');
  const [timeout, setTimeoutSecs] = useState(60);
  const [loaded, setLoaded] = useState(false);

  useEffect(() => {
    invoke<any>('get_settings').then(s => {
      const z = s.zmodem || {};
      setEnabled(z.enabled ?? true);
      setAutoDetect(z.auto_detect ?? true);
      setDownloadDir(z.download_dir ?? '~/Downloads');
      setTimeoutSecs(z.timeout_secs ?? 60);
      setLoaded(true);
    }).catch(() => setLoaded(true));
  }, []);

  useEffect(() => {
    if (!loaded) return;
    const timer = setTimeout(() => {
      invoke('update_settings', { patch: { zmodem: {
        enabled, auto_detect: autoDetect, download_dir: downloadDir, timeout_secs: timeout,
      }}}).catch(() => {});
      onApply();
    }, 300);
    return () => clearTimeout(timer);
  }, [enabled, autoDetect, downloadDir, timeout, loaded, onApply]);

  return (
    <div className="space-y-6">
      <h2 className="text-lg font-semibold text-gray-200">ZMODEM</h2>
      <div className="space-y-4">
        <label className="flex items-center gap-2 cursor-pointer">
          <input type="checkbox" checked={enabled} onChange={e => setEnabled(e.target.checked)} className="accent-accent-cyan" />
          <span className="text-sm text-gray-300">启用 ZMODEM</span>
        </label>
        <label className="flex items-center gap-2 cursor-pointer">
          <input type="checkbox" checked={autoDetect} onChange={e => setAutoDetect(e.target.checked)} className="accent-accent-cyan" />
          <span className="text-sm text-gray-300">自动检测</span>
        </label>
        <div>
          <label className="text-sm text-gray-400 block mb-1">下载目录</label>
          <input type="text" value={downloadDir} onChange={e => setDownloadDir(e.target.value)}
            className="w-full bg-surface border border-surface-border rounded px-3 py-1.5 text-sm text-gray-200 outline-none focus:border-accent-cyan max-w-md" />
        </div>
        <div>
          <label className="text-sm text-gray-400 block mb-1">超时时间 (秒)</label>
          <input type="number" min={10} max={600} value={timeout}
            onChange={e => setTimeoutSecs(Math.max(10, parseInt(e.target.value) || 60))}
            className="bg-surface border border-surface-border rounded px-3 py-1.5 text-sm text-gray-200 outline-none focus:border-accent-cyan w-24" />
        </div>
      </div>
    </div>
  );
}
