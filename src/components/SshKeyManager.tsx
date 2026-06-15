import { useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';

interface SshKeyInfo {
  name: string;
  path: string;
  key_type: string;
  is_public: boolean;
  fingerprint: string;
}

interface Props {
  onClose: () => void;
}

export function SshKeyManager({ onClose }: Props) {
  const [keys, setKeys] = useState<SshKeyInfo[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [generating, setGenerating] = useState(false);
  const [genType, setGenType] = useState('ed25519');
  const [genComment, setGenComment] = useState('');
  const [showGenForm, setShowGenForm] = useState(false);
  const [copiedKey, setCopiedKey] = useState<string | null>(null);
  const [viewingKey, setViewingKey] = useState<string | null>(null);
  const [viewContent, setViewContent] = useState<string>('');

  useEffect(() => {
    loadKeys();
  }, []);

  async function loadKeys() {
    setLoading(true);
    setError(null);
    try {
      const result = await invoke<SshKeyInfo[]>('list_ssh_keys');
      setKeys(result);
    } catch (e) {
      setError(`加载密钥失败: ${e}`);
    } finally {
      setLoading(false);
    }
  }

  async function handleGenerate() {
    if (!genType) return;
    setGenerating(true);
    setError(null);
    try {
      const pubKey = await invoke<string>('generate_ssh_key', {
        keyType: genType,
        comment: genComment || `${genType}-key`,
      });
      setShowGenForm(false);
      setGenComment('');
      await loadKeys();
      // Show the generated public key
      setViewContent(pubKey);
      setViewingKey(`id_${genType}.pub`);
    } catch (e) {
      setError(`生成密钥失败: ${e}`);
    } finally {
      setGenerating(false);
    }
  }

  async function handleViewKey(key: SshKeyInfo) {
    if (!key.is_public) return;
    try {
      const content = await invoke<string>('read_ssh_public_key', { path: key.path });
      setViewContent(content);
      setViewingKey(key.name);
    } catch (e) {
      setError(`读取公钥失败: ${e}`);
    }
  }

  async function handleCopyKey(key: SshKeyInfo) {
    if (!key.is_public) return;
    try {
      const content = await invoke<string>('read_ssh_public_key', { path: key.path });
      await navigator.clipboard.writeText(content.trim());
      setCopiedKey(key.name);
      setTimeout(() => setCopiedKey(null), 2000);
    } catch (e) {
      setError(`复制公钥失败: ${e}`);
    }
  }

  const publicKeys = keys.filter(k => k.is_public);
  const privateKeys = keys.filter(k => !k.is_public);

  return (
    <div className="fixed inset-0 bg-black/60 flex items-center justify-center z-50" onClick={onClose}>
      <div
        className="bg-surface-light border border-surface-border rounded-lg shadow-xl"
        style={{ width: '560px', maxHeight: '80vh', display: 'flex', flexDirection: 'column' }}
        onClick={(e) => e.stopPropagation()}
      >
        {/* Header */}
        <div className="flex items-center justify-between px-5 py-3 border-b border-surface-border">
          <h2 className="text-sm font-semibold text-gray-200">SSH 密钥管理</h2>
          <button
            onClick={onClose}
            className="text-gray-500 hover:text-white text-lg leading-none"
          >{'×'}</button>
        </div>

        {/* Body */}
        <div className="flex-1 overflow-y-auto px-5 py-4" style={{ minHeight: 0 }}>
          {error && (
            <div className="mb-3 px-3 py-2 bg-red-500/10 border border-red-500/30 rounded text-xs text-red-400 flex items-center justify-between">
              <span>{error}</span>
              <button onClick={() => setError(null)} className="hover:text-white ml-2">{'×'}</button>
            </div>
          )}

          {loading ? (
            <div className="text-center text-gray-500 text-sm py-8">加载中...</div>
          ) : (
            <>
              {/* Public Keys */}
              <div className="mb-4">
                <div className="text-xs text-gray-400 font-semibold mb-2">公钥 ({publicKeys.length})</div>
                {publicKeys.length === 0 ? (
                  <div className="text-xs text-gray-500 px-2">未找到公钥</div>
                ) : (
                  <div className="space-y-1">
                    {publicKeys.map((key) => (
                      <div
                        key={key.path}
                        className="flex items-center justify-between px-3 py-2 bg-surface rounded border border-surface-border hover:border-gray-600"
                      >
                        <div className="flex-1 min-w-0">
                          <div className="text-xs text-gray-200 font-mono truncate">{key.name}</div>
                          <div className="text-[10px] text-gray-500 truncate">{key.fingerprint || '无指纹'}</div>
                        </div>
                        <div className="flex items-center gap-1 ml-2 flex-shrink-0">
                          <button
                            onClick={() => handleViewKey(key)}
                            className="text-[10px] px-2 py-0.5 border border-surface-border rounded text-gray-400 hover:text-white hover:border-gray-500"
                          >查看</button>
                          <button
                            onClick={() => handleCopyKey(key)}
                            className="text-[10px] px-2 py-0.5 border border-surface-border rounded text-gray-400 hover:text-accent-cyan hover:border-accent-cyan/50"
                          >{copiedKey === key.name ? '已复制' : '复制'}</button>
                        </div>
                      </div>
                    ))}
                  </div>
                )}
              </div>

              {/* Private Keys */}
              <div className="mb-4">
                <div className="text-xs text-gray-400 font-semibold mb-2">私钥 ({privateKeys.length})</div>
                {privateKeys.length === 0 ? (
                  <div className="text-xs text-gray-500 px-2">未找到私钥</div>
                ) : (
                  <div className="space-y-1">
                    {privateKeys.map((key) => (
                      <div
                        key={key.path}
                        className="flex items-center justify-between px-3 py-2 bg-surface rounded border border-surface-border"
                      >
                        <div className="flex-1 min-w-0">
                          <div className="text-xs text-gray-200 font-mono truncate">{key.name}</div>
                          <div className="text-[10px] text-gray-500">{key.key_type} | {key.fingerprint || '无指纹'}</div>
                        </div>
                      </div>
                    ))}
                  </div>
                )}
              </div>

              {/* View public key content */}
              {viewingKey && (
                <div className="mb-4">
                  <div className="text-xs text-gray-400 font-semibold mb-1">
                    {viewingKey}
                    <button
                      onClick={() => { setViewingKey(null); setViewContent(''); }}
                      className="ml-2 text-gray-500 hover:text-white"
                    >{'×'}</button>
                  </div>
                  <textarea
                    readOnly
                    value={viewContent}
                    className="w-full bg-surface border border-surface-border rounded p-2 text-[10px] font-mono text-gray-300 resize-none"
                    rows={3}
                    onClick={(e) => (e.target as HTMLTextAreaElement).select()}
                  />
                </div>
              )}
            </>
          )}
        </div>

        {/* Footer */}
        <div className="px-5 py-3 border-t border-surface-border flex items-center justify-between">
          {showGenForm ? (
            <div className="flex items-center gap-2 flex-1">
              <select
                value={genType}
                onChange={(e) => setGenType(e.target.value)}
                className="bg-surface border border-surface-border rounded px-2 py-1 text-xs text-gray-200 outline-none"
              >
                <option value="ed25519">Ed25519</option>
                <option value="rsa">RSA</option>
                <option value="ecdsa">ECDSA</option>
              </select>
              <input
                type="text"
                value={genComment}
                onChange={(e) => setGenComment(e.target.value)}
                placeholder="备注 (可选)"
                className="flex-1 bg-surface border border-surface-border rounded px-2 py-1 text-xs text-gray-200 outline-none focus:border-accent-cyan"
              />
              <button
                onClick={handleGenerate}
                disabled={generating}
                className="px-3 py-1 text-xs bg-accent-cyan/20 text-accent-cyan rounded hover:bg-accent-cyan/30 disabled:opacity-50"
              >{generating ? '生成中...' : '确定'}</button>
              <button
                onClick={() => setShowGenForm(false)}
                className="px-2 py-1 text-xs text-gray-400 hover:text-white"
              >取消</button>
            </div>
          ) : (
            <>
              <button
                onClick={() => setShowGenForm(true)}
                className="px-3 py-1 text-xs bg-accent-cyan/20 text-accent-cyan rounded hover:bg-accent-cyan/30"
              >生成新密钥</button>
              <button
                onClick={onClose}
                className="px-3 py-1 text-xs text-gray-400 hover:text-white"
              >关闭</button>
            </>
          )}
        </div>
      </div>
    </div>
  );
}
