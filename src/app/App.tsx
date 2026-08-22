/**
 * CmdBox 空项目骨架的环境准备页。
 *
 * 当前页面只证明 React 与 Tauri 可以协同启动，不包含任何 Command Block、进程执行或
 * 文件删除业务；后续产品切片会在已经验证的开发环境上替换此内容。
 */
import { DevelopmentEntry } from "../components/DevelopmentEntry";
import "./App.css";

/** 展示当前骨架的职责和两条开发入口。 */
function App() {
  return (
    <main className="readiness-page">
      <section className="readiness-card" aria-labelledby="readiness-title">
        <p className="eyebrow">CmdBox · Windows First</p>
        <h1 id="readiness-title">环境准备完成</h1>
        <p className="summary">
          当前已建立 Tauri 2、React、TypeScript 与 Rust 的可构建空骨架，尚未加入任何命令执行能力。
        </p>

        <div className="entry-grid" aria-label="开发入口">
          <DevelopmentEntry
            label="完整桌面开发"
            title="Windows Tauri Dev"
            description="用于原生窗口、Rust 和后续系统能力验证。"
          />
          <DevelopmentEntry
            label="纯前端开发"
            title="Docker + Vite"
            description="用于快速界面开发和浏览器热更新。"
          />
        </div>

        <p className="status" role="status">
          基线状态：可安装、可构建、可测试、无演示 IPC、无业务权限。
        </p>
      </section>
    </main>
  );
}

export default App;
