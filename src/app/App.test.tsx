import { cleanup, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";
import App from "./App";

afterEach(function cleanupRenderedApp() {
  cleanup();
});

describe("CmdBox Command Workspace 视觉原型", function describeWorkspace() {
  it("呈现统一 Ready 工作区的核心证据与动作", function renderReadyWorkspace() {
    render(<App />);

    expect(
      screen.getByRole("heading", {
        name: "快速永久删除多个文件夹",
        level: 1,
      }),
    ).toBeDefined();
    expect(screen.getByRole("navigation", { name: "主导航" })).toBeDefined();
    expect(screen.getByLabelText("Command Block 索引")).toBeDefined();
    expect(screen.getByText(String.raw`D:\项目缓存`)).toBeDefined();
    expect(screen.getByText(String.raw`E:\旧版构建产物`)).toBeDefined();
    expect(screen.getByText(String.raw`F:\临时 下载`)).toBeDefined();
    expect(screen.getByText("预览已就绪")).toBeDefined();
    expect(screen.getByText("安全检查通过")).toBeDefined();
    expect(screen.getByLabelText("PowerShell 命令预览")).toHaveProperty(
      "textContent",
      expect.stringContaining("-LiteralPath"),
    );
    expect(screen.getByRole("button", { name: "永久删除" })).toBeDefined();
  });
});
