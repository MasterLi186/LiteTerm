import { useEffect, useState } from 'react';
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

export function ProcessTable({ sshParams, hostLabel }: Props) {
  const [processes, setProcesses] = useState<ProcessDetail[]>([]);
  const [sortKey, setSortKey] = useState<SortKey>('cpu');
  const [sortAsc, setSortAsc] = useState(false);
  const [selectedPid, setSelectedPid] = useState<number | null>(null);
  const [detail, setDetail] = useState<ProcessFullDetail | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  // Auto-refresh every 3 seconds
  useEffect(() => {
    loadProcesses();
    const timer = setInterval(loadProcesses, 3000);
    return () => clearInterval(timer);
  }, []);

  async function loadProcesses() {
    // Only show loading spinner on first load, not on refresh
    if (processes.length === 0) setLoading(true);
    try {
      const list = await invoke<ProcessDetail[]>('get_process_list', {
        host: sshParams.host,
        port: sshParams.port,
        user: sshParams.user,
        password: sshParams.password,
        authMethod: sshParams.authMethod,
        keyPath: sshParams.keyPath,
      });
      setProcesses(list);
      setError(null);
    } catch (e) {
      console.error('Failed to load processes:', e);
      if (processes.length === 0) setError(String(e));
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
    try {
      const d = await invoke<ProcessFullDetail>('get_process_detail', {
        host: sshParams.host,
        port: sshParams.port,
        user: sshParams.user,
        password: sshParams.password,
        authMethod: sshParams.authMethod,
        keyPath: sshParams.keyPath,
        pid,
      });
      setDetail(d);
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
                位置
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
            }}
          />
        </div>
      )}
    </div>
  );
}
