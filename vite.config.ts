import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

/**
 * CmdBox 的 Vite 开发配置。
 *
 * 本文件同时服务 Windows Tauri Dev 与后续 Docker 前端开发，固定端口可以让
 * Tauri 配置和健康检查共享同一个入口。Rust 目录由 Tauri 自己监听，Vite 不重复扫描。
 */

// @ts-expect-error 当前基线不额外引入 Node 类型，只读取 Tauri 官方约定的环境变量。
const host = process.env.TAURI_DEV_HOST;

/** 创建适用于 Tauri 的 Vite 配置。 */
export default defineConfig(async () => ({
  plugins: [react()],

  // 保留 Rust 编译错误，避免 Vite 清屏掩盖原生端失败。
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    host: host || false,
    hmr: host
      ? {
          protocol: "ws",
          host,
          port: 1421,
        }
      : undefined,
    watch: {
      // Rust 变更由 Tauri Watch 处理，前端监听只覆盖 Web 资源。
      ignored: ["**/src-tauri/**"],
    },
  },
}));
