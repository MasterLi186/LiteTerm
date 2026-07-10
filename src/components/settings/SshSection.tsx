import { useState, useEffect } from 'react';
import { invoke } from '@tauri-apps/api/core';

interface Props { onApply: () => void; }

export function SshSection({ onApply }: Props) {
  const [connectTimeout, setConnectTimeout] = useState(10);
  const [keepalive, setKeepalive] = useState(30);
  const [charset, setCharset] = useState('UTF-8');
  const [loaded, setLoaded] = useState(false);

  useEffect(() => {
    invoke<any>('get_settings').then(s => {
      const ssh = s.ssh || {};
      setConnectTimeout(ssh.connect_timeout_secs ?? 10);
      setKeepalive(ssh.keepalive_interval_secs ?? 30);
      setCharset(ssh.default_charset ?? 'UTF-8');
      setLoaded(true);
    }).catch(() => setLoaded(true));
  }, []);

  useEffect(() => {
    if (!loaded) return;
    const timer = setTimeout(() => {
      invoke('update_settings', { patch: { ssh: {
        connect_timeout_secs: connectTimeout, keepalive_interval_secs: keepalive, default_charset: charset,
      }}}).catch(() => {});
      onApply();
    }, 300);
    return () => clearTimeout(timer);
  }, [connectTimeout, keepalive, charset, loaded, onApply]);

  return (
    <div className="space-y-6">
      <h2 className="text-lg font-semibold text-gray-200">SSH</h2>
      <div className="space-y-4">
        <div>
          <label className="text-sm text-gray-400 block mb-1">连接超时 (秒)</label>
          <input type="number" min={1} max={120} value={connectTimeout}
            onChange={e => setConnectTimeout(Math.max(1, parseInt(e.target.value) || 10))}
            className="bg-surface border border-surface-border rounded px-3 py-1.5 text-sm text-gray-200 outline-none focus:border-accent-cyan w-24" />
        </div>
        <div>
          <label className="text-sm text-gray-400 block mb-1">Keepalive 间隔 (秒)</label>
          <input type="number" min={0} max={300} value={keepalive}
            onChange={e => setKeepalive(Math.max(0, parseInt(e.target.value) || 30))}
            className="bg-surface border border-surface-border rounded px-3 py-1.5 text-sm text-gray-200 outline-none focus:border-accent-cyan w-24" />
        </div>
        <div>
          <label className="text-sm text-gray-400 block mb-1">默认字符集</label>
          <select value={charset} onChange={e => setCharset(e.target.value)}
            className="bg-surface border border-surface-border rounded px-3 py-1.5 text-sm text-gray-200 outline-none focus:border-accent-cyan">
            <option value="UTF-8">UTF-8</option>
            <option value="GBK">GBK</option>
            <option value="GB2312">GB2312</option>
          </select>
        </div>
      </div>
    </div>
  );
}
