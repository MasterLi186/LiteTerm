import React from "react";
import ReactDOM from "react-dom/client";
import { invoke } from '@tauri-apps/api/core';
import App from "./App";
import { resolveUiLocale, setUiLocale } from './i18n';
import "./styles/globals.css";

// Block the Tauri webview's built-in right-click menu (back/forward/reload).
// Use the bubbling phase so React's onContextMenu handlers fire first.
document.addEventListener('contextmenu', (e) => {
  e.preventDefault();
});

async function startApp() {
  const systemLocale = await invoke<string | null>('get_system_locale').catch(() => null);
  const locale = resolveUiLocale(systemLocale, ...(navigator.languages ?? []), navigator.language);
  setUiLocale(locale);

  ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
    <React.StrictMode>
      <App />
    </React.StrictMode>,
  );
}

void startApp();
