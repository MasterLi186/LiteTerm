import { useEffect, useState } from 'react';
import { listen } from '@tauri-apps/api/event';
import type { MonitorData } from '../../types';

function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes}B`;
  if (bytes < 1048576) return `${(bytes / 1024).toFixed(1)}K`;
  if (bytes < 1073741824) return `${(bytes / 1048576).toFixed(1)}M`;
  return `${(bytes / 1073741824).toFixed(1)}G`;
}

function MetricBar({ label, value, unit, text }: {
  label: string;
  value: number;
  unit: string;
  text?: string;
}) {
  const color = value > 90 ? 'bg-accent-red' : value > 70 ? 'bg-accent-orange' : 'bg-accent-green';
  return (
    <div>
      <div className="flex justify-between text-gray-300">
        <span>{label}</span>
        <span>{text || `${value.toFixed(1)}${unit}`}</span>
      </div>
      <div className="h-1.5 bg-surface rounded-full mt-0.5">
        <div
          className={`h-full rounded-full ${color} transition-all duration-500`}
          style={{ width: `${Math.min(value, 100)}%` }}
        />
      </div>
    </div>
  );
}

interface Props {
  sessionId: string | null;
}

export function MonitorPanel({ sessionId }: Props) {
  const [data, setData] = useState<MonitorData | null>(null);

  useEffect(() => {
    if (!sessionId) {
      setData(null);
      return;
    }

    const unlisten = listen<MonitorData>('monitor-data', (event) => {
      if (event.payload.session_id === sessionId) {
        setData(event.payload);
      }
    });

    return () => {
      unlisten.then((fn) => fn());
    };
  }, [sessionId]);

  if (!sessionId) {
    return <div className="px-2 pb-2 text-gray-500 text-xs">连接后显示监控</div>;
  }

  if (!data) {
    return <div className="px-2 pb-2 text-gray-500 text-xs">采集中...</div>;
  }

  return (
    <div className="p-2 space-y-2 text-xs">
      <MetricBar label="CPU" value={data.cpu_percent} unit="%" />
      <MetricBar
        label="内存"
        value={data.memory_used_percent}
        unit="%"
        text={data.memory_text}
      />
      <div className="text-gray-400">负载: {data.load_text}</div>
      {data.disk_items.map((d) => (
        <MetricBar key={d.mount} label={d.mount} value={d.percent} unit="%" />
      ))}
      <div className="text-gray-400">
        网络: ↑{formatBytes(data.net_tx_rate)}/s ↓{formatBytes(data.net_rx_rate)}/s
      </div>
    </div>
  );
}
