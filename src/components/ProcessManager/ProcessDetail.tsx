import type { ProcessFullDetail } from '../../types';

interface Props {
  detail: ProcessFullDetail;
  onClose: () => void;
}

export function ProcessDetailPanel({ detail, onClose }: Props) {
  return (
    <div className="p-3 text-xs overflow-auto" style={{ maxHeight: '50vh' }}>
      <div className="flex items-center justify-between mb-3">
        <span className="text-accent-cyan font-semibold text-sm">
          PID {detail.pid} — {detail.command}
        </span>
        <button onClick={onClose} className="text-gray-500 hover:text-white text-lg px-1">×</button>
      </div>

      {/* 基本信息 */}
      <div className="grid grid-cols-[auto,1fr] gap-x-4 gap-y-1.5 mb-4">
        <span className="text-gray-500">PID</span>
        <span>{detail.pid}</span>
        <span className="text-gray-500">用户</span>
        <span>{detail.user}</span>
        <span className="text-gray-500">内存</span>
        <span>{detail.mem}</span>
        <span className="text-gray-500">可执行文件</span>
        <span className="break-all font-mono text-accent-green">{detail.location || '—'}</span>
        <span className="text-gray-500">工作目录</span>
        <span className="break-all font-mono">{detail.working_dir || '—'}</span>
      </div>

      {/* 进程树链 */}
      {detail.ancestors && detail.ancestors.length > 0 && (
        <div className="mb-4">
          <div className="text-gray-400 mb-1.5 font-semibold">进程树</div>
          <div className="bg-surface rounded p-2 font-mono text-[11px]">
            {[...detail.ancestors].reverse().map((a, i) => (
              <div key={a.pid} style={{ paddingLeft: `${i * 16}px` }}>
                <span className="text-gray-500">{i > 0 ? '└─ ' : ''}</span>
                <span className="text-accent-cyan">{a.pid}</span>
                <span className="text-gray-400 mx-1">{a.name}</span>
                {a.cmdline && a.cmdline !== a.name && (
                  <span className="text-gray-600 truncate" style={{ maxWidth: '400px', display: 'inline-block', verticalAlign: 'bottom', overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}
                    title={a.cmdline}>{a.cmdline}</span>
                )}
              </div>
            ))}
          </div>
        </div>
      )}

      {/* 完整命令行 */}
      <div className="mb-4">
        <div className="text-gray-400 mb-1.5 font-semibold">完整命令行</div>
        <div className="bg-surface rounded p-2 break-all font-mono text-[11px] text-accent-green select-all">
          {detail.full_command || '—'}
        </div>
      </div>

      {/* 环境变量 */}
      {detail.environ.length > 0 && (
        <div>
          <div className="text-gray-400 mb-1.5 font-semibold">环境变量 ({detail.environ.length})</div>
          <div className="bg-surface rounded p-2 max-h-48 overflow-auto">
            <table className="text-[11px] font-mono w-full">
              <tbody>
                {detail.environ.map((env, i) => (
                  <tr key={i} className="hover:bg-surface-lighter">
                    <td className="pr-3 text-accent-cyan whitespace-nowrap align-top py-0.5">
                      {env.key}
                    </td>
                    <td className="break-all text-gray-300 py-0.5">{env.value}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        </div>
      )}
    </div>
  );
}
