/** Command Workspace 的 Definition、可信 Preview、执行回归和宿主降级测试。 */
import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { StrictMode } from "react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { CommandWorkspace } from "../features/command-workspace/CommandWorkspace";
import type {
  CommandBlockDetails,
  CommandBlockSummary,
  CommandExecutionGateway,
  ExecutionStreamEvent,
  FixedExecutionGateway,
  PreviewCommandRequest,
  PreviewCommandResponse,
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
  /** 外部持有的拒绝器。 */
  let rejectPromise!: (reason?: unknown) => void;
  /** 当前挂起的 Promise。 */
  const promise = new Promise<T>((resolve, reject) => {
    resolvePromise = resolve;
    rejectPromise = reject;
  });
  return { promise, resolve: resolvePromise, reject: rejectPromise };
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

/** 为当前请求创建包含可核验 Hash、摘要和安全结论的 Rust Preview 响应。 */
function createPreviewResponse(
  request: PreviewCommandRequest,
  overrides: Partial<PreviewCommandResponse> = {},
): PreviewCommandResponse {
  return {
    commandBlockId: request.commandBlockId,
    revision: request.expectedRevision,
    runner: "windowsPowerShell",
    parameterSummaries: [
      {
        parameterKey: "text",
        label: "文本",
        displayValues: [String(request.parameterValues.text ?? "")],
        totalCount: 1,
        truncated: false,
      },
    ],
    previewText: "Write-Output 'PowerShell 默认'",
    fullSizeBytes: 38,
    truncated: false,
    riskLevel: "normal",
    actionLabel: "执行当前命令",
    safety: { state: "notApplicable", summary: null, warnings: [] },
    executionSpecHash: "a".repeat(64),
    ...overrides,
  };
}

/** 创建通用 Gateway Fixture，可精确替换 Definition 集合与 Preview 时序。 */
function createCommandGatewayFixture(options: {
  /** 当前列表按 Rust 顺序返回的摘要。 */
  summaries?: CommandBlockSummary[];
  /** 当前摘要对应的详情。 */
  details?: Map<string, CommandBlockDetails>;
  /** 当前测试使用的 Preview 实现。 */
  previewCommandBlock?: CommandExecutionGateway["previewCommandBlock"];
} = {}) {
  /** 当前 Fixture 返回的摘要集合。 */
  const summaries = options.summaries ?? commandSummaries;
  /** 当前两条 Summary 对应的 Details。 */
  const details = options.details ?? new Map<string, CommandBlockDetails>([
      [commandSummaries[0].id, createCommandDetails(commandSummaries[0], "PowerShell 默认")],
      [commandSummaries[1].id, createCommandDetails(commandSummaries[1], "CMD 默认")],
    ]);
  /** 通用 Gateway 的可观察替身。 */
  const gateway: CommandExecutionGateway = {
    /** 返回两条真实摘要的防御性数组。 */
    listCommandBlocks: vi.fn(async () => [...summaries]),
    /** 按业务 ID 返回当前测试详情。 */
    getCommandBlock: vi.fn(async (commandBlockId) => {
      const definition = details.get(commandBlockId);
      if (!definition) throw { code: "COMMAND_BLOCK_NOT_FOUND", message: "not found" };
      return definition;
    }),
    /** 按请求身份返回一份无副作用的可信 Preview。 */
    previewCommandBlock: vi.fn(
      options.previewCommandBlock ??
        (async function previewCurrentRequest(request) {
          return createPreviewResponse(request);
        }),
    ),
    /** Preview 流程不得提前调用通用 Run。 */
    runCommandBlock: vi.fn(async () => {
      throw new Error("Preview 流程不得调用通用 Run");
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

/** 等待当前 Preview 动作真实可用后再点击，避免把快照 effect 时序混入业务断言。 */
async function clickAvailablePreview(
  name: "生成 Preview" | "重试 Preview" | "重新生成 Preview" = "生成 Preview",
): Promise<void> {
  const button = await screen.findByRole("button", { name });
  await waitFor(() => expect(button).toHaveProperty("disabled", false));
  fireEvent.click(button);
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
    await waitFor(() => expect(screen.getByText("等待生成 Preview", { selector: ".runner-facts strong" })).toBeDefined());
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
    await waitFor(() =>
      expect(
        screen.getByText("等待生成 Preview", {
          selector: ".runner-facts strong",
        }),
      ).toBeDefined(),
    );
  });

  it("只用当前有效结构化快照请求 Rust Preview 并显示可信 Ready 证据", async function renderTrustedPreview() {
    const { commandFixture } = renderConnectedWorkspace();
    await screen.findByRole("textbox", { name: /文本/ });

    await waitFor(() => expect(screen.getByRole("button", { name: "生成 Preview" })).toHaveProperty("disabled", false));
    await clickAvailablePreview();

    await waitFor(() => {
      expect(commandFixture.gateway.previewCommandBlock).toHaveBeenCalledWith({
        commandBlockId: commandSummaries[0].id,
        expectedRevision: commandSummaries[0].revision,
        parameterValues: { text: "PowerShell 默认" },
      });
      expect(screen.getByText("Preview 已确认", { selector: ".runner-facts strong" })).toBeDefined();
    });
    expect(screen.getByText("Write-Output 'PowerShell 默认'")).toBeDefined();
    expect(screen.getByText("38 bytes")).toBeDefined();
    expect(screen.getByText("a".repeat(64))).toBeDefined();
    expect(screen.getByRole("button", { name: "执行当前命令" })).toHaveProperty("disabled", true);
    expect(screen.queryByRole("region", { name: "Safety Decision" })).toBeNull();
    expect(commandFixture.gateway.runCommandBlock).not.toHaveBeenCalled();
  });

  it("同一 React 提交中的双击只创建一个 Preview 请求", async function preventDuplicatePreviewRequest() {
    const pendingPreview = createDeferred<PreviewCommandResponse>();
    const fixture = createCommandGatewayFixture({
      previewCommandBlock: vi.fn(async function holdPreview() {
        return pendingPreview.promise;
      }),
    });
    render(
      <CommandWorkspace
        gateway={null}
        commandGateway={fixture.gateway}
        folderPicker={null}
      />,
    );
    const previewButton = await screen.findByRole("button", {
      name: "生成 Preview",
    });
    await waitFor(() =>
      expect(previewButton).toHaveProperty("disabled", false),
    );

    act(() => {
      previewButton.click();
      previewButton.click();
    });

    expect(fixture.gateway.previewCommandBlock).toHaveBeenCalledOnce();
    expect(fixture.gateway.runCommandBlock).not.toHaveBeenCalled();
  });

  it("参数写入先同步建立 pending 门禁并拒绝同一提交中的旧按钮事件", async function blockPreviewBeforeSnapshotEffect() {
    const fixture = createCommandGatewayFixture();
    render(
      <CommandWorkspace
        gateway={null}
        commandGateway={fixture.gateway}
        folderPicker={null}
      />,
    );
    const input = (await screen.findByRole("textbox", {
      name: /文本/,
    })) as HTMLInputElement;
    const previewButton = screen.getByRole("button", {
      name: "生成 Preview",
    });
    await waitFor(() =>
      expect(previewButton).toHaveProperty("disabled", false),
    );
    /** 原生 setter 让 input 与旧按钮事件处于同一 React 提交，Parameter Form effect 尚未交付新快照。 */
    const setNativeValue = Object.getOwnPropertyDescriptor(
      HTMLInputElement.prototype,
      "value",
    )?.set;
    if (!setNativeValue) {
      throw new Error("测试环境缺少 HTMLInputElement.value setter");
    }

    act(() => {
      setNativeValue.call(input, "pending current value");
      input.dispatchEvent(new Event("input", { bubbles: true }));
      previewButton.removeAttribute("disabled");
      previewButton.click();
    });

    expect(fixture.gateway.previewCommandBlock).not.toHaveBeenCalled();
    await waitFor(() =>
      expect(previewButton).toHaveProperty("disabled", false),
    );
    fireEvent.click(previewButton);
    await waitFor(() =>
      expect(fixture.gateway.previewCommandBlock).toHaveBeenCalledWith({
        commandBlockId: commandSummaries[0].id,
        expectedRevision: commandSummaries[0].revision,
        parameterValues: { text: "pending current value" },
      }),
    );
    expect(fixture.gateway.runCommandBlock).not.toHaveBeenCalled();
  });

  it("参数修改和改回原值都立即撤销旧 Preview 并丢弃迟到成功", async function invalidateLatePreviewSuccess() {
    const firstPreview = createDeferred<PreviewCommandResponse>();
    const secondPreview = createDeferred<PreviewCommandResponse>();
    const thirdPreview = createDeferred<PreviewCommandResponse>();
    /** 精确记录每次 Gateway 收到的独立结构化请求。 */
    const requests: PreviewCommandRequest[] = [];
    /** 按调用顺序返回三个受测试控制的 Preview。 */
    const deferredPreviews = [firstPreview, secondPreview, thirdPreview];
    const fixture = createCommandGatewayFixture({
      previewCommandBlock: vi.fn(async function previewInOrder(request) {
        requests.push(request);
        const deferred = deferredPreviews[requests.length - 1];
        if (!deferred) {
          throw new Error("测试只允许三次 Preview");
        }
        return deferred.promise;
      }),
    });
    render(
      <CommandWorkspace
        gateway={null}
        commandGateway={fixture.gateway}
        folderPicker={null}
      />,
    );
    const input = await screen.findByRole("textbox", { name: /文本/ });

    await clickAvailablePreview();
    await waitFor(() => expect(requests).toHaveLength(1));
    expect(requests[0].parameterValues).toEqual({ text: "PowerShell 默认" });
    fireEvent.change(input, { target: { value: "新的值" } });
    await waitFor(() =>
      expect(screen.getByRole("button", { name: "生成 Preview" })).toHaveProperty(
        "disabled",
        false,
      ),
    );
    await clickAvailablePreview();
    await waitFor(() => expect(requests).toHaveLength(2));
    expect(requests[1].parameterValues).toEqual({ text: "新的值" });
    await act(async () =>
      secondPreview.resolve(
        createPreviewResponse(requests[1], {
          previewText: "CURRENT_NEW_PREVIEW",
          executionSpecHash: "2".repeat(64),
        }),
      ),
    );
    expect(await screen.findByText("CURRENT_NEW_PREVIEW")).toBeDefined();
    await act(async () =>
      firstPreview.resolve(
        createPreviewResponse(requests[0], {
          previewText: "FIRST_LATE_PREVIEW",
          executionSpecHash: "1".repeat(64),
        }),
      ),
    );
    expect(screen.queryByText("FIRST_LATE_PREVIEW")).toBeNull();
    expect(screen.getByText("CURRENT_NEW_PREVIEW")).toBeDefined();

    fireEvent.change(input, { target: { value: "PowerShell 默认" } });
    expect(screen.queryByText("CURRENT_NEW_PREVIEW")).toBeNull();

    await waitFor(() =>
      expect(screen.getByRole("button", { name: "生成 Preview" })).toHaveProperty(
        "disabled",
        false,
      ),
    );
    await clickAvailablePreview();
    await waitFor(() => expect(requests).toHaveLength(3));
    await act(async () =>
      thirdPreview.resolve(
        createPreviewResponse(requests[2], {
          previewText: "CURRENT_PREVIEW",
          executionSpecHash: "3".repeat(64),
        }),
      ),
    );
    expect(await screen.findByText("CURRENT_PREVIEW")).toBeDefined();
    expect(screen.getByText("3".repeat(64))).toBeDefined();
    expect(fixture.gateway.runCommandBlock).not.toHaveBeenCalled();
  });

  it("A→B→相同 A 后丢弃第一次 A 的迟到 Preview 错误", async function discardRepeatedIdentityPreviewError() {
    const firstPreview = createDeferred<PreviewCommandResponse>();
    /** 第一次 A 挂起，后续请求直接返回当前身份的安全 Preview。 */
    let previewCall = 0;
    const fixture = createCommandGatewayFixture({
      previewCommandBlock: vi.fn(async function previewRepeatedIdentity(request) {
        previewCall += 1;
        if (previewCall === 1) {
          return firstPreview.promise;
        }
        return createPreviewResponse(request, { previewText: "CURRENT_A" });
      }),
    });
    render(
      <CommandWorkspace
        gateway={null}
        commandGateway={fixture.gateway}
        folderPicker={null}
      />,
    );
    await screen.findByDisplayValue("PowerShell 默认");
    await clickAvailablePreview();
    await waitFor(() =>
      expect(fixture.gateway.previewCommandBlock).toHaveBeenCalledTimes(1),
    );

    fireEvent.click(screen.getByRole("button", { name: /CMD 参数回显/ }));
    await screen.findByDisplayValue("CMD 默认");
    fireEvent.click(screen.getByRole("button", { name: /PowerShell 参数回显/ }));
    await screen.findByDisplayValue("PowerShell 默认");
    await act(async () =>
      firstPreview.reject({
        code: "VALIDATION_FAILED",
        message: "不应显示的旧错误",
        parameterKey: "text",
      }),
    );
    expect(screen.queryByText("请求参数未通过校验")).toBeNull();
    expect(screen.queryByText("不应显示的旧错误")).toBeNull();

    await clickAvailablePreview();
    expect(await screen.findByText("CURRENT_A")).toBeDefined();
    expect(fixture.gateway.runCommandBlock).not.toHaveBeenCalled();
  });

  it("只把当前已声明 key 的 Rust 错误映射到字段并在修改前同步清除", async function renderAndClearCurrentFieldError() {
    /** 第一次返回当前字段错误，第二次返回未知 key 的全局错误。 */
    let previewCall = 0;
    const fixture = createCommandGatewayFixture({
      previewCommandBlock: vi.fn(async function rejectPreviewParameters() {
        previewCall += 1;
        throw {
          code: "VALIDATION_FAILED",
          message: "后端原始文案不得显示",
          parameterKey: previewCall === 1 ? "text" : "unknown",
        };
      }),
    });
    render(
      <CommandWorkspace
        gateway={null}
        commandGateway={fixture.gateway}
        folderPicker={null}
      />,
    );
    const input = await screen.findByRole("textbox", { name: /文本/ });

    await clickAvailablePreview();
    const fieldError = await screen.findByText("请求参数未通过校验");
    expect(input.getAttribute("aria-invalid")).toBe("true");
    expect(input.getAttribute("aria-describedby")?.split(" ")).toContain(
      fieldError.id,
    );
    expect(screen.queryByText("后端原始文案不得显示")).toBeNull();

    fireEvent.change(input, { target: { value: "修正后的值" } });
    expect(screen.queryByText("请求参数未通过校验")).toBeNull();
    await waitFor(() => expect(input.getAttribute("aria-invalid")).toBe("false"));
    await clickAvailablePreview();
    const globalError = await screen.findByText("VALIDATION_FAILED");
    expect(globalError.closest(".execution-error")).not.toBeNull();
    expect(input.getAttribute("aria-invalid")).toBe("false");
  });

  it.each([
    ["REVISION_CONFLICT", "REVISION_CONFLICT"],
    ["STALE_PREVIEW", "STALE_PREVIEW"],
    ["UNSUPPORTED_RUNNER", "UNSUPPORTED_RUNNER"],
    ["NOT_PUBLISHED", "IPC_FAILED"],
  ] as const)(
    "把带当前 key 的 %s 仍作为工作区错误 %s",
    async function keepNonValidationErrorsGlobal(rejectedCode, visibleCode) {
      const fixture = createCommandGatewayFixture({
        previewCommandBlock: vi.fn(async function rejectNonValidationError() {
          throw {
            code: rejectedCode,
            message: "不得显示的原始错误",
            parameterKey: "text",
          };
        }),
      });
      render(
        <CommandWorkspace
          gateway={null}
          commandGateway={fixture.gateway}
          folderPicker={null}
        />,
      );
      const input = await screen.findByRole("textbox", { name: /文本/ });

      await clickAvailablePreview();
      const globalError = await screen.findByText(visibleCode);

      expect(globalError.closest(".execution-error")).not.toBeNull();
      expect(input.getAttribute("aria-invalid")).toBe("false");
      expect(screen.queryByText("不得显示的原始错误")).toBeNull();
    },
  );

  it("把 Preview 响应的错误 ID 与 revision 收敛为安全 IPC 失败", async function rejectPreviewIdentityMismatch() {
    /** 第一次篡改 ID，第二次篡改 revision。 */
    let previewCall = 0;
    const fixture = createCommandGatewayFixture({
      previewCommandBlock: vi.fn(async function returnMismatchedPreview(request) {
        previewCall += 1;
        return createPreviewResponse(
          request,
          previewCall === 1
            ? { commandBlockId: "builtin.other" }
            : { revision: request.expectedRevision + 1 },
        );
      }),
    });
    render(
      <CommandWorkspace
        gateway={null}
        commandGateway={fixture.gateway}
        folderPicker={null}
      />,
    );
    await screen.findByRole("textbox", { name: /文本/ });

    await clickAvailablePreview();
    expect(await screen.findByText("IPC_FAILED")).toBeDefined();
    expect(screen.queryByRole("button", { name: "执行当前命令" })).toBeNull();
    await clickAvailablePreview("重试 Preview");
    await waitFor(() =>
      expect(fixture.gateway.previewCommandBlock).toHaveBeenCalledTimes(2),
    );
    expect(screen.getByText("IPC_FAILED")).toBeDefined();
    expect(fixture.gateway.runCommandBlock).not.toHaveBeenCalled();
  });

  it("基础 UX 校验未通过时不允许请求 Rust Preview", async function blockInvalidParameterSnapshot() {
    const requiredDetails = createCommandDetails(commandSummaries[0], "");
    requiredDetails.parameters[0] = {
      type: "text",
      key: "text",
      label: "文本",
      description: "当前 Definition 的文本参数",
      required: true,
      remember: false,
      defaultValue: null,
      minLength: 1,
      maxLength: 32,
      placeholder: "输入文本",
    };
    const fixture = createCommandGatewayFixture({
      summaries: [commandSummaries[0]],
      details: new Map([[commandSummaries[0].id, requiredDetails]]),
    });
    render(
      <CommandWorkspace
        gateway={null}
        commandGateway={fixture.gateway}
        folderPicker={null}
      />,
    );
    const input = await screen.findByRole("textbox", { name: /文本/ });
    const previewButton = screen.getByRole("button", { name: "生成 Preview" });
    await waitFor(() => expect(previewButton).toHaveProperty("disabled", true));
    expect(fixture.gateway.previewCommandBlock).not.toHaveBeenCalled();

    fireEvent.change(input, { target: { value: "有效值" } });
    await waitFor(() => expect(previewButton).toHaveProperty("disabled", false));
    expect(fixture.gateway.previewCommandBlock).not.toHaveBeenCalled();
  });

  it("展示 passed Safety 并只使用 Rust actionLabel 形成 Ready 动作", async function renderPassedSafety() {
    const fixture = createCommandGatewayFixture({
      previewCommandBlock: vi.fn(async function returnPassedSafety(request) {
        return createPreviewResponse(request, {
          riskLevel: "destructive",
          actionLabel: "执行 Rust 已确认动作",
          safety: {
            state: "passed",
            summary: "Rust Safety 校验通过",
            warnings: [],
          },
        });
      }),
    });
    render(
      <CommandWorkspace
        gateway={null}
        commandGateway={fixture.gateway}
        folderPicker={null}
      />,
    );

    await clickAvailablePreview();
    const safety = await screen.findByRole("region", {
      name: "Safety Decision",
    });
    expect(safety.textContent).toContain("passed");
    expect(safety.textContent).toContain("Rust Safety 校验通过");
    expect(
      screen.getByRole("button", { name: "执行 Rust 已确认动作" }),
    ).toHaveProperty("disabled", true);
    expect(screen.getByText("Preview 已确认")).toBeDefined();
    expect(fixture.gateway.runCommandBlock).not.toHaveBeenCalled();
  });

  it("以纯文本显示 warning、摘要、URL、HTML、ANSI/OSC 与截断证据", async function renderUntrustedPreviewText() {
    /** 覆盖所有 Preview 文本字段的不可信字符矩阵。 */
    const previewText =
      "<b>Preview HTML</b> https://evil.invalid/path \u001b[31mRED\u001b[0m \u001b]8;;https://osc.invalid\u0007OSC\u001b]8;;\u0007";
    /** Rust 返回的完整 Execution Spec Hash。 */
    const fullHash = "9".repeat(64);
    const fixture = createCommandGatewayFixture({
      previewCommandBlock: vi.fn(async function returnWarningText(request) {
        return createPreviewResponse(request, {
          parameterSummaries: [
            {
              parameterKey: "text",
              label: "<img src=x onerror=alert(1)>",
              displayValues: [
                "https://summary.invalid/path",
                "\u001b[32mSUMMARY\u001b[0m",
              ],
              totalCount: 9,
              truncated: true,
            },
            {
              parameterKey: "second",
              label: "第二项",
              displayValues: ["保持 Rust 次序"],
              totalCount: 1,
              truncated: false,
            },
          ],
          previewText,
          fullSizeBytes: 4096,
          truncated: true,
          riskLevel: "destructive",
          actionLabel: "<strong>Rust 动作</strong>",
          safety: {
            state: "warning",
            summary: "<script>Safety summary</script>",
            warnings: [
              {
                code: "WARN_HTML",
                message:
                  "https://warning.invalid \u001b]8;;https://warning.invalid\u0007link\u001b]8;;\u0007",
              },
            ],
          },
          executionSpecHash: fullHash,
        });
      }),
    });
    render(
      <CommandWorkspace
        gateway={null}
        commandGateway={fixture.gateway}
        folderPicker={null}
      />,
    );

    await clickAvailablePreview();
    const preview = await screen.findByLabelText("Rust 生成的 Preview 文本");
    expect(preview.textContent).toBe(previewText);
    expect(preview.querySelector("*")).toBeNull();
    expect(
      Array.from(
        screen
          .getByLabelText("Rust 规范化参数摘要")
          .querySelectorAll("dt"),
      ).map((term) => term.textContent),
    ).toEqual(["<img src=x onerror=alert(1)>", "第二项"]);
    expect(screen.getByText("<img src=x onerror=alert(1)>")).toBeDefined();
    expect(screen.getByText("https://summary.invalid/path")).toBeDefined();
    expect(screen.getByText("保持 Rust 次序")).toBeDefined();
    expect(screen.getByText("9 项 · 摘要已截断")).toBeDefined();
    expect(screen.getByText("4096 bytes")).toBeDefined();
    expect(screen.getByText(fullHash)).toBeDefined();
    expect(
      screen.getByText(
        "当前可见 Preview 文本已截断；完整大小与 Hash 仍对应 Rust Core 的完整 Artifact。",
      ),
    ).toBeDefined();
    const safety = screen.getByRole("region", { name: "Safety Decision" });
    expect(safety.textContent).toContain("warning");
    expect(safety.textContent).toContain("<script>Safety summary</script>");
    expect(safety.textContent).toContain("https://warning.invalid");
    expect(safety.querySelector("script, a, img")).toBeNull();
    const action = screen.getByRole("button", {
      name: "<strong>Rust 动作</strong>",
    });
    expect(action.querySelector("strong")).toBeNull();
    expect(
      document.querySelector('a[href="https://evil.invalid/path"]'),
    ).toBeNull();
    expect(fixture.gateway.runCommandBlock).not.toHaveBeenCalled();
  });

  it("blocked Safety 只保留展示证据且不创建可执行动作", async function renderBlockedSafety() {
    const fixture = createCommandGatewayFixture({
      previewCommandBlock: vi.fn(async function returnBlockedSafety(request) {
        return createPreviewResponse(request, {
          riskLevel: "destructive",
          actionLabel: "不得出现的执行动作",
          safety: {
            state: "blocked",
            summary: "Rust 已拦截当前内容",
            warnings: [
              { code: "BLOCKED_TARGET", message: "请修改当前参数" },
            ],
          },
        });
      }),
    });
    render(
      <CommandWorkspace
        gateway={null}
        commandGateway={fixture.gateway}
        folderPicker={null}
      />,
    );

    await clickAvailablePreview();
    const safety = await screen.findByRole("region", {
      name: "Safety Decision",
    });
    expect(safety.textContent).toContain("blocked");
    expect(safety.textContent).toContain("Rust 已拦截当前内容");
    expect(safety.textContent).toContain("BLOCKED_TARGET");
    expect(screen.getByText("Preview 已拦截")).toBeDefined();
    expect(
      screen.queryByRole("button", { name: "不得出现的执行动作" }),
    ).toBeNull();
    expect(screen.getByRole("button", { name: "重试 Preview" })).toBeDefined();
    expect(fixture.gateway.runCommandBlock).not.toHaveBeenCalled();
  });

  it("Parameterless 在 Strict Mode 中每个 Definition generation 自动 Preview 一次并拒绝重复手动请求", async function autoPreviewParameterlessOnce() {
    const parameterlessSummary: CommandBlockSummary = {
      ...commandSummaries[0],
      id: "builtin.parameterless.strict",
      name: "无参数严格预览",
    };
    const parameterlessDetails: CommandBlockDetails = {
      ...parameterlessSummary,
      parameters: [],
    };
    const pendingPreview = createDeferred<PreviewCommandResponse>();
    const fixture = createCommandGatewayFixture({
      summaries: [parameterlessSummary],
      details: new Map([[parameterlessSummary.id, parameterlessDetails]]),
      previewCommandBlock: vi.fn(async function holdParameterlessPreview() {
        return pendingPreview.promise;
      }),
    });
    const rendered = render(
      <StrictMode>
        <CommandWorkspace
          gateway={null}
          commandGateway={fixture.gateway}
          folderPicker={null}
        />
      </StrictMode>,
    );

    await waitFor(() =>
      expect(fixture.gateway.previewCommandBlock).toHaveBeenCalledOnce(),
    );
    expect(fixture.gateway.previewCommandBlock).toHaveBeenCalledWith({
      commandBlockId: parameterlessSummary.id,
      expectedRevision: parameterlessSummary.revision,
      parameterValues: {},
    });
    expect(screen.queryByRole("form", { name: "类型化参数" })).toBeNull();
    /** 模拟旧按钮事件绕过 disabled；同步 active-request guard 仍必须阻止重复调用。 */
    const pendingButton = screen.getByRole("button", {
      name: "正在生成 Preview",
    });
    pendingButton.removeAttribute("disabled");
    fireEvent.click(pendingButton);
    expect(fixture.gateway.previewCommandBlock).toHaveBeenCalledOnce();
    expect(fixture.gateway.runCommandBlock).not.toHaveBeenCalled();

    rendered.unmount();
    await act(async () =>
      pendingPreview.resolve(
        createPreviewResponse({
          commandBlockId: parameterlessSummary.id,
          expectedRevision: parameterlessSummary.revision,
          parameterValues: {},
        }),
      ),
    );
    expect(fixture.gateway.runCommandBlock).not.toHaveBeenCalled();
  });

  it("Parameterless 自动 Preview 失败后只允许手动重试且永不自动 Run", async function retryFailedParameterlessPreview() {
    const parameterlessSummary: CommandBlockSummary = {
      ...commandSummaries[0],
      id: "builtin.parameterless.retry",
      name: "无参数重试",
    };
    const parameterlessDetails: CommandBlockDetails = {
      ...parameterlessSummary,
      parameters: [],
    };
    /** 第一次自动请求失败，第二次手动请求成功。 */
    let previewCall = 0;
    const fixture = createCommandGatewayFixture({
      summaries: [parameterlessSummary],
      details: new Map([[parameterlessSummary.id, parameterlessDetails]]),
      previewCommandBlock: vi.fn(async function retryParameterless(request) {
        previewCall += 1;
        if (previewCall === 1) {
          throw { code: "STALE_PREVIEW", message: "旧 Preview" };
        }
        return createPreviewResponse(request, {
          previewText: "PARAMETERLESS_CURRENT",
        });
      }),
    });
    render(
      <CommandWorkspace
        gateway={null}
        commandGateway={fixture.gateway}
        folderPicker={null}
      />,
    );

    expect(await screen.findByText("STALE_PREVIEW")).toBeDefined();
    expect(fixture.gateway.previewCommandBlock).toHaveBeenCalledOnce();
    await act(async () => Promise.resolve());
    expect(fixture.gateway.previewCommandBlock).toHaveBeenCalledOnce();
    await clickAvailablePreview("重试 Preview");
    expect(await screen.findByText("PARAMETERLESS_CURRENT")).toBeDefined();
    expect(fixture.gateway.previewCommandBlock).toHaveBeenCalledTimes(2);
    expect(fixture.gateway.runCommandBlock).not.toHaveBeenCalled();
  });

  it("Parameterless A→B→相同 A 为每个 Definition generation 各自动 Preview 一次", async function autoPreviewEveryParameterlessGeneration() {
    const summaries = commandSummaries.map((summary) => ({
      ...summary,
      name: `${summary.name} 无参数`,
    }));
    const details = new Map(
      summaries.map((summary) => [
        summary.id,
        { ...summary, parameters: [] } satisfies CommandBlockDetails,
      ]),
    );
    const fixture = createCommandGatewayFixture({ summaries, details });
    render(
      <CommandWorkspace
        gateway={null}
        commandGateway={fixture.gateway}
        folderPicker={null}
      />,
    );

    await waitFor(() =>
      expect(fixture.gateway.previewCommandBlock).toHaveBeenCalledTimes(1),
    );
    fireEvent.click(
      screen.getByRole("button", { name: /CMD 参数回显 无参数/ }),
    );
    await waitFor(() =>
      expect(fixture.gateway.previewCommandBlock).toHaveBeenCalledTimes(2),
    );
    fireEvent.click(
      screen.getByRole("button", { name: /PowerShell 参数回显 无参数/ }),
    );
    await waitFor(() =>
      expect(fixture.gateway.previewCommandBlock).toHaveBeenCalledTimes(3),
    );
    expect(
      vi.mocked(fixture.gateway.previewCommandBlock).mock.calls.map(
        ([request]) => request,
      ),
    ).toEqual([
      {
        commandBlockId: summaries[0].id,
        expectedRevision: summaries[0].revision,
        parameterValues: {},
      },
      {
        commandBlockId: summaries[1].id,
        expectedRevision: summaries[1].revision,
        parameterValues: {},
      },
      {
        commandBlockId: summaries[0].id,
        expectedRevision: summaries[0].revision,
        parameterValues: {},
      },
    ]);
    expect(fixture.gateway.runCommandBlock).not.toHaveBeenCalled();
  });

  it("通用桌面 Gateway 已连接但固定 Execution 未接线时只开放 Preview", async function showPendingRunWiring() {
    renderConnectedWorkspace();
    await screen.findByRole("heading", { name: "PowerShell 参数回显", level: 1 });

    expect(screen.queryByText("需要桌面宿主")).toBeNull();
    expect(screen.queryByText(/纯浏览器环境/)).toBeNull();
    expect(screen.getAllByText("Run 尚未接线").length).toBeGreaterThan(0);
    await waitFor(() => expect(screen.getByRole("button", { name: "生成 Preview" })).toHaveProperty("disabled", false));
    expect(screen.queryByRole("button", { name: "运行尚未接线" })).toBeNull();
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
