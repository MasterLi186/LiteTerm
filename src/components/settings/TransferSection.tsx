import { useState, useEffect } from 'react';
import { invoke } from '@tauri-apps/api/core';

interface Props { onApply: () => void; }

export function TransferSection({ onApply }: Props) {
  const [downloadDir, setDownloadDir] = useState('~/Downloads');
  const [resumeThreshold, setResumeThreshold] = useState(10);
  const [maxRetries, setMaxRetries] = useState(3);
  const [concurrent, setConcurrent] = useState(2);
  const [loaded, setLoaded] = useState(false);

  useEffect(() => {
    invoke<any>('get_settings').then(s => {
      const t = s.transfer || {};
      setDownloadDir(t.default_download_dir ?? '~/Downloads');
      setResumeThreshold(t.resume_threshold_mb ?? 10);
      setMaxRetries(t.max_retries ?? 3);
      setConcurrent(t.concurrent_transfers ?? 2);
      setLoaded(true);
    }).catch(() => setLoaded(true));
  }, []);

  useEffect(() => {
    if (!loaded) return;
    const timer = setTimeout(() => {
      invoke('update_settings', { patch: { transfer: {
        default_download_dir: downloadDir, resume_threshold_mb: resumeThreshold,
        max_retries: maxRetries, concurrent_transfers: concurrent,
      }}}).catch(() => {});
      onApply();
    }, 300);
    return () => clearTimeout(timer);
  }, [downloadDir, resumeThreshold, maxRetries, concurrent, loaded, onApply]);

  return (
    <div className="space-y-6">
      <h2 className="text-lg font-semibold text-gray-200">传输</h2>
      <div className="space-y-4">
        <div>
          <label className="text-sm text-gray-400 block mb-1">默认下载目录</label>
          <input type="text" value={downloadDir} onChange={e => setDownloadDir(e.target.value)}
            className="w-full bg-surface border border-surface-border rounded px-3 py-1.5 text-sm text-gray-200 outline-none focus:border-accent-cyan max-w-md" />
        </div>
        <div>
          <label className="text-sm text-gray-400 block mb-1">断点续传阈值 (MB)</label>
          <input type="number" min={1} max={1000} value={resumeThreshold}
            onChange={e => setResumeThreshold(Math.max(1, parseInt(e.target.value) || 10))}
            className="bg-surface border border-surface-border rounded px-3 py-1.5 text-sm text-gray-200 outline-none focus:border-accent-cyan w-24" />
        </div>
        <div>
          <label className="text-sm text-gray-400 block mb-1">最大重试次数</label>
          <input type="number" min={0} max={10} value={maxRetries}
            onChange={e => setMaxRetries(Math.max(0, parseInt(e.target.value) || 3))}
            className="bg-surface border border-surface-border rounded px-3 py-1.5 text-sm text-gray-200 outline-none focus:border-accent-cyan w-24" />
        </div>
        <div>
          <label className="text-sm text-gray-400 block mb-1">并发传输数 (1-8)</label>
          <input type="number" min={1} max={8} value={concurrent}
            onChange={e => setConcurrent(Math.min(8, Math.max(1, parseInt(e.target.value) || 2)))}
            className="bg-surface border border-surface-border rounded px-3 py-1.5 text-sm text-gray-200 outline-none focus:border-accent-cyan w-24" />
        </div>
      </div>
    </div>
  );
}
