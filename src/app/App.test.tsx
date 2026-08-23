/** Command Workspace 的宿主降级、执行事件、取消和错误状态测试。 */
import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { CommandWorkspace } from "../features/command-workspace/CommandWorkspace";
import type { ExecutionStreamEvent, FixedExecutionGateway } from "../features/command-workspace/execution-gateway";
import type { DesktopWindowControls } from "../features/command-workspace/desktop-window-controls";
import App from "./App";

/** 每个测试后卸载 React 树，避免 Channel 回调和 DOM 状态串扰。 */
afterEach(function cleanupRenderedApp() {
  cleanup();
});

/** 创建可由测试主动推送后端事件的固定任务 Gateway。 */
function createGatewayFixture(options?: { startFailure?: boolean; cancelFailure?: boolean; deferStart?: boolean; deferCancel?: boolean }) {
  /** 固定测试 Execution UUID。 */
  const executionId = "9be8ec5d-ef8c-4c2a-a7f5-12069b2ad555";
  /** 最近一次 Run 注册的 Channel 回调。 */
  let eventHandler: ((event: ExecutionStreamEvent) => void) | undefined;
  /** 取消调用收到的 Execution UUID。 */
  const cancelledExecutionIds: string[] = [];
  /** 允许测试精确控制启动响应时序的解析器。 */
  let resolveStartResponse: (() => void) | undefined;
  /** 允许测试精确控制取消响应时序的解析器。 */
  let resolveCancelResponse: (() => void) | undefined;
  /** 可观察的无副作用 Gateway。 */
  const gateway: FixedExecutionGateway = {
    /** 保存 Channel 回调并返回固定启动响应。 */
    async startFixedExecution(onEvent) {
      if (options?.startFailure) {
        throw { code: "PROCESS_START_FAILED", message: "无法启动固定 PowerShell 任务" };
      }
      eventHandler = onEvent;
      if (options?.deferStart) {
        await new Promise<void>((resolve) => {
          resolveStartResponse = resolve;
        });
      }
      return { executionId };
    },
    /** 记录取消目标并返回 Rust 已进入 Cancelling 的事实。 */
    async cancelExecution(targetExecutionId) {
      cancelledExecutionIds.push(targetExecutionId);
      if (options?.deferCancel) {
        await new Promise<void>((resolve) => {
          resolveCancelResponse = resolve;
        });
      }
      if (options?.cancelFailure) {
        throw { code: "CANCEL_FAILED", message: "无法取消当前 Execution" };
      }
      return { accepted: true, state: "cancelling" };
    },
  };
  return {
    /** 注入 Workspace 的 Gateway。 */
    gateway,
    /** 固定测试 Execution UUID。 */
    executionId,
    /** 取消调用记录。 */
    cancelledExecutionIds,
    /** 向当前 Channel 推送一个后端事件。 */
    emit(event: ExecutionStreamEvent) {
      if (!eventHandler) {
        throw new Error("应先启动任务再推送事件");
      }
      eventHandler(event);
    },
    /** 释放被测试挂起的启动响应。 */
    resolveStart() {
      resolveStartResponse?.();
    },
    /** 释放被测试挂起的取消响应。 */
    resolveCancel() {
      resolveCancelResponse?.();
    },
  };
}

