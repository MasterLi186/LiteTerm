import type { ProcessFullDetail } from '../../types';

interface Props {
  detail: ProcessFullDetail;
  onClose: () => void;
}

export function ProcessDetailPanel({ detail, onClose }: Props) {
  return (
    <div className="p-3 text-xs max-h-64 overflow-auto">
      <div className="flex items-center justify-between mb-2">
        <span className="text-accent-cyan font-semibold">
          {detail.pid} - {detail.command}
        </span>
        <button
          onClick={onClose}
          className="text-gray-500 hover:text-white"
        >
          x
        </button>
      </div>

      <div className="grid grid-cols-[auto,1fr] gap-x-4 gap-y-1 mb-3">
        <span className="text-gray-400">PID:</span>
        <span>{detail.pid}</span>
        <span className="text-gray-400">名称:</span>
        <span>{detail.command}</span>
        <span className="text-gray-400">位置:</span>
        <span className="break-all">{detail.location}</span>
        <span className="text-gray-400">工作目录:</span>
        <span className="break-all">{detail.working_dir}</span>
      </div>

      <div className="mb-3">
        <div className="text-gray-400 mb-1">完整命令行:</div>
        <div className="bg-surface rounded p-2 break-all font-mono text-[11px]">
          {detail.full_command}
        </div>
      </div>

      {detail.environ.length > 0 && (
        <div>
          <div className="text-gray-400 mb-1">环境变量:</div>
          <div className="bg-surface rounded p-2 max-h-32 overflow-auto">
            <table className="text-[11px] font-mono">
              <thead>
                <tr>
                  <th className="text-left pr-4 text-gray-500">变量名</th>
                  <th className="text-left text-gray-500">变量值</th>
                </tr>
              </thead>
              <tbody>
                {detail.environ.map((env, i) => (
                  <tr key={i}>
                    <td className="pr-4 text-accent-cyan whitespace-nowrap">
                      {env.key}
                    </td>
                    <td className="break-all">{env.value}</td>
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
