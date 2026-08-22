/**
 * CmdBox React 前端入口。
 *
 * 这里只负责把根组件挂载到 Vite 页面；Tauri 系统能力必须通过后续明确的 Typed IPC 接入。
 */
import React from "react";
import ReactDOM from "react-dom/client";
import App from "./app/App";

/** 将 CmdBox 根组件挂载到页面中的唯一根节点。 */
function bootstrapApp() {
  ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
    <React.StrictMode>
      <App />
    </React.StrictMode>,
  );
}

bootstrapApp();
