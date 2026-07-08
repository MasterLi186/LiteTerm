import { useEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { ProcessDetailPanel } from './ProcessDetail';
import type { ProcessDetail, ProcessFullDetail } from '../../types';

type SortKey = 'pid' | 'user' | 'mem' | 'cpu' | 'command';

function parseMemToBytes(mem: string): number {
  const num = parseFloat(mem);
  if (isNaN(num)) return 0;
  if (mem.endsWith('G')) return num * 1073741824;
  if (mem.endsWith('M')) return num * 1048576;
  if (mem.endsWith('K')) return num * 1024;
  return num;
}

function SortHeader({
  label,
  sortKey,
  current,
  asc,
  onClick,
  width,
  align,
}: {
  label: string;
  sortKey: SortKey;
  current: SortKey;
  asc: boolean;
  onClick: (key: SortKey) => void;
  width: string;
  align?: string;
}) {
  const arrow = current === sortKey ? (asc ? ' ^' : ' v') : '';
  return (
    <th
      className={`${align || 'text-left'} px-2 py-1 text-gray-400 font-normal cursor-pointer hover:text-gray-200 select-none ${width}`}
      onClick={() => onClick(sortKey)}
    >
      {label}
      {arrow}
    </th>
  );
}

interface Props {
  sessionId: string;
  sshParams: {
    host: string;
    port: number;
    user: string;
    password: string | null;
    authMethod: string;
    keyPath: string | null;
  };
  hostLabel: string;
}

export function ProcessTable({ sessionId, sshParams, hostLabel }: Props) {
  const [processes, setProcesses] = useState<ProcessDetail[]>([]);
  const [sortKey, setSortKey] = useState<SortKey>('cpu');
  const [sortAsc, setSortAsc] = useState(false);
  const [selectedPid, setSelectedPid] = useState<number | null>(null);
  const selectedPidRef = useRef<number | null>(null);
  const [detail, setDetail] = useState<ProcessFullDetail | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  // Auto-refresh every 3 seconds(列表 + 已选中的进程详情)
  useEffect(() => {
    loadProcesses();
    const timer = setInterval(() => {
      loadProcesses();
      if (selectedPidRef.current) handleRowClick(selectedPidRef.current);
    }, 3000);
    return () => clearInterval(timer);
  }, []);

  async function loadProcesses() {
    if (processes.length === 0) setLoading(true);
    try {
      // -O lstart 加启动时间(0.026s),不用 -H 进程树(256 核上 24 秒)
      // 输出: PID USER LSTART %CPU RSS COMMAND(lstart 展开为 DAY MON DD HH:MM:SS YYYY)
      const psOutput = await invoke<string>('sftp_exec', {
        sessionId,
        command: "ps -eo pid,user,lstart,pcpu,rss,comm --no-headers 2>/dev/null | awk '$9+0>0.1{print}' | head -100",
      });
      const list: ProcessDetail[] = [];
      for (const line of psOutput.split('\n')) {
        const parts = line.trim().split(/\s+/);
        // 格式: PID USER DAY MON DD HH:MM:SS YYYY %CPU RSS COMM
        if (parts.length < 10) continue;
        const pid = parseInt(parts[0]);
        if (isNaN(pid)) continue;
        const startTime = `${parts[3]} ${parts[4]} ${parts[5]}`;
        const cpu = parseFloat(parts[7]) || 0;
        const rssKb = parseInt(parts[8]) || 0;
        const mem = rssKb >= 1048576 ? `${(rssKb/1048576).toFixed(1)}G` : rssKb >= 1024 ? `${(rssKb/1024).toFixed(1)}M` : `${rssKb}K`;
        const command = parts.slice(9).join(' ');
        list.push({ pid, user: parts[1], cpu, mem, command, full_command: command, location: startTime });
      }
      list.sort((a, b) => b.cpu - a.cpu);
      setProcesses(list);
      setError(null);
    } catch (e) {
      // 已有数据时静默跳过(等下次 3 秒刷新),只在首次加载零数据时提示
      if (processes.length === 0) {
        setError(String(e));
        setTimeout(() => setError(null), 5000);
      }
    }
    setLoading(false);
  }

  const sorted = [...processes].sort((a, b) => {
    let cmp = 0;
    switch (sortKey) {
      case 'pid':
        cmp = a.pid - b.pid;
        break;
      case 'user':
        cmp = a.user.localeCompare(b.user);
        break;
      case 'mem':
        cmp = parseMemToBytes(a.mem) - parseMemToBytes(b.mem);
        break;
      case 'cpu':
        cmp = a.cpu - b.cpu;
        break;
      case 'command':
        cmp = a.command.localeCompare(b.command);
        break;
    }
    return sortAsc ? cmp : -cmp;
  });

  function handleSort(key: SortKey) {
    if (sortKey === key) {
      setSortAsc(!sortAsc);
    } else {
      setSortKey(key);
      setSortAsc(false);
    }
  }

  async function handleRowClick(pid: number) {
    setSelectedPid(pid);
    selectedPidRef.current = pid;
    try {
      // 一次性获取:status + cmdline + exe + cwd + 环境变量 + 进程树链(向上追溯到 PID 1)
      const output = await invoke<string>('sftp_exec', {
        sessionId,
        command: `cat /proc/${pid}/status 2>/dev/null; echo '===CMDLINE==='; tr '\\0' ' ' < /proc/${pid}/cmdline 2>/dev/null; echo; echo '===EXE==='; readlink /proc/${pid}/exe 2>/dev/null; echo '===CWD==='; readlink /proc/${pid}/cwd 2>/dev/null; echo '===ENV==='; tr '\\0' '\\n' < /proc/${pid}/environ 2>/dev/null; echo '===LSTART==='; ps -p ${pid} -o lstart= 2>/dev/null; echo '===TREE==='; p=${pid}; while [ "$p" != "1" ] && [ "$p" != "0" ] && [ -n "$p" ]; do pp=$(awk '{print $4}' /proc/$p/stat 2>/dev/null); name=$(awk '{gsub(/[()]/, "", $2); print $2}' /proc/$p/stat 2>/dev/null); cmd=$(tr '\\0' ' ' < /proc/$p/cmdline 2>/dev/null); echo "$p|$name|$cmd"; p=$pp; done; echo "1|systemd|/sbin/init"`,
      });
      const sections = output.split(/===\w+===/);
      const status = sections[0] || '';
      const cmdline = (sections[1] || '').trim();
      const exe = (sections[2] || '').trim();
      const cwd = (sections[3] || '').trim();
      const env = (sections[4] || '').trim();
      const lstart = (sections[5] || '').trim();
      const treeText = (sections[6] || '').trim();

      const get = (key: string) => {
        const m = status.match(new RegExp(`^${key}:\\s*(.+)$`, 'm'));
        return m ? m[1].trim() : '';
      };
      const rssKb = parseInt(get('VmRSS')) || 0;
      const mem = rssKb >= 1048576 ? `${(rssKb/1048576).toFixed(1)}G` : rssKb >= 1024 ? `${(rssKb/1024).toFixed(1)}M` : `${rssKb}K`;

      // 解析进程树链: "PID|NAME|CMDLINE" 每行一个祖先
      const ancestors = treeText.split('\n').filter(Boolean).map(line => {
        const [p, name, ...rest] = line.split('|');
        return { pid: parseInt(p) || 0, name: name || '', cmdline: rest.join('|').trim() };
      });

      setDetail({
        pid,
        user: get('Uid').split(/\s+/)[0] || '',
        cpu: 0,
        mem,
        command: get('Name'),
        full_command: cmdline || exe,
        location: exe,
        working_dir: cwd,
        start_time: lstart,
        environ: env.split('\n').filter(Boolean).map(line => {
          const eq = line.indexOf('=');
          return eq > 0 ? { key: line.substring(0, eq), value: line.substring(eq + 1) } : { key: line, value: '' };
        }),
        ancestors,
      });
    } catch (e) {
      console.error('Failed to load process detail:', e);
    }
  }

  return (
    <div className="flex flex-col h-full bg-surface text-gray-200 text-sm">
      {/* Toolbar */}
      <div className="flex items-center gap-2 px-3 py-1 border-b border-surface-border bg-surface-light">
        <span className="text-gray-400">
          进程列表 - {hostLabel}
          {!loading && ` (${processes.length})`}
        </span>
        <div className="flex-1" />
        {loading && <span className="text-xs text-gray-500">加载中...</span>}
        <button
          onClick={loadProcesses}
          disabled={loading}
          className="text-xs text-gray-400 hover:text-white px-2 py-0.5 border border-surface-border rounded disabled:opacity-50"
        >
          刷新
        </button>
      </div>

      {/* Error */}
      {error && (
        <div className="px-3 py-1 text-xs text-accent-red bg-accent-red/10 border-b border-accent-red/30">
          {error}
        </div>
      )}

      {/* Process table */}
      <div className="flex-1 overflow-auto">
        <table className="w-full text-xs table-fixed">
          <thead className="sticky top-0 bg-surface-light border-b border-surface-border">
            <tr>
              <SortHeader label="PID" sortKey="pid" current={sortKey} asc={sortAsc} onClick={handleSort} width="w-[70px]" />
              <SortHeader label="用户" sortKey="user" current={sortKey} asc={sortAsc} onClick={handleSort} width="w-[70px]" />
              <SortHeader label="内存" sortKey="mem" current={sortKey} asc={sortAsc} onClick={handleSort} width="w-[70px]" align="text-right" />
              <SortHeader label="CPU" sortKey="cpu" current={sortKey} asc={sortAsc} onClick={handleSort} width="w-[55px]" align="text-right" />
              <th className="text-left px-2 py-1 text-gray-400 font-normal" style={{ width: '40%' }}>
                名称 | 命令行
              </th>
              <th className="text-left px-2 py-1 text-gray-400 font-normal" style={{ width: '20%' }}>
                启动时间
              </th>
            </tr>
          </thead>
          <tbody>
            {sorted.map((p) => (
              <tr
                key={p.pid}
                onClick={() => handleRowClick(p.pid)}
                className={`cursor-pointer hover:bg-surface-lighter ${
                  selectedPid === p.pid ? 'bg-accent-cyan/10' : ''
                }`}
              >
                <td className="px-2 py-0.5 text-gray-400">{p.pid}</td>
                <td className="px-2 py-0.5">{p.user}</td>
                <td className="px-2 py-0.5 text-right">{p.mem}</td>
                <td className="px-2 py-0.5 text-right">{p.cpu.toFixed(1)}</td>
                <td className="px-2 py-0.5 truncate max-w-md">
                  {p.full_command || p.command}
                </td>
                <td className="px-2 py-0.5 truncate text-gray-500">
                  {p.location}
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>

      {/* Detail panel at bottom */}
      {detail && (
        <div className="border-t border-surface-border bg-surface-light">
          <ProcessDetailPanel
            detail={detail}
            onClose={() => {
              setDetail(null);
              setSelectedPid(null);
              selectedPidRef.current = null;
            }}
          />
        </div>
      )}
    </div>
  );
}
