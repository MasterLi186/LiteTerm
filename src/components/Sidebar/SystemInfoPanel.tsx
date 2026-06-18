import { useEffect, useState, useRef } from 'react';
import { listen } from '@tauri-apps/api/event';
import type { MonitorData } from '../../types';

function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes}B`;
  if (bytes < 1048576) return `${(bytes / 1024).toFixed(0)}K`;
  if (bytes < 1073741824) return `${(bytes / 1048576).toFixed(1)}M`;
  return `${(bytes / 1073741824).toFixed(1)}G`;
}

/* ── Gauge Bar ── */
function GaugeBar({ label, value, detail, color }: {
  label: string; value: number; detail: string; color?: string;
}) {
  const barColor = color || (value > 90 ? '#f85149' : value > 70 ? '#d29922' : '#00d4ff');
  const glowColor = value > 90 ? 'rgba(248,81,73,0.3)' : value > 70 ? 'rgba(210,153,34,0.2)' : 'rgba(0,212,255,0.15)';
  return (
    <div style={{ padding: '8px 12px', background: '#0d1117', borderRadius: '6px', marginBottom: '6px' }}>
      <div style={{ display: 'flex', justifyContent: 'space-between', marginBottom: '5px', alignItems: 'baseline' }}>
        <span style={{ color: '#8b949e', fontSize: '11px', letterSpacing: '0.5px' }}>{label}</span>
        <span style={{ color: '#e6edf3', fontSize: '12px', fontWeight: 500, fontVariantNumeric: 'tabular-nums' }}>{detail}</span>
      </div>
      <div style={{ height: '4px', background: '#21262d', borderRadius: '2px', overflow: 'hidden' }}>
        <div style={{
          height: '100%', borderRadius: '2px',
          width: `${Math.min(value, 100)}%`,
          background: `linear-gradient(90deg, ${barColor}88, ${barColor})`,
          boxShadow: `0 0 8px ${glowColor}`,
          transition: 'width 0.6s ease',
        }} />
      </div>
    </div>
  );
}

/* ── Mini Area Chart with gradient fill ── */
function MiniChart({ data, color, height = 40 }: { data: number[]; color: string; height?: number }) {
  if (data.length < 2) return <div style={{ height, background: '#0d1117', borderRadius: '4px' }} />;
  const max = Math.max(...data, 1);
  const w = 200;
  const pts = data.map((v, i) => `${(i / (data.length - 1)) * w},${height - (v / max) * (height - 4)}`).join(' ');
  const fill = `0,${height} ${pts} ${w},${height}`;
  const id = `grad-${color.replace('#', '')}`;
  return (
    <svg viewBox={`0 0 ${w} ${height}`} style={{ width: '100%', height, display: 'block' }} preserveAspectRatio="none">
      <defs>
        <linearGradient id={id} x1="0" y1="0" x2="0" y2="1">
          <stop offset="0%" stopColor={color} stopOpacity="0.35" />
          <stop offset="100%" stopColor={color} stopOpacity="0.02" />
        </linearGradient>
      </defs>
      <polygon points={fill} fill={`url(#${id})`} />
      <polyline points={pts} fill="none" stroke={color} strokeWidth="1.5" strokeLinejoin="round" />
    </svg>
  );
}

/* ── Section Card wrapper ── */
function Card({ children, title, extra }: { children: React.ReactNode; title?: string; extra?: React.ReactNode }) {
  return (
    <div style={{ margin: '8px', background: '#161b22', borderRadius: '8px', border: '1px solid #21262d', overflow: 'hidden' }}>
      {title && (
        <div style={{
          display: 'flex', justifyContent: 'space-between', alignItems: 'center',
          padding: '8px 12px', borderBottom: '1px solid #21262d',
          background: 'linear-gradient(180deg, rgba(255,255,255,0.03) 0%, transparent 100%)',
        }}>
          <span style={{ color: '#8b949e', fontSize: '11px', fontWeight: 600, letterSpacing: '0.8px', textTransform: 'uppercase' }}>{title}</span>
          {extra}
        </div>
      )}
      {children}
    </div>
  );
}

type ProcessTab = 'mem' | 'cpu' | 'cmd';

interface Props {
  sessionId: string;
  hostIp?: string;
  onOpenProcessManager?: () => void;
}

