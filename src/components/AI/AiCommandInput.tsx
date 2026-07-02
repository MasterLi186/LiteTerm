import { useState } from 'react';
import { invoke } from '@tauri-apps/api/core';

const SYSTEM_PROMPT = '你是一个 Linux/macOS 命令助手。根据用户的描述，只返回一条可直接执行的 shell 命令，不要解释，不要 markdown。';

interface Props {
  visible: boolean;
  onClose: () => void;
  onExecute: (command: string) => void;
}

export function AiCommandInput({ visible, onClose, onExecute }: Props) {
  const [input, setInput] = useState('');
  const [result, setResult] = useState('');
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState('');

  if (!visible) return null;

  async function handleSubmit() {
    if (!input.trim() || loading) return;
    setLoading(true);
    setError('');
    setResult('');

    try {
      const status: any = await invoke('ai_status');
      if (!status.sidecar_running) {
        if (!status.model_exists && !status.downloading) {
          invoke('ai_download_model').catch(() => {});
        }
        setError(status.downloading ? 'AI 模型正在下载中，请稍候...' : 'AI 服务未就绪');
        setLoading(false);
        return;
      }

      const resp: string = await invoke('ai_chat', {
        systemPrompt: SYSTEM_PROMPT,
        userMessage: input.trim(),
        maxTokens: 128,
      });
      setResult(resp);
    } catch (e: any) {
      setError(typeof e === 'string' ? e : e.message || 'AI 请求失败');
    }
    setLoading(false);
  }

  function handleKeyDown(e: React.KeyboardEvent) {
    if (e.key === 'Enter' && !e.shiftKey) {
      e.preventDefault();
      handleSubmit();
    }
    if (e.key === 'Escape') {
      onClose();
    }
  }

  return (
    <div
      style={{
        position: 'absolute', top: 36, left: 0, right: 0, zIndex: 50,
        background: '#1c2128', border: '1px solid #30363d', borderRadius: '0 0 8px 8px',
        padding: '8px 12px', display: 'flex', flexDirection: 'column', gap: '6px',
        boxShadow: '0 4px 12px rgba(0,0,0,0.4)',
      }}
      onClick={(e) => e.stopPropagation()}
    >
      <div style={{ display: 'flex', gap: '8px', alignItems: 'center' }}>
        <span style={{ color: '#00d4ff', fontSize: '14px', flexShrink: 0 }}>AI</span>
        <input
          autoFocus
          value={input}
          onChange={(e) => setInput(e.target.value)}
          onKeyDown={handleKeyDown}
          placeholder="描述你想执行的操作（如：查看 80 端口占用）"
          style={{
            flex: 1, background: '#0d1117', border: '1px solid #30363d', borderRadius: '4px',
            padding: '4px 8px', color: '#e6edf3', fontSize: '13px', outline: 'none',
          }}
        />
        <button
          onClick={handleSubmit}
          disabled={loading || !input.trim()}
          style={{
            background: '#00d4ff', color: '#0d1117', border: 'none', borderRadius: '4px',
            padding: '4px 12px', fontSize: '12px', cursor: loading ? 'wait' : 'pointer',
            opacity: loading || !input.trim() ? 0.5 : 1,
          }}
        >
          {loading ? '...' : '生成'}
        </button>
        <span
          onClick={onClose}
          style={{ color: '#8b949e', cursor: 'pointer', fontSize: '16px' }}
        >×</span>
      </div>

      {error && (
        <div style={{ color: '#f85149', fontSize: '12px' }}>{error}</div>
      )}

      {result && (
        <div style={{
          display: 'flex', alignItems: 'center', gap: '8px',
          background: '#0d1117', borderRadius: '4px', padding: '6px 10px',
        }}>
          <code style={{ flex: 1, color: '#3fb950', fontSize: '13px', fontFamily: 'monospace' }}>
            {result}
          </code>
          <button
            onClick={() => { onExecute(result); onClose(); }}
            style={{
              background: '#238636', color: '#fff', border: 'none', borderRadius: '4px',
              padding: '2px 10px', fontSize: '12px', cursor: 'pointer',
            }}
          >执行</button>
          <button
            onClick={() => { navigator.clipboard.writeText(result); }}
            style={{
              background: '#30363d', color: '#e6edf3', border: 'none', borderRadius: '4px',
              padding: '2px 10px', fontSize: '12px', cursor: 'pointer',
            }}
          >复制</button>
        </div>
      )}
    </div>
  );
}
