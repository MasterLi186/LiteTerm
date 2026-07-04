import { invoke } from '@tauri-apps/api/core';

const MAX_LOG_LINES = 500;
const logBuffer: string[] = [];

function timestamp(): string {
  const d = new Date();
  return `${d.getHours().toString().padStart(2, '0')}:${d.getMinutes().toString().padStart(2, '0')}:${d.getSeconds().toString().padStart(2, '0')}.${d.getMilliseconds().toString().padStart(3, '0')}`;
}

export function log(category: string, message: string, data?: unknown) {
  const line = `[${timestamp()}] [${category}] ${message}${data !== undefined ? ' ' + JSON.stringify(data) : ''}`;
  logBuffer.push(line);
  if (logBuffer.length > MAX_LOG_LINES) logBuffer.shift();
  console.log(line);
  // 同步写入 ~/guishell.log(通过后端 app_log）
  invoke('frontend_log', { category, message: message + (data !== undefined ? ' ' + JSON.stringify(data) : '') }).catch(() => {});
}

export function getLogBuffer(): string[] {
  return [...logBuffer];
}

export function getLogText(): string {
  return logBuffer.join('\n');
}
