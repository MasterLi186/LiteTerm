import { useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import type { ConnectionStore, ShellInfo, SerialPortInfo } from '../types';

interface Props {
  onClose: () => void;
  onOpenShell: (shellPath: string, shellName: string) => void;
  onConnectSSH: (groupId: string, hostId: string) => void;
  onNewSSH: () => void;
  onOpenSerial: (device: string, baudRate: number, name: string) => void;
  connections: ConnectionStore;
}

const BAUD_RATES = [9600, 19200, 38400, 57600, 115200, 230400, 460800, 921600];

export function NewTabSelector({ onClose, onOpenShell, onConnectSSH, onNewSSH, onOpenSerial, connections }: Props) {
  const [shells, setShells] = useState<ShellInfo[]>([]);
  const [serialPorts, setSerialPorts] = useState<SerialPortInfo[]>([]);
  const [baudRates, setBaudRates] = useState<Record<string, number>>({});
  const [loadingSerial, setLoadingSerial] = useState(false);

  useEffect(() => {
    invoke<ShellInfo[]>('list_shells').then(setShells).catch(() => {});
    refreshSerialPorts();
  }, []);

  function refreshSerialPorts() {
    setLoadingSerial(true);
    invoke<SerialPortInfo[]>('list_serial_ports')
      .then((ports) => {
        setSerialPorts(ports);
        // Init default baud rate for new ports
        const defaults: Record<string, number> = { ...baudRates };
        ports.forEach((p) => {
          if (!defaults[p.path]) defaults[p.path] = 115200;
        });
        setBaudRates(defaults);
      })
      .catch(() => setSerialPorts([]))
      .finally(() => setLoadingSerial(false));
  }

  const groupEntries = Object.entries(connections.groups);

  return (
    <div className="fixed inset-0 bg-black/60 flex items-center justify-center z-50" onClick={onClose}>
      <div
        className="bg-surface-light border border-surface-border rounded-lg shadow-xl w-[520px] max-h-[80vh] overflow-y-auto"
        onClick={(e) => e.stopPropagation()}
      >
        {/* Header */}
        <div className="flex items-center justify-between px-4 py-3 border-b border-surface-border">
          <h2 className="text-sm font-semibold text-gray-200">新建标签页</h2>
          <button onClick={onClose} className="text-gray-500 hover:text-white text-lg leading-none">×</button>
        </div>

        <div className="p-4 space-y-5">
          {/* Shell section */}
          <div>
            <div className="text-xs font-semibold text-gray-400 mb-2 flex items-center gap-1.5">
              <span className="text-accent-cyan">$</span>
              <span>Shell 环境</span>
            </div>
            <div className="flex flex-wrap gap-2">
              {shells.map((shell) => (
                <button
                  key={shell.path}
                  onClick={() => { onOpenShell(shell.path, shell.name); onClose(); }}
                  className="px-3 py-1.5 text-sm bg-surface border border-surface-border rounded hover:border-accent-cyan hover:text-accent-cyan text-gray-300 transition-colors"
                >
                  {shell.name}
                </button>
              ))}
              {shells.length === 0 && (
                <span className="text-xs text-gray-500">未检测到可用 Shell</span>
              )}
            </div>
          </div>

          {/* SSH section */}
          <div>
            <div className="text-xs font-semibold text-gray-400 mb-2 flex items-center gap-1.5">
              <span className="text-accent-green">@</span>
              <span>SSH 连接</span>
            </div>
            {groupEntries.length === 0 ? (
              <div className="text-xs text-gray-500 mb-2">暂无保存的连接</div>
            ) : (
              <div className="space-y-1 mb-2">
                {groupEntries.map(([groupId, group]) => (
                  <div key={groupId}>
                    <div className="text-xs text-gray-500 flex items-center gap-1 mb-0.5 px-1">
                      <span
                        className="w-2 h-2 rounded-full inline-block flex-shrink-0"
                        style={{ backgroundColor: group.color }}
                      />
                      {group.label}
                    </div>
                    {Object.entries(group.hosts).map(([hostId, host]) => (
                      <button
                        key={hostId}
                        onClick={() => { onConnectSSH(groupId, hostId); onClose(); }}
                        className="w-full text-left pl-5 pr-3 py-1 text-sm text-gray-300 hover:bg-surface-lighter rounded flex items-center justify-between group"
                      >
                        <span className="truncate">{host.label}</span>
                        <span className="text-xs text-gray-500 group-hover:text-accent-green flex-shrink-0 ml-2">
                          {host.host}:{host.port}
                        </span>
                      </button>
                    ))}
                  </div>
                ))}
              </div>
            )}
            <button
              onClick={() => { onNewSSH(); onClose(); }}
              className="text-xs text-accent-cyan hover:text-white transition-colors"
            >
              + 新建 SSH 连接
            </button>
          </div>

          {/* Serial section */}
          <div>
            <div className="text-xs font-semibold text-gray-400 mb-2 flex items-center justify-between">
              <span className="flex items-center gap-1.5">
                <span className="text-accent-yellow">~</span>
                <span>串口设备</span>
              </span>
              <button
                onClick={refreshSerialPorts}
                className="text-[10px] text-gray-500 hover:text-accent-cyan transition-colors"
              >
                {loadingSerial ? '扫描中...' : '刷新'}
              </button>
            </div>
            {serialPorts.length === 0 ? (
              <div className="text-xs text-gray-500">未检测到串口设备</div>
            ) : (
              <div className="space-y-2">
                {serialPorts.map((port) => (
                  <div
                    key={port.path}
                    className="flex items-center gap-2 bg-surface border border-surface-border rounded px-3 py-2"
                  >
                    <div className="flex-1 min-w-0">
                      <div className="text-sm text-gray-200 truncate">{port.path}</div>
                      <div className="text-[10px] text-gray-500">{port.port_type}</div>
                    </div>
                    <select
                      value={baudRates[port.path] || 115200}
                      onChange={(e) => setBaudRates({ ...baudRates, [port.path]: Number(e.target.value) })}
                      className="bg-surface-light border border-surface-border rounded px-2 py-1 text-xs text-gray-300 outline-none focus:border-accent-cyan"
                    >
                      {BAUD_RATES.map((rate) => (
                        <option key={rate} value={rate}>
                          {rate}
                        </option>
                      ))}
                    </select>
                    <button
                      onClick={() => {
                        onOpenSerial(port.path, baudRates[port.path] || 115200, port.name);
                        onClose();
                      }}
                      className="px-2 py-1 text-xs bg-accent-cyan/20 text-accent-cyan rounded hover:bg-accent-cyan/30 transition-colors flex-shrink-0"
                    >
                      连接
                    </button>
                  </div>
                ))}
              </div>
            )}
          </div>
        </div>
      </div>
    </div>
  );
}