export function SystemInfoPanel({ sessionId, hostIp, onOpenProcessManager }: Props) {
  const [data, setData] = useState<MonitorData | null>(null);
  const [processTab, setProcessTab] = useState<ProcessTab>('cpu');
  const [selectedIface, setSelectedIface] = useState<string | null>(null);
  const netRxHistory = useRef<number[]>([]);
  const netTxHistory = useRef<number[]>([]);

  useEffect(() => {
    const unlisten = listen<MonitorData>('monitor-data', (event) => {
      if (event.payload.session_id === sessionId) {
        setData(event.payload);
        const iface = selectedIface || event.payload.net_interface;
        const ifaceData = event.payload.net_per_iface?.find(n => n.name === iface);
        const rx = ifaceData ? ifaceData.rx_rate : event.payload.net_rx_rate;
        const tx = ifaceData ? ifaceData.tx_rate : event.payload.net_tx_rate;
        netRxHistory.current = [...netRxHistory.current.slice(-59), rx];
        netTxHistory.current = [...netTxHistory.current.slice(-59), tx];
      }
    });
    return () => { unlisten.then((fn) => fn()); };
  }, [sessionId, selectedIface]);

  if (!data) {
    return (
      <div style={{ padding: '24px 16px', textAlign: 'center' }}>
        <div style={{ color: '#00d4ff', fontSize: '18px', marginBottom: '8px' }}>◎</div>
        <div style={{ color: '#8b949e', fontSize: '12px' }}>正在采集系统信息...</div>
      </div>
    );
  }

  function memToBytes(s: string): number {
    const n = parseFloat(s);
    if (isNaN(n)) return 0;
    if (s.endsWith('G')) return n * 1073741824;
    if (s.endsWith('M')) return n * 1048576;
    if (s.endsWith('K')) return n * 1024;
    return n;
  }

  const sortedProcesses = [...data.processes].sort((a, b) => {
    if (processTab === 'mem') return memToBytes(b.mem) - memToBytes(a.mem);
    if (processTab === 'cpu') return b.cpu - a.cpu;
    return a.command.localeCompare(b.command);
  });

  const tabLabels: Record<ProcessTab, string> = { mem: '内存', cpu: 'CPU', cmd: '命令' };

  return (
    <div style={{ fontSize: '12px' }}>

      {/* ── Status ── */}
      <div style={{ padding: '10px 16px', display: 'flex', alignItems: 'center', gap: '8px', borderBottom: '1px solid #21262d' }}>
        <span style={{ width: '8px', height: '8px', borderRadius: '50%', background: sessionId === 'local' ? '#58a6ff' : '#3fb950', boxShadow: sessionId === 'local' ? '0 0 6px rgba(88,166,255,0.5)' : '0 0 6px rgba(63,185,80,0.5)', display: 'inline-block' }} />
        <span style={{ color: '#e6edf3', fontWeight: 500 }}>{sessionId === 'local' ? '本机' : '已连接'}</span>
        {hostIp && <span style={{ color: '#484f58', marginLeft: 'auto', fontSize: '11px' }}>{hostIp}</span>}
      </div>

      {/* ── Uptime & Load ── */}
      <Card>
        <div style={{ padding: '10px 12px', display: 'flex', gap: '16px' }}>
          <div style={{ flex: 1 }}>
            <div style={{ color: '#484f58', fontSize: '10px', marginBottom: '2px' }}>运行时间</div>
            <div style={{ color: '#e6edf3', fontSize: '12px', fontWeight: 500 }}>{data.uptime_text}</div>
          </div>
          <div style={{ flex: 1 }}>
            <div style={{ color: '#484f58', fontSize: '10px', marginBottom: '2px' }}>系统负载</div>
            <div style={{ color: '#e6edf3', fontSize: '12px', fontWeight: 500, fontVariantNumeric: 'tabular-nums' }}>{data.load_text}</div>
          </div>
        </div>
      </Card>

      {/* ── Gauges ── */}
      <Card title="资源">
        <div style={{ padding: '8px 6px 4px' }}>
          <GaugeBar label={data.cpu_info ? `CPU (${data.cpu_info})` : 'CPU'} value={data.cpu_percent} detail={`${data.cpu_percent.toFixed(1)}%`} />
          <GaugeBar label="内存" value={data.memory_used_percent} detail={data.memory_text} />
          <GaugeBar label="交换" value={data.swap_percent} detail={data.swap_text} />
        </div>
      </Card>

      {/* ── Process List ── */}
      <Card title="进程" extra={
        <div style={{ display: 'flex', gap: '2px' }}>
          {(['mem', 'cpu', 'cmd'] as ProcessTab[]).map((tab) => (
            <button
              key={tab}
              onClick={() => setProcessTab(tab)}
              style={{
                padding: '2px 10px', borderRadius: '4px', border: 'none', cursor: 'pointer',
                fontSize: '10px', fontWeight: 500, transition: 'all 0.2s',
                background: processTab === tab ? 'rgba(0,212,255,0.15)' : 'transparent',
                color: processTab === tab ? '#00d4ff' : '#484f58',
              }}
            >
              {tabLabels[tab]}
            </button>
          ))}
        </div>
      }>
        {sortedProcesses.slice(0, 8).map((p, i) => (
          <div
            key={i}
            onClick={() => onOpenProcessManager?.()}
            style={{
              display: 'flex', alignItems: 'center', gap: '6px',
              padding: '7px 12px', cursor: 'pointer',
              borderBottom: i < 7 ? '1px solid #21262d' : 'none',
              background: i % 2 === 1 ? 'rgba(255,255,255,0.015)' : 'transparent',
              transition: 'background 0.15s',
            }}
            onMouseEnter={e => (e.currentTarget.style.background = 'rgba(0,212,255,0.06)')}
            onMouseLeave={e => (e.currentTarget.style.background = i % 2 === 1 ? 'rgba(255,255,255,0.015)' : 'transparent')}
          >
            <span style={{ width: '44px', textAlign: 'right', color: '#8b949e', flexShrink: 0, fontVariantNumeric: 'tabular-nums', fontSize: '11px' }}>{p.mem}</span>
            <span style={{
              width: '46px', flexShrink: 0, fontSize: '11px', position: 'relative',
              textAlign: 'right', color: '#e6edf3', fontVariantNumeric: 'tabular-nums',
              background: '#21262d', borderRadius: '3px', padding: '1px 4px', overflow: 'hidden',
            }}>
              <span style={{
                position: 'absolute', left: 0, top: 0, bottom: 0,
                width: `${Math.min(p.cpu, 100)}%`,
                background: p.cpu > 80 ? 'rgba(248,81,73,0.4)' : p.cpu > 50 ? 'rgba(210,153,34,0.4)' : 'rgba(63,185,80,0.35)',
                borderRadius: '3px', transition: 'width 0.5s',
              }} />
              <span style={{ position: 'relative' }}>{p.cpu.toFixed(1)}%</span>
            </span>
            <span style={{ flex: 1, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap', color: '#c9d1d9', fontSize: '11px', marginLeft: '4px' }}>{p.command}</span>
          </div>
        ))}
      </Card>

      {/* ── Network ── */}
      <Card title="网络" extra={
        <select
          value={selectedIface || data.net_interface}
          onChange={(e) => { setSelectedIface(e.target.value); netRxHistory.current = []; netTxHistory.current = []; }}
          style={{
            background: '#21262d', border: '1px solid #30363d', borderRadius: '4px',
            color: '#8b949e', fontSize: '10px', padding: '1px 4px', outline: 'none',
            cursor: 'pointer',
          }}
        >
          {(data.net_interfaces || [data.net_interface]).map((iface) => (
            <option key={iface} value={iface}>{iface}</option>
          ))}
        </select>
      }>
        <div style={{ padding: '8px 12px 4px' }}>
          <div style={{ display: 'flex', justifyContent: 'space-between', marginBottom: '6px' }}>
            {(() => {
              const iface = selectedIface || data.net_interface;
              const ifaceData = data.net_per_iface?.find(n => n.name === iface);
              const tx = ifaceData ? ifaceData.tx_rate : data.net_tx_rate;
              const rx = ifaceData ? ifaceData.rx_rate : data.net_rx_rate;
              return (<>
                <span style={{ color: '#3fb950', fontSize: '11px' }}>↑ {formatBytes(tx)}/s</span>
                <span style={{ color: '#58a6ff', fontSize: '11px' }}>↓ {formatBytes(rx)}/s</span>
              </>);
            })()}
          </div>
          <div style={{ background: '#0d1117', borderRadius: '4px', overflow: 'hidden', padding: '4px 0' }}>
            <MiniChart data={netTxHistory.current} color="#3fb950" height={28} />
            <MiniChart data={netRxHistory.current} color="#58a6ff" height={28} />
          </div>
        </div>
      </Card>

      {/* ── Disk ── */}
      <Card title="磁盘">
        <div>
          <div style={{
            display: 'flex', padding: '6px 12px', borderBottom: '1px solid #21262d',
            color: '#484f58', fontSize: '10px', fontWeight: 600, letterSpacing: '0.5px',
          }}>
            <span style={{ flex: 1 }}>挂载点</span>
            <span style={{ width: '110px', textAlign: 'right' }}>可用/总量</span>
          </div>
          {data.disk_items.map((d, i) => (
            <div key={i} style={{
              display: 'flex', alignItems: 'center', padding: '5px 12px',
              borderBottom: i < data.disk_items.length - 1 ? '1px solid #21262d' : 'none',
              background: i % 2 === 1 ? 'rgba(255,255,255,0.015)' : 'transparent',
            }}>
              <span style={{ flex: 1, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap', color: '#c9d1d9', fontSize: '11px' }}>{d.mount}</span>
              <span style={{
                width: '36px', textAlign: 'right', flexShrink: 0, fontSize: '10px', fontVariantNumeric: 'tabular-nums',
                color: d.percent > 90 ? '#f85149' : d.percent > 70 ? '#d29922' : '#8b949e',
                fontWeight: d.percent > 90 ? 600 : 400,
              }}>{d.percent}%</span>
              <span style={{ width: '90px', textAlign: 'right', color: '#8b949e', fontSize: '11px', flexShrink: 0, fontVariantNumeric: 'tabular-nums' }}>
                {d.avail}/{d.size}
              </span>
            </div>
          ))}
        </div>
      </Card>
    </div>
  );
}
