import { useState, useEffect } from 'react';
import { invoke } from '@tauri-apps/api/core';
import type { ConnectionStore } from '../types';

interface TunnelInfo {
  id: string;
  tunnel_type: string;
  local_port: number;
  remote_host: string;
  remote_port: number;
  status: string;
}

interface Props {
  onClose: () => void;
  connections: ConnectionStore;
}

export function TunnelManager({ onClose, connections }: Props) {
  const [tunnels, setTunnels] = useState<TunnelInfo[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [creating, setCreating] = useState(false);

  // Form state
  const [selectedHost, setSelectedHost] = useState('');
  const [localPort, setLocalPort] = useState('');
  const [remoteHost, setRemoteHost] = useState('127.0.0.1');
  const [remotePort, setRemotePort] = useState('');

  useEffect(() => {
    loadTunnels();
  }, []);

  async function loadTunnels() {
    setLoading(true);
    try {
      const list = await invoke<TunnelInfo[]>('list_tunnels');
      setTunnels(list);
    } catch (e) {
      setError(`加载隧道列表失败: ${e}`);
    } finally {
      setLoading(false);
    }
  }

  // Build flat host list from connections
  const hostList: Array<{ key: string; label: string; host: string; port: number; user: string; auth: string; keyPath: string }> = [];
  for (const [groupId, group] of Object.entries(connections.groups)) {
    for (const [hostId, host] of Object.entries(group.hosts)) {
      const resolvedAuth = host.auth === 'keyring' ? 'password' : host.auth;
      hostList.push({
        key: `${groupId}/${hostId}`,
        label: host.label,
        host: host.host,
        port: host.port,
        user: host.user,
        auth: resolvedAuth,
        keyPath: host.key_path,
      });
    }
  }

  async function handleCreate() {
    if (!selectedHost || !localPort || !remotePort) return;
    const hostInfo = hostList.find(h => h.key === selectedHost);
    if (!hostInfo) return;

    setCreating(true);
    setError(null);
    try {
      await invoke<string>('create_tunnel', {
        host: hostInfo.host,
        port: hostInfo.port,
        user: hostInfo.user,
        password: null,
        authMethod: hostInfo.auth,
        keyPath: hostInfo.keyPath || null,
        tunnelType: 'local',
        localPort: parseInt(localPort, 10),
        remoteHost,
        remotePort: parseInt(remotePort, 10),
      });
      setLocalPort('');
      setRemotePort('');
      await loadTunnels();
    } catch (e) {
      setError(`创建隧道失败: ${e}`);
    } finally {
      setCreating(false);
    }
  }

  async function handleClose(id: string) {
    try {
      await invoke('close_tunnel', { id });
      await loadTunnels();
    } catch (e) {
      setError(`关闭隧道失败: ${e}`);
    }
  }

  return (
    <div className="fixed inset-0 bg-black/60 flex items-center justify-center z-50" onClick={onClose}>
      <div
        className="bg-surface-light border border-surface-border rounded-lg shadow-xl"
        style={{ width: '520px', maxHeight: '80vh', display: 'flex', flexDirection: 'column' }}
        onClick={(e) => e.stopPropagation()}
      >
        {/* Header */}
        <div className="flex items-center justify-between px-5 py-3 border-b border-surface-border">
          <h2 className="text-sm font-semibold text-gray-200">SSH 隧道管理</h2>
          <button
            onClick={onClose}
            className="text-gray-500 hover:text-white text-lg leading-none"
          >{'x'}</button>
        </div>

        {/* Body */}
        <div className="flex-1 overflow-y-auto px-5 py-4" style={{ minHeight: 0 }}>
          {error && (
            <div className="mb-3 px-3 py-2 bg-red-500/10 border border-red-500/30 rounded text-xs text-red-400 flex items-center justify-between">
              <span>{error}</span>
              <button onClick={() => setError(null)} className="hover:text-white ml-2">{'x'}</button>
            </div>
          )}

          {/* Active tunnels */}
          <div className="mb-4">
            <div className="text-xs text-gray-400 font-semibold mb-2">活跃隧道 ({tunnels.length})</div>
            {loading ? (
              <div className="text-xs text-gray-500 text-center py-4">加载中...</div>
            ) : tunnels.length === 0 ? (
              <div className="text-xs text-gray-500 text-center py-4">暂无活跃隧道</div>
            ) : (
              <div className="space-y-1">
                {tunnels.map((tunnel) => (
                  <div
                    key={tunnel.id}
                    className="flex items-center justify-between px-3 py-2 bg-surface rounded border border-surface-border"
                  >
                    <div className="flex-1 min-w-0">
                      <div className="text-xs text-gray-200 font-mono">
                        :{tunnel.local_port} {'-->'} {tunnel.remote_host}:{tunnel.remote_port}
                      </div>
                      <div className="text-[10px] text-gray-500">
                        {tunnel.tunnel_type} | {tunnel.status}
                      </div>
                    </div>
                    <button
                      onClick={() => handleClose(tunnel.id)}
                      className="text-[10px] px-2 py-0.5 border border-surface-border rounded text-gray-400 hover:text-accent-red hover:border-red-500/50 ml-2 flex-shrink-0"
                    >关闭</button>
                  </div>
                ))}
              </div>
            )}
          </div>

          {/* Create new tunnel */}
          <div>
            <div className="text-xs text-gray-400 font-semibold mb-2">新建隧道</div>
            <div className="space-y-2">
              <div>
                <label className="block text-[10px] text-gray-500 mb-0.5">SSH 主机</label>
                <select
                  value={selectedHost}
                  onChange={(e) => setSelectedHost(e.target.value)}
                  className="w-full bg-surface border border-surface-border rounded px-2 py-1.5 text-xs text-gray-200 outline-none focus:border-accent-cyan"
                >
                  <option value="">选择连接...</option>
                  {hostList.map((h) => (
                    <option key={h.key} value={h.key}>
                      {h.label} ({h.host}:{h.port})
                    </option>
                  ))}
                </select>
              </div>
              <div className="flex gap-2">
                <div className="flex-1">
                  <label className="block text-[10px] text-gray-500 mb-0.5">本地端口</label>
                  <input
                    type="text"
                    value={localPort}
                    onChange={(e) => setLocalPort(e.target.value)}
                    placeholder="8080"
                    className="w-full bg-surface border border-surface-border rounded px-2 py-1.5 text-xs text-gray-200 outline-none focus:border-accent-cyan"
                  />
                </div>
                <div className="flex-1">
                  <label className="block text-[10px] text-gray-500 mb-0.5">远程主机</label>
                  <input
                    type="text"
                    value={remoteHost}
                    onChange={(e) => setRemoteHost(e.target.value)}
                    placeholder="127.0.0.1"
                    className="w-full bg-surface border border-surface-border rounded px-2 py-1.5 text-xs text-gray-200 outline-none focus:border-accent-cyan"
                  />
                </div>
                <div className="flex-1">
                  <label className="block text-[10px] text-gray-500 mb-0.5">远程端口</label>
                  <input
                    type="text"
                    value={remotePort}
                    onChange={(e) => setRemotePort(e.target.value)}
                    placeholder="3306"
                    className="w-full bg-surface border border-surface-border rounded px-2 py-1.5 text-xs text-gray-200 outline-none focus:border-accent-cyan"
                  />
                </div>
              </div>
              <button
                onClick={handleCreate}
                disabled={creating || !selectedHost || !localPort || !remotePort}
                className="w-full px-3 py-1.5 text-xs bg-accent-cyan/20 text-accent-cyan rounded hover:bg-accent-cyan/30 disabled:opacity-50 disabled:cursor-not-allowed"
              >{creating ? '创建中...' : '创建隧道'}</button>
            </div>
          </div>
        </div>

        {/* Footer */}
        <div className="px-5 py-3 border-t border-surface-border flex justify-end">
          <button
            onClick={onClose}
            className="px-3 py-1 text-xs text-gray-400 hover:text-white"
          >关闭</button>
        </div>
      </div>
    </div>
  );
}
