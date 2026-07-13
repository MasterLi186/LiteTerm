import { useEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { log } from '../utils/logger';

interface NetConnection {
  state: string;
  local: string;
  peer: string;
  process: string;
  pid: string;
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
  initialIface?: string;
}

const REFRESH_OPTIONS = [
  { label: '1 秒', value: 1000 },
  { label: '2 秒', value: 2000 },
  { label: '3 秒', value: 3000 },
  { label: '5 秒', value: 5000 },
  { label: '10 秒', value: 10000 },
  { label: '30 秒', value: 30000 },
];

function formatBytes(bytes: number): string {
  if (bytes >= 1073741824) return `${(bytes / 1073741824).toFixed(1)}G`;
  if (bytes >= 1048576) return `${(bytes / 1048576).toFixed(1)}M`;
  if (bytes >= 1024) return `${(bytes / 1024).toFixed(1)}K`;
  return `${bytes}B`;
}

type SortKey = 'state' | 'local' | 'peer' | 'process' | 'pid';

export function NetworkDetail({ sessionId, hostLabel, isLocal, initialIface }: Props) {
  const [connections, setConnections] = useState<NetConnection[]>([]);
  const [ifaces, setIfaces] = useState<string[]>([]);
  const [selectedIface, setSelectedIface] = useState(initialIface || '');
  const selectedIfaceRef = useRef(initialIface || '');
  const ifaceInitialized = useRef(!!initialIface);
  const [ifaceIps, setIfaceIps] = useState<Record<string, string>>({});
  const [rxRate, setRxRate] = useState(0);
  const [txRate, setTxRate] = useState(0);
  const [refreshInterval, setRefreshInterval] = useState(3000);
  const [loading, setLoading] = useState(true);
  const [sortKey, setSortKey] = useState<SortKey>('process');
  const [sortAsc, setSortAsc] = useState(true);
  const hasDataRef = useRef(false);

  function selectIface(iface: string) {
    setSelectedIface(iface);
    selectedIfaceRef.current = iface;
    ifaceInitialized.current = true;
  }

  // 匹配本组件对应的 session（本机="local"，SSH=sessionId）
  const monitorSessionId = isLocal ? 'local' : sessionId;

  // 从 monitor 事件获取网卡列表和速率（按 session_id 过滤）
  useEffect(() => {
    const unlisten = listen<any>('monitor-data', (event) => {
      const p = event.payload;
      if (p.session_id !== monitorSessionId) return;
      if (p.net_interfaces?.length > 0) {
        setIfaces(p.net_interfaces);
        if (!ifaceInitialized.current && p.net_interface) {
          selectIface(p.net_interface);
        }
      }
      const iface = selectedIfaceRef.current;
      if (!iface) return;
      const ifaceData = p.net_per_iface?.find((n: any) => n.name === iface);
      if (ifaceData) {
        setRxRate(ifaceData.rx_rate);
        setTxRate(ifaceData.tx_rate);
      }
    });
    return () => { unlisten.then(fn => fn()); };
  }, []);

  // 定时刷新连接列表
  useEffect(() => {
    loadConnections();
    const timer = setInterval(loadConnections, refreshInterval);
    return () => clearInterval(timer);
  }, [selectedIface, refreshInterval]);

  async function execCmd(command: string): Promise<string> {
    if (isLocal) {
      // 本机直接用 shell 执行
      return invoke<string>('run_shell_command', { command });
    }
    return invoke<string>('sftp_exec', { sessionId, command });
  }

  async function loadConnections() {
    if (!hasDataRef.current) setLoading(true);
    try {
      const cmd = `echo '===IP==='; ip -4 addr show 2>/dev/null | awk '/inet /{print $NF,$2}'; echo '===SS==='; ss -tnp 2>/dev/null | tail -n +2`;
      const output = await execCmd(cmd);

      const ipSection = output.split('===SS===')[0]?.split('===IP===')[1] || '';
      const ssSection = output.split('===SS===')[1] || '';

      const ipMap: Record<string, string> = {};
      for (const line of ipSection.trim().split('\n')) {
        const parts = line.trim().split(/\s+/);
        if (parts.length >= 2) {
          ipMap[parts[0]] = parts[1].split('/')[0];
        }
      }
      setIfaceIps(ipMap);

      const conns: NetConnection[] = [];
      for (const line of ssSection.trim().split('\n')) {
        if (!line.trim()) continue;
        const parts = line.trim().split(/\s+/);
        if (parts.length < 5) continue;
        const procMatch = line.match(/users:\(\("([^"]*)",pid=(\d+)/);
        conns.push({
          state: parts[0],
          local: parts[3],
          peer: parts[4],
          process: procMatch ? procMatch[1] : '',
          pid: procMatch ? procMatch[2] : '',
        });
      }

      const iface = selectedIfaceRef.current;
      const ifaceIp = ipMap[iface];
      const filtered = iface && ifaceIp
        ? conns.filter(c => c.local.replace(/:\d+$/, '') === ifaceIp)
        : conns;

      setConnections(filtered);
      hasDataRef.current = filtered.length > 0 || conns.length > 0;
    } catch (e) {
      log('NET', `加载连接失败: ${e}`);
    }
    setLoading(false);
  }

  function handleSort(key: SortKey) {
    if (key === sortKey) setSortAsc(!sortAsc);
    else { setSortKey(key); setSortAsc(true); }
  }

  const sorted = [...connections].sort((a, b) => {
    let cmp = 0;
    switch (sortKey) {
      case 'state': cmp = a.state.localeCompare(b.state); break;
      case 'local': cmp = a.local.localeCompare(b.local); break;
      case 'peer': cmp = a.peer.localeCompare(b.peer); break;
      case 'process': cmp = a.process.localeCompare(b.process); break;
      case 'pid': cmp = parseInt(a.pid || '0') - parseInt(b.pid || '0'); break;
    }
    return sortAsc ? cmp : -cmp;
  });

  const arrow = (key: SortKey) => sortKey === key ? (sortAsc ? ' ^' : ' v') : '';

  return (
    <div className="flex flex-col h-full w-full bg-surface text-gray-200 text-xs">
      {/* 顶栏 */}
      <div className="flex items-center gap-4 px-4 py-2 border-b border-surface-border bg-surface-light flex-shrink-0">
        <div className="flex items-center gap-2">
          <span className="text-gray-400">网卡:</span>
          <select
            value={selectedIface}
            onChange={(e) => selectIface(e.target.value)}
            className="bg-surface border border-surface-border rounded px-2 py-1 text-xs text-gray-300 outline-none focus:border-accent-cyan"
          >
            {ifaces.map(iface => (
              <option key={iface} value={iface}>{iface}{ifaceIps[iface] ? ` (${ifaceIps[iface]})` : ''}</option>
            ))}
          </select>
        </div>

        <div className="flex items-center gap-4">
          <span className="text-accent-green">↑ {formatBytes(txRate)}/s</span>
          <span className="text-accent-cyan">↓ {formatBytes(rxRate)}/s</span>
        </div>

        <div className="flex items-center gap-2 ml-auto">
          <span className="text-gray-400">刷新:</span>
          <select
            value={refreshInterval}
            onChange={(e) => setRefreshInterval(Number(e.target.value))}
            className="bg-surface border border-surface-border rounded px-2 py-1 text-xs text-gray-300 outline-none focus:border-accent-cyan"
          >
            {REFRESH_OPTIONS.map(opt => (
              <option key={opt.value} value={opt.value}>{opt.label}</option>
            ))}
          </select>
          <span className="text-gray-500">{connections.length} 条连接</span>
        </div>
      </div>

      {/* 连接表格 */}
      <div className="flex-1 overflow-auto min-h-0">
        {loading && !hasDataRef.current ? (
          <div className="flex items-center justify-center h-full text-gray-500">加载中...</div>
        ) : (
          <table className="w-full border-collapse table-fixed">
            <thead className="sticky top-0 bg-surface-light z-10">
              <tr className="text-[11px]">
                <th className="text-left px-3 py-1.5 text-gray-400 font-normal cursor-pointer hover:text-gray-200 select-none w-[60px]" onClick={() => handleSort('state')}>状态{arrow('state')}</th>
                <th className="text-left px-3 py-1.5 text-gray-400 font-normal cursor-pointer hover:text-gray-200 select-none w-[28%]" onClick={() => handleSort('local')}>本地地址{arrow('local')}</th>
                <th className="text-left px-3 py-1.5 text-gray-400 font-normal cursor-pointer hover:text-gray-200 select-none w-[28%]" onClick={() => handleSort('peer')}>远程地址{arrow('peer')}</th>
                <th className="text-left px-3 py-1.5 text-gray-400 font-normal cursor-pointer hover:text-gray-200 select-none w-[70px]" onClick={() => handleSort('pid')}>PID{arrow('pid')}</th>
                <th className="text-left px-3 py-1.5 text-gray-400 font-normal cursor-pointer hover:text-gray-200 select-none" onClick={() => handleSort('process')}>进程{arrow('process')}</th>
              </tr>
            </thead>
            <tbody>
              {sorted.map((conn, i) => (
                <tr key={i} className="hover:bg-surface-lighter border-b border-surface-border/50">
                  <td className="px-3 py-1 truncate">
                    <span className={conn.state === 'ESTAB' ? 'text-accent-green' : conn.state === 'LISTEN' ? 'text-accent-cyan' : 'text-gray-400'}>
                      {conn.state}
                    </span>
                  </td>
                  <td className="px-3 py-1 text-gray-300 font-mono truncate">{conn.local}</td>
                  <td className="px-3 py-1 text-gray-300 font-mono truncate">{conn.peer}</td>
                  <td className="px-3 py-1 text-gray-500 truncate">{conn.pid}</td>
                  <td className="px-3 py-1 text-gray-200 truncate">{conn.process || <span className="text-gray-500">-</span>}</td>
                </tr>
              ))}
              {sorted.length === 0 && (
                <tr>
                  <td colSpan={5} className="text-center py-8 text-gray-500">
                    {selectedIface ? `${selectedIface} 上没有活跃连接` : '没有活跃连接'}
                  </td>
                </tr>
              )}
            </tbody>
          </table>
        )}
      </div>
    </div>
  );
}
