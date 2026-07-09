import { useEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { ProcessDetailPanel } from './ProcessDetail';
import type { ProcessDetail, ProcessFullDetail } from '../../types';
import { log } from '../../utils/logger';

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
  sessionId?: string;
  sshParams?: {
    host: string;
    port: number;
    user: string;
    password: string | null;
    authMethod: string;
    keyPath: string | null;
  };
  hostLabel: string;
  isLocal?: boolean;
}

export function ProcessTable({ sessionId, sshParams, hostLabel, isLocal }: Props) {
  const [processes, setProcesses] = useState<ProcessDetail[]>([]);
  const [sortKey, setSortKey] = useState<SortKey>('cpu');
  const [sortAsc, setSortAsc] = useState(false);
  const [selectedPid, setSelectedPid] = useState<number | null>(null);
  const selectedPidRef = useRef<number | null>(null);
  const [detail, setDetail] = useState<ProcessFullDetail | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const hasDataRef = useRef(false);
  const [procStats, setProcStats] = useState<{ total: number; running: number; sleeping: number; zombie: number; stopped: number } | null>(null);

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
    if (!hasDataRef.current) setLoading(true);
    try {
      let list: ProcessDetail[] = [];
      if (isLocal) {
        // 本地进程(sysinfo,跨平台)
        const data = await invoke<any[]>('get_local_processes');
        list = (Array.isArray(data) ? data : []).map((p: any) => ({
          pid: p.pid, user: p.user || '', cpu: p.cpu || 0, mem: p.mem || '0K',
          command: p.command || '', full_command: p.full_command || p.command || '',
          location: p.start_time ? new Date(p.start_time * 1000).toLocaleString() : '',
        }));
      } else {
        // 远端 SSH 进程
        const psOutput = await invoke<string>('sftp_exec', {
          sessionId,
          command: "ps -eo pid,user,lstart,pcpu,rss,comm --no-headers 2>/dev/null | awk '$8+0>0.1{print}' | head -100",
        });
        for (const line of psOutput.split('\n')) {
          const parts = line.trim().split(/\s+/);
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
      }
      list.sort((a, b) => b.cpu - a.cpu);
      setProcesses(list);
      hasDataRef.current = list.length > 0;
      setError(null);
      // 进程状态统计(轻量,只读 /proc/stat 一行)
      invoke<string>('sftp_exec', {
        sessionId,
        command: "ps h -eo stat | cut -c1 | sort | uniq -c",
      }).then((out: unknown) => {
        if (typeof out !== 'string') return;
        const lines = (out as string).trim().split('\n');
        let total = 0, running = 0, sleeping = 0, zombie = 0, stopped = 0;
        for (const line of lines) {
          const m = line.trim().match(/^(\d+)\s+(\S)/);
          if (m) {
            const count = parseInt(m[1]);
            total += count;
            if (m[2] === 'R') running = count;
            else if (m[2] === 'S' || m[2] === 'D' || m[2] === 'I') sleeping += count;
            else if (m[2] === 'Z') zombie = count;
            else if (m[2] === 'T' || m[2] === 't') stopped += count;
          }
        }
        setProcStats({ total, running, sleeping, zombie, stopped });
      }).catch(() => {});
    } catch (e) {
      if (!hasDataRef.current) {
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
      if (isLocal) {
        const d = await invoke<any>('get_local_process_detail', { pid });
        setDetail({
          pid, user: d.user || '', cpu: 0, mem: d.mem || '0K',
          command: d.command || '', full_command: d.full_command || '',
          location: d.location || '', working_dir: d.working_dir || '',
          start_time: d.start_time ? new Date(d.start_time * 1000).toLocaleString() : '',
          environ: (d.environ || []).map((line: string) => {
            const eq = line.indexOf('=');
            return eq > 0 ? { key: line.substring(0, eq), value: line.substring(eq + 1) } : { key: line, value: '' };
          }),
          ancestors: (d.ancestors || []).map((a: any) => ({ pid: a.pid || 0, name: a.name || '', cmdline: a.cmdline || '' })),
        });
        return;
      }
      const output = await invoke<string>('sftp_exec', {
        sessionId,
        command: `cat /proc/${pid}/status 2>/dev/null; echo '===CMDLINE==='; tr '\\0' ' ' < /proc/${pid}/cmdline 2>/dev/null; echo; echo '===EXE==='; readlink /proc/${pid}/exe 2>/dev/null; echo '===CWD==='; readlink /proc/${pid}/cwd 2>/dev/null; echo '===ENV==='; tr '\\0' '\\n' < /proc/${pid}/environ 2>/dev/null; echo '===LSTART==='; ps -p ${pid} -o lstart= 2>/dev/null; echo '===TREE==='; p=${pid}; i=0; while [ "$p" != "1" ] && [ "$p" != "0" ] && [ -n "$p" ] && [ $i -lt 50 ]; do i=$((i+1)); pp=$(awk '{print $4}' /proc/$p/stat 2>/dev/null); name=$(awk '{gsub(/[()]/, "", $2); print $2}' /proc/$p/stat 2>/dev/null); cmd=$(tr '\\0' ' ' < /proc/$p/cmdline 2>/dev/null); echo "$p|$name|$cmd"; p=$pp; done; echo "1|systemd|/sbin/init"`,
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
      log('进程', '加载进程详情失败: ' + String(e));
    }
  }

  return (
    <div className="flex flex-col h-full bg-surface text-gray-200 text-sm">
      {/* Toolbar */}
      <div className="flex items-center gap-2 px-3 py-1 border-b border-surface-border bg-surface-light flex-wrap">
        <span className="text-gray-400">
          进程列表 - {hostLabel}
          {!loading && ` (${processes.length})`}
        </span>
        {procStats && (
          <span className="text-[10px] text-gray-500" style={{ fontVariantNumeric: 'tabular-nums' }}>
            共 <span className={procStats.total > 10000 ? 'text-accent-red font-bold' : 'text-gray-300'}>{procStats.total}</span>
            {' '}运行 <span className="text-accent-green">{procStats.running}</span>
            {' '}休眠 <span className="text-gray-400">{procStats.sleeping}</span>
            {procStats.zombie > 0 && <>{' '}僵尸 <span className="text-accent-red font-bold">{procStats.zombie}</span></>}
            {procStats.stopped > 0 && <>{' '}停止 <span className="text-accent-yellow">{procStats.stopped}</span></>}
          </span>
        )}
        {procStats && procStats.total > 10000 && (
          <span className="text-[10px] text-accent-red bg-accent-red/10 px-1.5 py-0.5 rounded">
            ⚠ 进程数异常({procStats.total}),可能存在进程泄漏
          </span>
        )}
        {procStats && procStats.zombie > 100 && (
          <span className="text-[10px] text-accent-red bg-accent-red/10 px-1.5 py-0.5 rounded">
            ⚠ 僵尸进程过多({procStats.zombie}),建议排查父进程
          </span>
        )}
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
