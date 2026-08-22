import { CommandWorkspacePrototype } from "../features/command-workspace/CommandWorkspacePrototype";
import "./App.css";

/**
 * 装配当前经过用户确认的 Command Workspace 前端视觉原型。
 *
 * 原型不连接 Tauri IPC，也不会执行任何命令或文件系统操作。
 */
function App() {
  return <CommandWorkspacePrototype />;
}

export default App;
