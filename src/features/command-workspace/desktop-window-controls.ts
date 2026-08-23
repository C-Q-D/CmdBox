/**
 * CmdBox 主窗口的最小桌面控制边界。
 *
 * 本模块只代理当前 Tauri 窗口的最小化、最大化切换与普通关闭，不接受窗口标签、
 * 尺寸、位置或任意命令。浏览器预览不会获得伪造的窗口控制能力。
 */
import { isTauri } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";

/** Command Workspace 可以调用的最小主窗口操作。 */
export interface DesktopWindowControls {
  /** 最小化当前 CmdBox 主窗口。 */
  minimize(): Promise<void>;
  /** 在最大化与还原之间切换当前 CmdBox 主窗口。 */
  toggleMaximize(): Promise<void>;
  /** 普通关闭当前 CmdBox 主窗口。 */
  close(): Promise<void>;
}

/** 为真实 Tauri 宿主创建当前窗口控制；普通浏览器返回 `null`。 */
export function createDesktopWindowControls(): DesktopWindowControls | null {
  if (!isTauri()) {
    return null;
  }
  const currentWindow = getCurrentWindow();
  return {
    /** 调用 Tauri 当前窗口最小化命令。 */
    minimize: () => currentWindow.minimize(),
    /** 调用 Tauri 当前窗口最大化切换命令。 */
    toggleMaximize: () => currentWindow.toggleMaximize(),
    /** 调用 Tauri 当前窗口普通关闭命令。 */
    close: () => currentWindow.close(),
  };
}
