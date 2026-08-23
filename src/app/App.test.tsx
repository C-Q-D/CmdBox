/** Command Workspace 的宿主降级、执行事件、取消和错误状态测试。 */
import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { CommandWorkspace } from "../features/command-workspace/CommandWorkspace";
import type {
  CommandBlockDetails,
  CommandBlockSummary,
  CommandExecutionGateway,
  ExecutionStreamEvent,
  FixedExecutionGateway,
} from "../features/command-workspace/execution-gateway";
import type { DesktopWindowControls } from "../features/command-workspace/desktop-window-controls";
import type { FolderPicker } from "../features/command-workspace/folder-picker";
import App from "./App";

/** 每个测试后卸载 React 树，避免 Channel 回调和 DOM 状态串扰。 */
afterEach(function cleanupRenderedApp() {
  cleanup();
});

/** 创建一个可由异步竞态测试精确解析的 Promise。 */
function createDeferred<T>() {
  /** 外部持有的解析器。 */
  let resolvePromise!: (value: T) => void;
  /** 当前挂起的 Promise。 */
  const promise = new Promise<T>((resolve) => {
    resolvePromise = resolve;
  });
  return { promise, resolve: resolvePromise };
}

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

/** 两个真实 Gateway 摘要的稳定测试形状。 */
const commandSummaries: CommandBlockSummary[] = [
  {
    id: "builtin.parameter-echo.windows-powershell",
    name: "PowerShell 参数回显",
    description: "用 Windows PowerShell 回显六类类型化参数。",
    origin: "builtin",
    runner: "windowsPowerShell",
    riskLevel: "normal",
    revision: 1,
  },
  {
    id: "builtin.parameter-echo.cmd",
    name: "CMD 参数回显",
    description: "用 CMD 回显六类类型化参数。",
    origin: "builtin",
    runner: "cmd",
    riskLevel: "normal",
    revision: 1,
  },
];

/** 为一个真实 Summary 创建带同 key 不同默认值的最小 Details。 */
function createCommandDetails(
  summary: CommandBlockSummary,
  defaultValue: string,
): CommandBlockDetails {
  return {
    ...summary,
    parameters: [
      {
        type: "text",
        key: "text",
        label: "文本",
        description: "当前 Definition 的文本参数",
        required: false,
        remember: false,
        defaultValue,
        minLength: 0,
        maxLength: 32,
        placeholder: "输入文本",
      },
    ],
  };
}

/** 创建只允许 list/get 的通用 Gateway，并记录 Preview/Run 不应被调用。 */
function createCommandGatewayFixture() {
  /** 当前两条 Summary 对应的 Details。 */
  const details = new Map<string, CommandBlockDetails>([
    [commandSummaries[0].id, createCommandDetails(commandSummaries[0], "PowerShell 默认")],
    [commandSummaries[1].id, createCommandDetails(commandSummaries[1], "CMD 默认")],
  ]);
  /** 通用 Gateway 的可观察替身。 */
  const gateway: CommandExecutionGateway = {
    /** 返回两条真实摘要的防御性数组。 */
    listCommandBlocks: vi.fn(async () => [...commandSummaries]),
    /** 按业务 ID 返回当前测试详情。 */
    getCommandBlock: vi.fn(async (commandBlockId) => {
      const definition = details.get(commandBlockId);
      if (!definition) throw { code: "COMMAND_BLOCK_NOT_FOUND", message: "not found" };
      return definition;
    }),
    /** UI-FORM 原子不得调用 Preview。 */
    previewCommandBlock: vi.fn(async () => {
      throw new Error("UI-FORM 不得调用 Preview");
    }),
    /** UI-FORM 原子不得调用通用 Run。 */
    runCommandBlock: vi.fn(async () => {
      throw new Error("UI-FORM 不得调用通用 Run");
    }),
    /** 通用取消不是当前固定 Execution Gateway 的职责。 */
    cancelExecution: vi.fn(async () => ({ accepted: false, state: null })),
  };
  return { gateway, details };
}