describe("CmdBox Command Workspace", function describeWorkspace() {
  it("纯浏览器环境显示桌面宿主要求并保持运行禁用", function showHostRequirement() {
    render(<App />);

    expect(screen.getByRole("heading", { name: "执行链路验收", level: 1 })).toBeDefined();
    expect(screen.getByRole("navigation", { name: "主导航" })).toBeDefined();
    expect(screen.getByLabelText("Command Block 索引")).toBeDefined();
    expect(screen.getAllByText("需要桌面宿主").length).toBeGreaterThan(0);
    expect((screen.getByRole("button", { name: "运行验收任务" }) as HTMLButtonElement).disabled).toBe(true);
    expect((screen.getByRole("button", { name: "最小化窗口" }) as HTMLButtonElement).disabled).toBe(true);
    expect((screen.getByRole("button", { name: "最大化或还原窗口" }) as HTMLButtonElement).disabled).toBe(true);
    expect((screen.getByRole("button", { name: "关闭窗口" }) as HTMLButtonElement).disabled).toBe(true);
  });

  it("把唯一标题栏的三个按钮映射到最小当前窗口能力", function controlDesktopWindow() {
    const windowControls: DesktopWindowControls = {
      /** 记录最小化动作。 */
      minimize: vi.fn(async () => undefined),
      /** 记录最大化切换动作。 */
      toggleMaximize: vi.fn(async () => undefined),
      /** 记录普通关闭动作。 */
      close: vi.fn(async () => undefined),
    };
    render(<CommandWorkspace gateway={null} windowControls={windowControls} />);

    fireEvent.click(screen.getByRole("button", { name: "最小化窗口" }));
    fireEvent.click(screen.getByRole("button", { name: "最大化或还原窗口" }));
    fireEvent.click(screen.getByRole("button", { name: "关闭窗口" }));

    expect(windowControls.minimize).toHaveBeenCalledOnce();
    expect(windowControls.toggleMaximize).toHaveBeenCalledOnce();
    expect(windowControls.close).toHaveBeenCalledOnce();
  });

  it("按命令名称过滤索引并呈现无结果状态", function filterCommandIndex() {
    render(<App />);
    fireEvent.change(screen.getByRole("searchbox", { name: "搜索命令块" }), { target: { value: "永久删除" } });
    expect(screen.getByText("快速永久删除多个文件夹")).toBeDefined();
    expect(screen.queryByText("执行链路验收", { selector: ".command-row strong" })).toBeNull();

    fireEvent.change(screen.getByRole("searchbox", { name: "搜索命令块" }), { target: { value: "不存在的命令" } });
    expect(screen.getByRole("status")).toHaveProperty("textContent", "没有匹配的命令块");
  });

  it("只按后端事件显示纯文本输出和自然终态", async function renderExecutionEvents() {
    const fixture = createGatewayFixture();
    const rendered = render(<CommandWorkspace gateway={fixture.gateway} />);

    fireEvent.click(screen.getByRole("button", { name: "运行验收任务" }));
    await waitFor(() => expect(screen.getByText(fixture.executionId)).toBeDefined());
    await act(async () => {
      fixture.emit({ event: "started", data: { executionId: fixture.executionId, sequence: 0 } });
      fixture.emit({
        event: "output",
        data: {
          executionId: fixture.executionId,
          sequence: 1,
          fragments: [{ fragmentSequence: 0, stream: "stdout", text: "<b>plain html</b> https://example.invalid \u001b[31mANSI" }],
          droppedBytesBefore: 0,
        },
      });
      fixture.emit({
        event: "output",
        data: {
          executionId: fixture.executionId,
          sequence: 1,
          fragments: [{ fragmentSequence: 1, stream: "stdout", text: "重复事件不得显示" }],
          droppedBytesBefore: 0,
        },
      });
      fixture.emit({
        event: "finished",
        data: { executionId: fixture.executionId, sequence: 2, exitCode: 0, durationMs: 8123, droppedOutputBytes: 0 },
      });
      fixture.emit({
        event: "output",
        data: {
          executionId: fixture.executionId,
          sequence: 3,
          fragments: [{ fragmentSequence: 2, stream: "stderr", text: "终态后输出不得显示" }],
          droppedBytesBefore: 0,
        },
      });
      fixture.emit({
        event: "failed",
        data: { executionId: fixture.executionId, sequence: 4, message: "终态不得覆盖", durationMs: 9000, droppedOutputBytes: 0 },
      });
    });

    expect(screen.getByText(/<b>plain html<\/b>/)).toBeDefined();
    expect(rendered.container.querySelector(".execution-output b")).toBeNull();
    expect(screen.queryByRole("link", { name: /example\.invalid/ })).toBeNull();
    expect(screen.queryByText("重复事件不得显示")).toBeNull();
    expect(screen.getByText("任务自然结束")).toBeDefined();
    expect(screen.getByText("8123 ms")).toBeDefined();
    expect(screen.getByText("0", { selector: "dd" })).toBeDefined();
    expect(screen.queryByText("终态后输出不得显示")).toBeNull();
    expect(screen.queryByText("任务失败")).toBeNull();
  });

  it("只在启动响应锁定 Execution ID 后重放匹配事件", async function lockResponseExecutionId() {
    const fixture = createGatewayFixture({ deferStart: true });
    render(<CommandWorkspace gateway={fixture.gateway} />);

    fireEvent.click(screen.getByRole("button", { name: "运行验收任务" }));
    await act(async () => {
      fixture.emit({ event: "started", data: { executionId: "11111111-1111-4111-8111-111111111111", sequence: 0 } });
      fixture.emit({ event: "finished", data: { executionId: "11111111-1111-4111-8111-111111111111", sequence: 1, exitCode: 0, durationMs: 1, droppedOutputBytes: 0 } });
    });
    expect(screen.queryByText("11111111-1111-4111-8111-111111111111")).toBeNull();
    expect(screen.queryByText("任务自然结束")).toBeNull();

    await act(async () => fixture.resolveStart());
    await waitFor(() => expect(screen.getByText(fixture.executionId)).toBeDefined());
    expect(screen.queryByText("任务自然结束")).toBeNull();
  });

  it("把启动响应前的可信与错误 ID 输出共同限制在 512 KiB 内", async function boundPreResponseEvents() {
    const fixture = createGatewayFixture({ deferStart: true });
    const rendered = render(<CommandWorkspace gateway={fixture.gateway} />);
    const chunk = "x".repeat(64 * 1024);
    const wrongExecutionId = "22222222-2222-4222-8222-222222222222";

    fireEvent.click(screen.getByRole("button", { name: "运行验收任务" }));
    await act(async () => {
      fixture.emit({ event: "started", data: { executionId: fixture.executionId, sequence: 0 } });
      for (let sequence = 1; sequence <= 12; sequence += 1) {
        fixture.emit({
          event: "output",
          data: {
            executionId: sequence <= 4 ? wrongExecutionId : fixture.executionId,
            sequence,
            fragments: [{ fragmentSequence: sequence, stream: "stdout", text: chunk }],
            droppedBytesBefore: 0,
          },
        });
      }
      fixture.resolveStart();
    });

    await waitFor(() => expect(screen.getAllByText("运行中").length).toBeGreaterThan(0));
    const retainedText = Array.from(rendered.container.querySelectorAll(".execution-output pre"))
      .map((element) => element.textContent ?? "")
      .join("");
    expect(new TextEncoder().encode(retainedText).byteLength).toBeLessThanOrEqual(512 * 1024);
    expect(screen.getByText(/早期实时输出已有 \d+ 字节未保留/)).toBeDefined();
  });

  it("不把错误 Execution ID 的预响应淘汰计入当前任务", async function ignoreWrongIdDrops() {
    const fixture = createGatewayFixture({ deferStart: true });
    render(<CommandWorkspace gateway={fixture.gateway} />);

    fireEvent.click(screen.getByRole("button", { name: "运行验收任务" }));
    await act(async () => {
      fixture.emit({
        event: "output",
        data: {
          executionId: "33333333-3333-4333-8333-333333333333",
          sequence: 1,
          fragments: [{ fragmentSequence: 1, stream: "stdout", text: "x".repeat(600 * 1024) }],
          droppedBytesBefore: 700,
        },
      });
      fixture.resolveStart();
    });

    await waitFor(() => expect(screen.getByText(fixture.executionId)).toBeDefined());
    expect(screen.queryByText(/早期实时输出已有 \d+ 字节未保留/)).toBeNull();
  });

  it("完整记录匹配 Execution 被淘汰事件的文本与 Rust 丢弃字节", async function preserveAuthenticatedDrops() {
    const fixture = createGatewayFixture({ deferStart: true });
    render(<CommandWorkspace gateway={fixture.gateway} />);

    fireEvent.click(screen.getByRole("button", { name: "运行验收任务" }));
    await act(async () => {
      fixture.emit({
        event: "output",
        data: {
          executionId: fixture.executionId,
          sequence: 1,
          fragments: [{ fragmentSequence: 1, stream: "stderr", text: "x".repeat(600 * 1024) }],
          droppedBytesBefore: 1000,
        },
      });
      fixture.resolveStart();
    });

    await waitFor(() => {
      expect(screen.getByText("早期实时输出已有 615400 字节未保留；外部任务未因此阻塞。")).toBeDefined();
    });
  });

  it("取消响应和后端终态分别推进 Cancelling 与 Cancelled", async function cancelExecution() {
    const fixture = createGatewayFixture();
    render(<CommandWorkspace gateway={fixture.gateway} />);

    fireEvent.click(screen.getByRole("button", { name: "运行验收任务" }));
    await waitFor(() => expect(screen.getByText(fixture.executionId)).toBeDefined());
    await act(async () => {
      fixture.emit({ event: "started", data: { executionId: fixture.executionId, sequence: 0 } });
    });
    fireEvent.click(screen.getByRole("button", { name: "终止任务" }));
    await waitFor(() => expect(screen.getAllByText("正在终止进程树").length).toBeGreaterThan(0));
    expect(fixture.cancelledExecutionIds).toEqual([fixture.executionId]);

    await act(async () => {
      fixture.emit({
        event: "cancelled",
        data: { executionId: fixture.executionId, sequence: 1, durationMs: 950, droppedOutputBytes: 0 },
      });
    });
    expect(screen.getByText("任务已取消")).toBeDefined();
    expect(screen.getByRole("button", { name: "再次运行" })).toBeDefined();
  });

  it("终态不被较晚的取消响应倒退", async function isolateLateCancelResponse() {
    const fixture = createGatewayFixture({ deferCancel: true });
    render(<CommandWorkspace gateway={fixture.gateway} />);

    fireEvent.click(screen.getByRole("button", { name: "运行验收任务" }));
    await waitFor(() => expect(screen.getByText(fixture.executionId)).toBeDefined());
    await act(async () => fixture.emit({ event: "started", data: { executionId: fixture.executionId, sequence: 0 } }));
    fireEvent.click(screen.getByRole("button", { name: "终止任务" }));
    await act(async () => fixture.emit({ event: "cancelled", data: { executionId: fixture.executionId, sequence: 1, durationMs: 50, droppedOutputBytes: 0 } }));
    expect(screen.getByText("任务已取消")).toBeDefined();

    await act(async () => fixture.resolveCancel());
    expect(screen.queryByText("正在终止进程树")).toBeNull();
    expect(screen.getByText("任务已取消")).toBeDefined();
  });

  it("下一次运行不受旧取消错误污染", async function isolateLateCancelError() {
    const fixture = createGatewayFixture({ deferCancel: true, cancelFailure: true });
    render(<CommandWorkspace gateway={fixture.gateway} />);

    fireEvent.click(screen.getByRole("button", { name: "运行验收任务" }));
    await waitFor(() => expect(screen.getByText(fixture.executionId)).toBeDefined());
    await act(async () => fixture.emit({ event: "started", data: { executionId: fixture.executionId, sequence: 0 } }));
    fireEvent.click(screen.getByRole("button", { name: "终止任务" }));
    await act(async () => fixture.emit({ event: "cancelled", data: { executionId: fixture.executionId, sequence: 1, durationMs: 50, droppedOutputBytes: 0 } }));
    fireEvent.click(screen.getByRole("button", { name: "再次运行" }));
    await act(async () => fixture.resolveCancel());

    expect(screen.queryByRole("alert")).toBeNull();
    expect(screen.getAllByText("正在建立执行").length).toBeGreaterThan(0);
  });

  it("启动失败显示公开错误并恢复可运行状态", async function showStartFailure() {
    const fixture = createGatewayFixture({ startFailure: true });
    render(<CommandWorkspace gateway={fixture.gateway} />);

    fireEvent.click(screen.getByRole("button", { name: "运行验收任务" }));
    await waitFor(() => {
      expect(screen.getByRole("alert")).toHaveProperty("textContent", expect.stringContaining("PROCESS_START_FAILED"));
    });
    expect(screen.getByRole("button", { name: "运行验收任务" })).toBeDefined();
  });
});
