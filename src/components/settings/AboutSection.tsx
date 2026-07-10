import { useState, useEffect } from 'react';
import { invoke } from '@tauri-apps/api/core';

export function AboutSection() {
  const [info, setInfo] = useState<any>(null);

  useEffect(() => {
    invoke<any>('get_system_info').then(setInfo).catch(() => {});
  }, []);

  return (
    <div className="space-y-6">
      <h2 className="text-lg font-semibold text-gray-200">关于 LiteTerm</h2>
      {info && (
        <div className="space-y-3">
          <div className="flex items-center gap-3">
            <span className="text-2xl">⌨</span>
            <div>
              <div className="text-lg text-gray-200 font-semibold">LiteTerm</div>
              <div className="text-sm text-gray-400">轻量级跨平台 SSH 客户端</div>
            </div>
          </div>
          <div className="bg-surface-light rounded-lg p-4 space-y-2 text-sm">
            <div className="flex justify-between"><span className="text-gray-400">版本</span><span className="text-gray-200">v{info.app_version}</span></div>
            <div className="flex justify-between"><span className="text-gray-400">操作系统</span><span className="text-gray-200">{info.os} ({info.arch})</span></div>
            <div className="flex justify-between"><span className="text-gray-400">主机名</span><span className="text-gray-200">{info.hostname}</span></div>
            <div className="flex justify-between"><span className="text-gray-400">用户</span><span className="text-gray-200">{info.username}</span></div>
          </div>
          <div className="text-xs text-gray-500">基于 Tauri 2 + React + xterm.js 构建</div>
        </div>
      )}
    </div>
  );
}