/** 渲染同时带真实 Definition 与可选既有 Execution Gateway 的工作区。 */
function renderConnectedWorkspace(gateway: FixedExecutionGateway | null = null) {
  const commandFixture = createCommandGatewayFixture();
  const rendered = render(
    <CommandWorkspace
      gateway={gateway}
      commandGateway={commandFixture.gateway}
      folderPicker={null}
    />,
  );
  return { ...rendered, commandFixture };
}

describe("CmdBox Command Workspace", function describeWorkspace() {
  it("纯浏览器环境显示桌面宿主要求并保持运行禁用", function showHostRequirement() {
    render(<App />);

    expect(screen.getByRole("heading", { name: "Command Workspace", level: 1 })).toBeDefined();
    expect(screen.getByRole("navigation", { name: "主导航" })).toBeDefined();
    expect(screen.getByLabelText("Command Block 索引")).toBeDefined();
    expect(screen.getAllByText("需要桌面宿主").length).toBeGreaterThan(0);
    expect((screen.getByRole("button", { name: "需要桌面宿主" }) as HTMLButtonElement).disabled).toBe(true);
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

  it("加载两条真实 Summary、首选第一项并只读取当前 Details", async function loadRealDefinitions() {
    const { commandFixture } = renderConnectedWorkspace();

    await waitFor(() => expect(screen.getByRole("heading", { name: "PowerShell 参数回显", level: 1 })).toBeDefined());
    expect(screen.getByText("CMD 参数回显", { selector: ".command-row strong" })).toBeDefined();
    expect(commandFixture.gateway.listCommandBlocks).toHaveBeenCalledOnce();
    expect(commandFixture.gateway.getCommandBlock).toHaveBeenCalledWith(commandSummaries[0].id);
    expect(screen.getByText("Windows PowerShell", { selector: ".runner-facts strong" })).toBeDefined();
    expect(screen.getByText("正在配置参数", { selector: ".runner-facts strong" })).toBeDefined();
    expect(screen.getByText("builtin", { selector: ".runner-note dd" })).toBeDefined();
    expect(screen.getByText("1", { selector: ".runner-note dd" })).toBeDefined();
    expect(commandFixture.gateway.previewCommandBlock).not.toHaveBeenCalled();
    expect(commandFixture.gateway.runCommandBlock).not.toHaveBeenCalled();
    expect(screen.queryByText("快速永久删除多个文件夹")).toBeNull();
    expect(screen.queryByText(/共 25 个命令块/)).toBeNull();
    expect(screen.queryByText(/创建受同一 Job 管理的诊断子进程/)).toBeNull();
    expect(screen.queryByText(/安全检查通过/)).toBeNull();
    expect(screen.queryByText(/工作目录/)).toBeNull();

    fireEvent.change(screen.getByRole("textbox", { name: /文本/ }), { target: { value: "新的有效值" } });
    expect(screen.getByText("正在配置参数", { selector: ".runner-facts strong" })).toBeDefined();
  });

  it("通用桌面 Gateway 已连接但固定 Execution 未接线时显示准确不可运行状态", async function showPendingRunWiring() {
    renderConnectedWorkspace();
    await screen.findByRole("heading", { name: "PowerShell 参数回显", level: 1 });

    expect(screen.queryByText("需要桌面宿主")).toBeNull();
    expect(screen.queryByText(/纯浏览器环境/)).toBeNull();
    expect(screen.getAllByText("Run 尚未接线").length).toBeGreaterThan(0);
    const runButton = screen.getByRole("button", { name: "运行尚未接线" });
    expect((runButton as HTMLButtonElement).disabled).toBe(true);
    expect(screen.queryByRole("button", { name: "运行验收任务" })).toBeNull();
  });

  it("切换真实命令时按 id/revision/generation 重建表单且不继承同 key", async function resetFormOnCommandSwitch() {
    const { commandFixture } = renderConnectedWorkspace();
    const input = await screen.findByRole("textbox", { name: /文本/ });
    expect(input).toHaveProperty("value", "PowerShell 默认");
    fireEvent.change(input, { target: { value: "用户编辑" } });

    fireEvent.click(screen.getByRole("button", { name: /CMD 参数回显/ }));
    await waitFor(() => {
      expect(screen.getByRole("heading", { name: "CMD 参数回显", level: 1 })).toBeDefined();
      expect(screen.getByRole("textbox", { name: /文本/ })).toHaveProperty("value", "CMD 默认");
    });
    fireEvent.click(screen.getByRole("button", { name: /PowerShell 参数回显/ }));
    await waitFor(() => expect(screen.getByRole("textbox", { name: /文本/ })).toHaveProperty("value", "PowerShell 默认"));
    expect(commandFixture.gateway.getCommandBlock).toHaveBeenCalledTimes(3);
  });

  it("按真实命令名称过滤索引并呈现无结果状态", async function filterCommandIndex() {
    renderConnectedWorkspace();
    await waitFor(() => expect(screen.getByText("PowerShell 参数回显", { selector: ".command-row strong" })).toBeDefined());
    fireEvent.change(screen.getByRole("searchbox", { name: "搜索命令块" }), { target: { value: "CMD" } });
    expect(screen.getByText("CMD 参数回显", { selector: ".command-row strong" })).toBeDefined();
    expect(screen.queryByText("PowerShell 参数回显", { selector: ".command-row strong" })).toBeNull();

    fireEvent.change(screen.getByRole("searchbox", { name: "搜索命令块" }), { target: { value: "不存在的命令" } });
    expect(screen.getByRole("status")).toHaveProperty("textContent", "没有匹配的命令块");
  });

  it("丢弃 List/Get 乱序和卸载后的迟到响应", async function discardLateDefinitionResponses() {
    const firstDetails = createDeferred<CommandBlockDetails>();
    const secondDetails = createDeferred<CommandBlockDetails>();
    const fixture = createCommandGatewayFixture();
    fixture.gateway.getCommandBlock = vi.fn((commandBlockId: string) =>
      commandBlockId === commandSummaries[0].id
        ? firstDetails.promise
        : secondDetails.promise,
    );
    const rendered = render(
      <CommandWorkspace gateway={null} commandGateway={fixture.gateway} folderPicker={null} />,
    );
    await waitFor(() => expect(screen.getByText("CMD 参数回显", { selector: ".command-row strong" })).toBeDefined());
    fireEvent.click(screen.getByRole("button", { name: /CMD 参数回显/ }));
    await act(async () => secondDetails.resolve(createCommandDetails(commandSummaries[1], "CMD 新定义")));
    await waitFor(() => expect(screen.getByRole("heading", { name: "CMD 参数回显", level: 1 })).toBeDefined());
    await act(async () => firstDetails.resolve(createCommandDetails(commandSummaries[0], "迟到旧定义")));
    expect(screen.getByRole("heading", { name: "CMD 参数回显", level: 1 })).toBeDefined();
    expect(screen.getByRole("textbox", { name: /文本/ })).toHaveProperty("value", "CMD 新定义");

    const lateList = createDeferred<CommandBlockSummary[]>();
    const unmountFixture = createCommandGatewayFixture();
    unmountFixture.gateway.listCommandBlocks = vi.fn(() => lateList.promise);
    rendered.unmount();
    const lateRendered = render(<CommandWorkspace gateway={null} commandGateway={unmountFixture.gateway} folderPicker={null} />);
    lateRendered.unmount();
    await act(async () => lateList.resolve([...commandSummaries]));
    expect(unmountFixture.gateway.getCommandBlock).not.toHaveBeenCalled();
  });

  it("同 id/revision 经 A→B→A 多 generation 切换时丢弃第一次 A 的迟到 Details", async function discardRepeatedIdentityGeneration() {
    const firstA = createDeferred<CommandBlockDetails>();
    const middleB = createDeferred<CommandBlockDetails>();
    const latestA = createDeferred<CommandBlockDetails>();
    const pendingDetails = [firstA, middleB, latestA];
    let requestIndex = 0;
    const fixture = createCommandGatewayFixture();
    fixture.gateway.getCommandBlock = vi.fn(() => {
      const request = pendingDetails[requestIndex];
      requestIndex += 1;
      if (!request) throw new Error("测试只允许三个 Details 请求");
      return request.promise;
    });
    render(
      <CommandWorkspace
        gateway={null}
        commandGateway={fixture.gateway}
        folderPicker={null}
      />,
    );
    await waitFor(() => expect(fixture.gateway.getCommandBlock).toHaveBeenCalledTimes(1));
    fireEvent.click(screen.getByRole("button", { name: /CMD 参数回显/ }));
    fireEvent.click(screen.getByRole("button", { name: /PowerShell 参数回显/ }));
    expect(fixture.gateway.getCommandBlock).toHaveBeenCalledTimes(3);

    await act(async () =>
      latestA.resolve(createCommandDetails(commandSummaries[0], "A 最新 generation")),
    );
    await waitFor(() =>
      expect(screen.getByRole("textbox", { name: /文本/ })).toHaveProperty(
        "value",
        "A 最新 generation",
      ),
    );
    await act(async () =>
      firstA.resolve(createCommandDetails(commandSummaries[0], "A 迟到旧 generation")),
    );
    await act(async () =>
      middleB.resolve(createCommandDetails(commandSummaries[1], "B 迟到 generation")),
    );
    expect(screen.getByRole("heading", { name: "PowerShell 参数回显", level: 1 })).toBeDefined();
    expect(screen.getByRole("textbox", { name: /文本/ })).toHaveProperty(
      "value",
      "A 最新 generation",
    );
  });

  it("组件卸载后静默丢弃当前 Get Details 的迟到结果", async function discardGetAfterUnmount() {
    const lateDetails = createDeferred<CommandBlockDetails>();
    const fixture = createCommandGatewayFixture();
    fixture.gateway.getCommandBlock = vi.fn(() => lateDetails.promise);
    const rendered = render(
      <CommandWorkspace
        gateway={null}
        commandGateway={fixture.gateway}
        folderPicker={null}
      />,
    );
    await waitFor(() => expect(fixture.gateway.getCommandBlock).toHaveBeenCalledOnce());
    rendered.unmount();

    await act(async () =>
      lateDetails.resolve(createCommandDetails(commandSummaries[0], "卸载后迟到")),
    );
    expect(fixture.gateway.previewCommandBlock).not.toHaveBeenCalled();
    expect(fixture.gateway.runCommandBlock).not.toHaveBeenCalled();
  });

  it("把当前 generation 的 Details id 不匹配收敛为安全 IPC 失败并结束加载", async function rejectMismatchedDefinitionId() {
    const fixture = createCommandGatewayFixture();
    fixture.gateway.getCommandBlock = vi.fn(async () => ({
      ...createCommandDetails(commandSummaries[0], "错误响应"),
      id: "builtin.unexpected",
    }));
    render(
      <CommandWorkspace
        gateway={null}
        commandGateway={fixture.gateway}
        folderPicker={null}
      />,
    );

    const alert = await screen.findByRole("alert");
    expect(alert).toHaveProperty(
      "textContent",
      "IPC_FAILEDCmdBox 无法完成桌面宿主调用",
    );
    expect(screen.getByText("定义读取失败", { selector: ".runner-facts strong" })).toBeDefined();
    expect(screen.queryByRole("textbox", { name: /文本/ })).toBeNull();
    expect(screen.queryByText("正在读取当前 Definition…")).toBeNull();
  });

  it("把当前 generation 的 Details revision 不匹配收敛为安全冲突并结束加载", async function rejectMismatchedDefinitionRevision() {
    const fixture = createCommandGatewayFixture();
    fixture.gateway.getCommandBlock = vi.fn(async () => ({
      ...createCommandDetails(commandSummaries[0], "旧 revision"),
      revision: commandSummaries[0].revision + 1,
    }));
    render(
      <CommandWorkspace
        gateway={null}
        commandGateway={fixture.gateway}
        folderPicker={null}
      />,
    );

    const alert = await screen.findByRole("alert");
    expect(alert).toHaveProperty(
      "textContent",
      "REVISION_CONFLICTCommand Block 已更新，请重新载入",
    );
    expect(screen.getByText("定义读取失败", { selector: ".runner-facts strong" })).toBeDefined();
    expect(screen.queryByRole("textbox", { name: /文本/ })).toBeNull();
    expect(screen.queryByText("正在读取当前 Definition…")).toBeNull();
  });

  it("只按后端事件显示纯文本输出和自然终态", async function renderExecutionEvents() {
    const fixture = createGatewayFixture();
    const rendered = renderConnectedWorkspace(fixture.gateway);

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
    renderConnectedWorkspace(fixture.gateway);

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
    const rendered = renderConnectedWorkspace(fixture.gateway);
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
    renderConnectedWorkspace(fixture.gateway);

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
    renderConnectedWorkspace(fixture.gateway);

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

  it("Execution 活跃时锁定切换、输入、Picker 和移除，并在终态后恢复", async function lockConfigurationDuringExecution() {
    const executionFixture = createGatewayFixture({ deferStart: true });
    const commandFixture = createCommandGatewayFixture();
    const firstDefinition = commandFixture.details.get(commandSummaries[0].id);
    if (!firstDefinition) throw new Error("测试必须存在第一条 Definition");
    firstDefinition.parameters.push({
      type: "folders",
      key: "folders",
      label: "多个目录",
      description: null,
      required: false,
      remember: false,
      mustExist: true,
      minItems: 0,
      maxItems: 3,
      defaultValue: ["C:\\keep"],
    });
    const picker: FolderPicker = {
      /** 当前测试不使用单目录。 */
      pickFolder: vi.fn(async () => "C:\\new"),
      /** 当前测试不应在锁定期间打开多目录 Dialog。 */
      pickFolders: vi.fn(async () => ["C:\\new"]),
    };
    render(
      <CommandWorkspace
        gateway={executionFixture.gateway}
        commandGateway={commandFixture.gateway}
        folderPicker={picker}
      />,
    );
    await screen.findByRole("textbox", { name: /文本/ });

    fireEvent.click(screen.getByRole("button", { name: "运行验收任务" }));
    await waitFor(() => {
      expect((screen.getByRole("textbox", { name: /文本/ }) as HTMLInputElement).disabled).toBe(true);
      expect((screen.getByRole("button", { name: "添加多个目录" }) as HTMLButtonElement).disabled).toBe(true);
      expect((screen.getByRole("button", { name: "移除多个目录第 1 项" }) as HTMLButtonElement).disabled).toBe(true);
      expect((screen.getByRole("button", { name: /CMD 参数回显/ }) as HTMLButtonElement).disabled).toBe(true);
      expect((screen.getByRole("searchbox", { name: "搜索命令块" }) as HTMLInputElement).disabled).toBe(true);
    });
    fireEvent.click(screen.getByRole("button", { name: /CMD 参数回显/ }));
    expect(commandFixture.gateway.getCommandBlock).toHaveBeenCalledTimes(1);

    await act(async () => executionFixture.resolveStart());
    await act(async () => executionFixture.emit({ event: "started", data: { executionId: executionFixture.executionId, sequence: 0 } }));
    await act(async () => executionFixture.emit({ event: "finished", data: { executionId: executionFixture.executionId, sequence: 1, exitCode: 0, durationMs: 10, droppedOutputBytes: 0 } }));
    await waitFor(() => {
      expect((screen.getByRole("textbox", { name: /文本/ }) as HTMLInputElement).disabled).toBe(false);
      expect((screen.getByRole("button", { name: "添加多个目录" }) as HTMLButtonElement).disabled).toBe(false);
      expect((screen.getByRole("button", { name: "移除多个目录第 1 项" }) as HTMLButtonElement).disabled).toBe(false);
      expect((screen.getByRole("button", { name: /CMD 参数回显/ }) as HTMLButtonElement).disabled).toBe(false);
      expect((screen.getByRole("searchbox", { name: "搜索命令块" }) as HTMLInputElement).disabled).toBe(false);
    });
    expect(picker.pickFolders).not.toHaveBeenCalled();
  });

  it("取消响应和后端终态分别推进 Cancelling 与 Cancelled", async function cancelExecution() {
    const fixture = createGatewayFixture();
    renderConnectedWorkspace(fixture.gateway);

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
    renderConnectedWorkspace(fixture.gateway);

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
    renderConnectedWorkspace(fixture.gateway);

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
    renderConnectedWorkspace(fixture.gateway);

    fireEvent.click(screen.getByRole("button", { name: "运行验收任务" }));
    await waitFor(() => {
      expect(screen.getByRole("alert")).toHaveProperty("textContent", expect.stringContaining("PROCESS_START_FAILED"));
    });
    expect(screen.getByRole("button", { name: "运行验收任务" })).toBeDefined();
  });
});
