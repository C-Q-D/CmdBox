/** CmdBox React 入口，只负责装配正式 Command Workspace。 */
import { CommandWorkspace } from "../features/command-workspace/CommandWorkspace";
import "./App.css";

/**
 * 装配继承已确认视觉语法的正式 Command Workspace。
 *
 * 当前默认入口读取 Rust Core 的两个无破坏 CMD-02 Built-in 并渲染统一类型化参数表单；
 * 已验证的 CMD-01 Execution 内核继续保留，通用 Preview/Run 尚未在本原子接线。
 */
function App() {
  return <CommandWorkspace />;
}

export default App;
