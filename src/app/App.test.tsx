/** 验证 CmdBox 空骨架向开发者呈现稳定、可识别的开发入口。 */
import { cleanup, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";
import App from "./App";

/** 每个测试结束后清理 jsdom 中的 React 根节点。 */
afterEach(function cleanupRenderedApp() {
  cleanup();
});

describe("CmdBox 开发环境准备页", function describeReadinessPage() {
  /** 验证环境状态和两类日常开发路径同时可见。 */
  it("呈现准备完成状态及 Windows、Docker 入口", function renderReadinessPage() {
    render(<App />);

    expect(
      screen.getByRole("heading", { name: "环境准备完成" }),
    ).toBeDefined();
    expect(screen.getByText("Windows Tauri Dev")).toBeDefined();
    expect(screen.getByText("Docker + Vite")).toBeDefined();
  });
});
