import { CommandWorkspace } from "../features/command-workspace/CommandWorkspace";
import "./App.css";

/**
 * 装配继承已确认视觉语法的正式 Command Workspace。
 *
 * 当前默认入口只接入 Rust Core 内置的无破坏 CMD-01 固定验收任务。
 */
function App() {
  return <CommandWorkspace />;
}

export default App;
