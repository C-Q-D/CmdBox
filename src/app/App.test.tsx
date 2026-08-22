import { cleanup, fireEvent, render, screen } from "@testing-library/react";
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

  it("按命令名称过滤索引并呈现无结果状态", function filterCommandIndex() {
    render(<App />);

    fireEvent.change(screen.getByRole("searchbox", { name: "搜索命令块" }), {
      target: { value: "压缩" },
    });

    expect(screen.getByText("压缩并分卷备份目录")).toBeDefined();
    expect(screen.queryByText("清理构建缓存目录")).toBeNull();

    fireEvent.change(screen.getByRole("searchbox", { name: "搜索命令块" }), {
      target: { value: "不存在的命令" },
    });
    expect(screen.getByRole("status", { name: "" })).toHaveProperty(
      "textContent",
      "没有匹配的命令块",
    );
  });

  it("目标变化后使 Preview 失效并可恢复演示状态", function invalidatePreview() {
    render(<App />);

    fireEvent.click(
      screen.getByRole("button", { name: `移除 ${String.raw`D:\项目缓存`}` }),
    );

    expect(screen.getByRole("heading", { name: "目标文件夹（2）" })).toBeDefined();
    expect(screen.getByText("需要重新预览")).toBeDefined();
    expect(screen.getByText("旧 Preview 已失效")).toBeDefined();
    expect(
      (screen.getByRole("button", { name: "永久删除" }) as HTMLButtonElement)
        .disabled,
    ).toBe(true);

    fireEvent.click(screen.getByRole("button", { name: "恢复原型状态" }));

    expect(screen.getByRole("heading", { name: "目标文件夹（3）" })).toBeDefined();
    expect(screen.getByText(String.raw`D:\项目缓存`)).toBeDefined();
    expect(screen.getByText("预览已就绪")).toBeDefined();
    expect(
      (screen.getByRole("button", { name: "永久删除" }) as HTMLButtonElement)
        .disabled,
    ).toBe(false);
  });

  it("永久删除动作只打开无副作用说明", function explainPrototypeAction() {
    render(<App />);

    fireEvent.click(screen.getByRole("button", { name: "永久删除" }));

    const dialog = screen.getByRole("dialog", { name: "永久删除动作说明" });
    expect(dialog).toBeDefined();
    expect(screen.getByText(/前端视觉原型不会执行真实命令/)).toBeDefined();

    fireEvent.keyDown(dialog, { key: "Escape" });
    expect(screen.queryByRole("dialog")).toBeNull();
  });
});
