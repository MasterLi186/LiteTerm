import { useState } from 'react';
import type { AuthMethod } from '../types';

interface Props {
  onClose: () => void;
  onConnect: (params: {
    groupId: string;
    groupLabel: string;
    groupColor: string;
    hostId: string;
    host: string;
    port: number;
    user: string;
    password: string;
    authMethod: AuthMethod;
    keyPath: string;
    label: string;
  }) => void;
}

export function ConnectionDialog({ onClose, onConnect }: Props) {
  const [label, setLabel] = useState('');
  const [host, setHost] = useState('');
  const [port, setPort] = useState('22');
  const [user, setUser] = useState('root');
  const [password, setPassword] = useState('');
  const [authMethod, setAuthMethod] = useState<AuthMethod>('keyring');
  const [keyPath, setKeyPath] = useState('~/.ssh/id_rsa');
  const [group, setGroup] = useState('default');

  const handleSubmit = (e: React.FormEvent) => {
    e.preventDefault();
    onConnect({
      groupId: group,
      groupLabel: group,
      groupColor: '#58a6ff',
      hostId: `${host}:${port}`,
      host,
      port: parseInt(port, 10) || 22,
      user,
      password,
      authMethod,
      keyPath,
      label: label || `${user}@${host}`,
    });
  };

  return (
    <div className="fixed inset-0 bg-black/60 flex items-center justify-center z-50">
      <form
        onSubmit={handleSubmit}
        className="bg-surface-light border border-surface-border rounded-lg p-6 w-96 shadow-2xl"
      >
        <h2 className="text-lg font-semibold text-gray-200 mb-4">新建连接</h2>

        <div className="space-y-3">
          <div>
            <label className="block text-xs text-gray-400 mb-1">名称</label>
            <input
              type="text"
              value={label}
              onChange={(e) => setLabel(e.target.value)}
              placeholder="我的服务器"
              className="w-full bg-surface border border-surface-border rounded px-3 py-1.5 text-sm text-gray-200 focus:outline-none focus:border-accent-cyan"
            />
          </div>

          <div className="flex gap-2">
            <div className="flex-1">
              <label className="block text-xs text-gray-400 mb-1">主机地址</label>
              <input
                type="text"
                value={host}
                onChange={(e) => setHost(e.target.value)}
                placeholder="192.168.1.1"
                required
                className="w-full bg-surface border border-surface-border rounded px-3 py-1.5 text-sm text-gray-200 focus:outline-none focus:border-accent-cyan"
              />
            </div>
            <div className="w-20">
              <label className="block text-xs text-gray-400 mb-1">端口</label>
              <input
                type="text"
                value={port}
                onChange={(e) => setPort(e.target.value)}
                className="w-full bg-surface border border-surface-border rounded px-3 py-1.5 text-sm text-gray-200 focus:outline-none focus:border-accent-cyan"
              />
            </div>
          </div>

          <div>
            <label className="block text-xs text-gray-400 mb-1">用户名</label>
            <input
              type="text"
              value={user}
              onChange={(e) => setUser(e.target.value)}
              required
              className="w-full bg-surface border border-surface-border rounded px-3 py-1.5 text-sm text-gray-200 focus:outline-none focus:border-accent-cyan"
            />
          </div>

          <div>
            <label className="block text-xs text-gray-400 mb-1">认证方式</label>
            <select
              value={authMethod}
              onChange={(e) => setAuthMethod(e.target.value as AuthMethod)}
              className="w-full bg-surface border border-surface-border rounded px-3 py-1.5 text-sm text-gray-200 focus:outline-none focus:border-accent-cyan"
            >
              <option value="keyring">密码</option>
              <option value="key">密钥文件</option>
              <option value="agent">SSH Agent</option>
            </select>
          </div>

          {authMethod === 'keyring' && (
            <div>
              <label className="block text-xs text-gray-400 mb-1">密码</label>
              <input
                type="password"
                value={password}
                onChange={(e) => setPassword(e.target.value)}
                className="w-full bg-surface border border-surface-border rounded px-3 py-1.5 text-sm text-gray-200 focus:outline-none focus:border-accent-cyan"
              />
            </div>
          )}

          {authMethod === 'key' && (
            <>
              <div>
                <label className="block text-xs text-gray-400 mb-1">密钥文件</label>
                <input
                  type="text"
                  value={keyPath}
                  onChange={(e) => setKeyPath(e.target.value)}
                  className="w-full bg-surface border border-surface-border rounded px-3 py-1.5 text-sm text-gray-200 focus:outline-none focus:border-accent-cyan"
                />
              </div>
              <div>
                <label className="block text-xs text-gray-400 mb-1">密钥密码 (可选)</label>
                <input
                  type="password"
                  value={password}
                  onChange={(e) => setPassword(e.target.value)}
                  className="w-full bg-surface border border-surface-border rounded px-3 py-1.5 text-sm text-gray-200 focus:outline-none focus:border-accent-cyan"
                />
              </div>
            </>
          )}

          <div>
            <label className="block text-xs text-gray-400 mb-1">分组</label>
            <input
              type="text"
              value={group}
              onChange={(e) => setGroup(e.target.value)}
              className="w-full bg-surface border border-surface-border rounded px-3 py-1.5 text-sm text-gray-200 focus:outline-none focus:border-accent-cyan"
            />
          </div>
        </div>

        <div className="flex justify-end gap-2 mt-5">
          <button
            type="button"
            onClick={onClose}
            className="px-4 py-1.5 text-sm text-gray-400 hover:text-white border border-surface-border rounded hover:bg-surface-lighter"
          >
            取消
          </button>
          <button
            type="submit"
            className="px-4 py-1.5 text-sm text-white bg-accent-cyan/20 border border-accent-cyan/50 rounded hover:bg-accent-cyan/30"
          >
            连接
          </button>
        </div>
      </form>
    </div>
  );
}
