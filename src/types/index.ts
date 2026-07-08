export type SplitNode =
  | { type: 'terminal'; terminalId: string }
  | { type: 'split'; direction: 'horizontal' | 'vertical'; first: SplitNode; second: SplitNode; ratio: number };

export type AuthMethod = 'keyring' | 'key' | 'agent';

export interface HostConfig {
  label: string;
  host: string;
  port: number;
  user: string;
  auth: AuthMethod;
  key_path: string;
  charset: string;
  proxy_jump?: string;
}

export interface GroupConfig {
  label: string;
  color: string;
  hosts: Record<string, HostConfig>;
}

export interface ConnectionStore {
  groups: Record<string, GroupConfig>;
}

export interface Tab {
  id: string;
  label: string;
  type: 'local' | 'ssh' | 'process' | 'serial' | 'recording';
  sshParams?: {
    host: string;
    port: number;
    user: string;
    password: string | null;
    authMethod: string;
    keyPath: string | null;
    proxyJump?: string | null;
  };
  shellPath?: string;
  serialParams?: {
    device: string;
    baudRate: number;
  };
  recordingPath?: string;
  fileBrowserHidden?: boolean;
}

export interface DiskItem {
  mount: string;
  avail: string;
  size: string;
  percent: number;
}

export interface ProcessInfo {
  mem: string;
  cpu: number;
  command: string;
}

export interface MonitorData {
  session_id: string;
  cpu_percent: number;
  memory_used_percent: number;
  memory_text: string;
  swap_text: string;
  swap_percent: number;
  uptime_text: string;
  load_text: string;
  disk_items: DiskItem[];
  net_rx_rate: number;
  net_tx_rate: number;
  net_interface: string;
  net_interfaces: string[];
  net_per_iface: { name: string; rx_rate: number; tx_rate: number }[];
  cpu_info: string;
  processes: ProcessInfo[];
}

export interface FileEntry {
  name: string;
  is_dir: boolean;
  size: number;
  mtime: number;
  permissions: string;
  owner: string;
  group: string;
}

export interface ProcessDetail {
  pid: number;
  user: string;
  mem: string;
  cpu: number;
  command: string;
  full_command: string;
  location: string;
}

export interface EnvVar {
  key: string;
  value: string;
}

export interface ProcessAncestor {
  pid: number;
  name: string;
  cmdline: string;
}

export interface ProcessFullDetail extends ProcessDetail {
  working_dir: string;
  environ: EnvVar[];
  ancestors: ProcessAncestor[];
}

export interface ShellInfo {
  name: string;
  path: string;
}

export interface SerialPortInfo {
  name: string;
  path: string;
  port_type: string;
}
