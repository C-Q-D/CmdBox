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
  PreviewCommandRequest,
  PreviewCommandResponse,
  VerifyRunRequest,
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

/** 通用 Gateway Fixture 的 Definition、Preview、Run 与 Cancel 时序选项。 */
interface CommandGatewayFixtureOptions {
  /** 当前列表按 Rust 顺序返回的摘要。 */
  summaries?: CommandBlockSummary[];
  /** 当前摘要对应的详情。 */
  details?: Map<string, CommandBlockDetails>;
  /** 当前测试使用的 Preview 实现。 */
  previewCommandBlock?: CommandExecutionGateway["previewCommandBlock"];
  /** Run 响应使用的测试 Execution UUID。 */
  executionId?: string;
  /** 是否挂起 Run 响应供测试先推送 Channel 事件。 */
  deferRun?: boolean;
  /** Run 响应需要抛出的公开或未知拒绝值。 */
  runFailure?: unknown;
  /** 是否挂起 Cancel 响应供测试制造终态竞态。 */
  deferCancel?: boolean;
  /** Cancel 响应需要抛出的拒绝值。 */
  cancelFailure?: unknown;
  /** Cancel 成功时返回的精确 Rust 事实。 */
  cancelResponse?: Awaited<
    ReturnType<CommandExecutionGateway["cancelExecution"]>
  >;
}

/** 创建可主动推送 Channel 事件并控制 Run/Cancel 时序的通用 Gateway Fixture。 */
function createCommandGatewayFixture(
  options: CommandGatewayFixtureOptions = {},
) {
  /** 当前 Fixture 返回的摘要集合。 */
  const summaries = options.summaries ?? commandSummaries;
  /** 当前两条 Summary 对应的 Details。 */
  const details = options.details ?? new Map<string, CommandBlockDetails>([
      [commandSummaries[0].id, createCommandDetails(commandSummaries[0], "PowerShell 默认")],
      [commandSummaries[1].id, createCommandDetails(commandSummaries[1], "CMD 默认")],
    ]);
  /** 当前 Run 响应使用的稳定 Execution UUID。 */
  const executionId =
    options.executionId ?? "9be8ec5d-ef8c-4c2a-a7f5-12069b2ad555";
  /** 每个 Run 调用各自注册的专属 Channel 回调。 */
  const eventHandlers: Array<(event: ExecutionStreamEvent) => void> = [];
  /** Gateway 收到的每份深复制 Run 请求。 */
  const runRequests: VerifyRunRequest[] = [];
  /** Cancel 调用收到的可信 Execution UUID。 */
  const cancelledExecutionIds: string[] = [];
  /** 测试控制 Run 响应时序的外部解析器。 */
  let resolveRunResponse: (() => void) | undefined;
  /** 测试控制 Cancel 响应时序的外部解析器。 */
  let resolveCancelResponse: (() => void) | undefined;
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
    /** 保存本次结构化请求和专属 Channel，并按测试选项返回或拒绝。 */
    runCommandBlock: vi.fn(async (request, onEvent) => {
      runRequests.push(request);
      eventHandlers.push(onEvent);
      if (options.deferRun) {
        await new Promise<void>((resolve) => {
          resolveRunResponse = resolve;
        });
      }
      if (options.runFailure !== undefined) {
        throw options.runFailure;
      }
      return { executionId };
    }),
    /** 记录目标 UUID，并按测试选项返回幂等事实、失败或挂起结果。 */
    cancelExecution: vi.fn(async (targetExecutionId) => {
      cancelledExecutionIds.push(targetExecutionId);
      if (options.deferCancel) {
        await new Promise<void>((resolve) => {
          resolveCancelResponse = resolve;
        });
      }
      if (options.cancelFailure !== undefined) {
        throw options.cancelFailure;
      }
      return (
        options.cancelResponse ?? { accepted: true, state: "cancelling" as const }
      );
    }),
  };
  return {
    gateway,
    details,
    executionId,
    runRequests,
    eventHandlers,
    cancelledExecutionIds,
    /** 向指定 Run 的专属 Channel 推送一个后端事件。 */
    emit(event: ExecutionStreamEvent, runIndex = eventHandlers.length - 1) {
      const handler = eventHandlers[runIndex];
      if (!handler) {
        throw new Error("应先请求 Run 再推送事件");
      }
      handler(event);
    },
    /** 释放当前挂起的 Run 响应。 */
    resolveRun() {
      resolveRunResponse?.();
    },
    /** 释放当前挂起的 Cancel 响应。 */
    resolveCancel() {
      resolveCancelResponse?.();
    },
  };
}

/** 渲染只接入唯一通用 Gateway 的真实 Definition 工作区。 */
function renderConnectedWorkspace(
  commandFixture = createCommandGatewayFixture(),
) {
  const rendered = render(
    <CommandWorkspace
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

/** 完成默认 Definition 的 Preview，并等待 Rust actionLabel 变为一次性 Run 动作。 */
async function prepareConfirmedPreview(): Promise<HTMLButtonElement> {
  await screen.findByRole("textbox", { name: /文本/ });
  await clickAvailablePreview();
  return screen.findByRole("button", { name: "执行当前命令" });
}

/** 完成 Preview 并点击一次通用 Run。 */
async function startConfirmedExecution(): Promise<void> {
  const runButton = await prepareConfirmedPreview();
  fireEvent.click(runButton);
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
    render(<CommandWorkspace windowControls={windowControls} />);

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
    expect(screen.getByRole("button", { name: "执行当前命令" })).toHaveProperty("disabled", false);
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
    ).toHaveProperty("disabled", false);
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

  it("通用桌面 Gateway 已连接时先开放 Preview 再开放一次性 Run", async function showPreviewThenRun() {
    renderConnectedWorkspace();
    await screen.findByRole("heading", { name: "PowerShell 参数回显", level: 1 });

    expect(screen.queryByText("需要桌面宿主")).toBeNull();
    expect(screen.queryByText(/纯浏览器环境/)).toBeNull();
    await waitFor(() => expect(screen.getByRole("button", { name: "生成 Preview" })).toHaveProperty("disabled", false));
    expect(screen.queryByRole("button", { name: "执行当前命令" })).toBeNull();
    await clickAvailablePreview();
    expect(await screen.findByRole("button", { name: "执行当前命令" })).toHaveProperty("disabled", false);
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
      <CommandWorkspace commandGateway={fixture.gateway} folderPicker={null} />,
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
    const lateRendered = render(<CommandWorkspace commandGateway={unmountFixture.gateway} folderPicker={null} />);
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

  it("同一提交双击只消费一次确认快照且 Run 响应不伪造 Started", async function consumePreviewOnce() {
    const fixture = createCommandGatewayFixture({ deferRun: true });
    renderConnectedWorkspace(fixture);
    const runButton = await prepareConfirmedPreview();
    /** Preview Gateway 收到的参数图用于验证 Run 再次隔离。 */
    const previewRequest = vi.mocked(fixture.gateway.previewCommandBlock).mock
      .calls[0]?.[0];

    await act(async () => {
      runButton.click();
      runButton.click();
    });

    expect(fixture.gateway.runCommandBlock).toHaveBeenCalledOnce();
    expect(fixture.eventHandlers).toHaveLength(1);
    expect(fixture.runRequests).toEqual([
      {
        commandBlockId: commandSummaries[0].id,
        expectedRevision: 1,
        parameterValues: { text: "PowerShell 默认" },
        executionSpecHash: "a".repeat(64),
      },
    ]);
    expect(fixture.runRequests[0]?.parameterValues).not.toBe(
      previewRequest?.parameterValues,
    );
    expect(Object.isFrozen(fixture.runRequests[0])).toBe(true);
    expect(screen.queryByText("a".repeat(64))).toBeNull();
    expect(screen.queryByRole("button", { name: "执行当前命令" })).toBeNull();
    expect(screen.getAllByText("正在建立执行").length).toBeGreaterThan(0);

    await act(async () => fixture.resolveRun());
    await waitFor(() => expect(screen.getByText(fixture.executionId)).toBeDefined());
    expect(screen.getAllByText("正在建立执行").length).toBeGreaterThan(0);
    expect(screen.queryByText("运行中")).toBeNull();
    expect(screen.getByRole("button", { name: "终止任务" })).toBeDefined();
  });

  it("只按后端事件显示纯文本输出和自然终态", async function renderExecutionEvents() {
    const fixture = createCommandGatewayFixture();
    const rendered = renderConnectedWorkspace(fixture);

    await startConfirmedExecution();
    await waitFor(() => expect(screen.getByText(fixture.executionId)).toBeDefined());
    await act(async () => {
      fixture.emit({ event: "started", data: { executionId: fixture.executionId, sequence: 0 } });
      fixture.emit({
        event: "output",
        data: {
          executionId: fixture.executionId,
          sequence: 1,
          fragments: [
            { fragmentSequence: 0, stream: "stdout", text: "<b>plain html</b> https://example.invalid \u001b[31mANSI \u001b]8;;https://osc.invalid\u0007OSC\u001b]8;;\u0007" },
            { fragmentSequence: 1, stream: "stderr", text: "" },
          ],
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
        data: { executionId: fixture.executionId, sequence: 2, exitCode: 7, outcome: "failure", durationMs: 8123, droppedOutputBytes: 0 },
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
        data: { executionId: fixture.executionId, sequence: 4, message: "终态不得覆盖", outcome: "none", durationMs: 9000, droppedOutputBytes: 0 },
      });
    });

    expect(screen.getByText(/<b>plain html<\/b>/)).toBeDefined();
    expect(rendered.container.querySelector(".execution-output b")).toBeNull();
    expect(screen.queryByRole("link", { name: /example\.invalid/ })).toBeNull();
    expect(screen.queryByRole("link", { name: /osc\.invalid/ })).toBeNull();
    expect(rendered.container.querySelectorAll(".execution-output .output-line")).toHaveLength(1);
    expect(screen.queryByText("重复事件不得显示")).toBeNull();
    expect(screen.getByText("任务自然结束")).toBeDefined();
    expect(screen.getByText("8123 ms")).toBeDefined();
    expect(screen.getByText("7", { selector: "dd" })).toBeDefined();
    expect(screen.queryByText("终态后输出不得显示")).toBeNull();
    expect(screen.queryByText("任务内部失败")).toBeNull();
    expect(screen.queryByText(/Outcome/)).toBeNull();
    expect(screen.getByRole("button", { name: "重新生成 Preview" })).toBeDefined();
    expect(screen.queryByRole("button", { name: "执行当前命令" })).toBeNull();
  });

  it("只在启动响应锁定 Execution ID 后重放匹配事件", async function lockResponseExecutionId() {
    const fixture = createCommandGatewayFixture({ deferRun: true });
    renderConnectedWorkspace(fixture);

    await startConfirmedExecution();
    await act(async () => {
      fixture.emit({ event: "started", data: { executionId: "11111111-1111-4111-8111-111111111111", sequence: 0 } });
      fixture.emit({ event: "finished", data: { executionId: "11111111-1111-4111-8111-111111111111", sequence: 1, exitCode: 0, outcome: "success", durationMs: 1, droppedOutputBytes: 0 } });
    });
    expect(screen.queryByText("11111111-1111-4111-8111-111111111111")).toBeNull();
    expect(screen.queryByText("任务自然结束")).toBeNull();

    await act(async () => fixture.resolveRun());
    await waitFor(() => expect(screen.getByText(fixture.executionId)).toBeDefined());
    expect(screen.queryByText("任务自然结束")).toBeNull();
  });

  it("把启动响应前的可信与错误 ID 输出共同限制在 512 KiB 内", async function boundPreResponseEvents() {
    const fixture = createCommandGatewayFixture({ deferRun: true });
    const rendered = renderConnectedWorkspace(fixture);
    const chunk = "x".repeat(64 * 1024);
    const wrongExecutionId = "22222222-2222-4222-8222-222222222222";

    await startConfirmedExecution();
    await act(async () => {
      fixture.emit({ event: "started", data: { executionId: fixture.executionId, sequence: 0 } });
      for (let sequence = 1; sequence <= 13; sequence += 1) {
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
      fixture.resolveRun();
    });

    await waitFor(() => expect(screen.getAllByText("运行中").length).toBeGreaterThan(0));
    const retainedText = Array.from(rendered.container.querySelectorAll(".execution-output pre"))
      .map((element) => element.textContent ?? "")
      .join("");
    expect(new TextEncoder().encode(retainedText).byteLength).toBeLessThanOrEqual(512 * 1024);
    expect(screen.getByText(/早期实时输出已有 \d+ 字节未保留/)).toBeDefined();
  });

  it("不把错误 Execution ID 的预响应淘汰计入当前任务", async function ignoreWrongIdDrops() {
    const fixture = createCommandGatewayFixture({ deferRun: true });
    renderConnectedWorkspace(fixture);

    await startConfirmedExecution();
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
      fixture.resolveRun();
    });

    await waitFor(() => expect(screen.getByText(fixture.executionId)).toBeDefined());
    expect(screen.queryByText(/早期实时输出已有 \d+ 字节未保留/)).toBeNull();
  });

  it("完整记录匹配 Execution 被淘汰事件的文本与 Rust 丢弃字节", async function preserveAuthenticatedDrops() {
    const fixture = createCommandGatewayFixture({ deferRun: true });
    renderConnectedWorkspace(fixture);

    await startConfirmedExecution();
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
      fixture.resolveRun();
    });

    await waitFor(() => {
      expect(screen.getByText("早期实时输出已有 615400 字节未保留；外部任务未因此阻塞。")).toBeDefined();
    });
  });

  it("Execution 活跃时锁定切换、输入、Picker 和移除，并在终态后恢复", async function lockConfigurationDuringExecution() {
    const commandFixture = createCommandGatewayFixture({ deferRun: true });
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
        commandGateway={commandFixture.gateway}
        folderPicker={picker}
      />,
    );
    await startConfirmedExecution();
    await waitFor(() => {
      expect((screen.getByRole("textbox", { name: /文本/ }) as HTMLInputElement).disabled).toBe(true);
      expect((screen.getByRole("button", { name: "添加多个目录" }) as HTMLButtonElement).disabled).toBe(true);
      expect((screen.getByRole("button", { name: "移除多个目录第 1 项" }) as HTMLButtonElement).disabled).toBe(true);
      expect((screen.getByRole("button", { name: /CMD 参数回显/ }) as HTMLButtonElement).disabled).toBe(true);
      expect((screen.getByRole("searchbox", { name: "搜索命令块" }) as HTMLInputElement).disabled).toBe(true);
      expect((screen.getByRole("button", { name: "生成 Preview" }) as HTMLButtonElement).disabled).toBe(true);
    });
    fireEvent.click(screen.getByRole("button", { name: /CMD 参数回显/ }));
    expect(commandFixture.gateway.getCommandBlock).toHaveBeenCalledTimes(1);

    await act(async () => commandFixture.resolveRun());
    await act(async () => commandFixture.emit({ event: "started", data: { executionId: commandFixture.executionId, sequence: 0 } }));
    await act(async () => commandFixture.emit({ event: "finished", data: { executionId: commandFixture.executionId, sequence: 1, exitCode: 0, outcome: "success", durationMs: 10, droppedOutputBytes: 0 } }));
    await waitFor(() => {
      expect((screen.getByRole("textbox", { name: /文本/ }) as HTMLInputElement).disabled).toBe(false);
      expect((screen.getByRole("button", { name: "添加多个目录" }) as HTMLButtonElement).disabled).toBe(false);
      expect((screen.getByRole("button", { name: "移除多个目录第 1 项" }) as HTMLButtonElement).disabled).toBe(false);
      expect((screen.getByRole("button", { name: /CMD 参数回显/ }) as HTMLButtonElement).disabled).toBe(false);
      expect((screen.getByRole("searchbox", { name: "搜索命令块" }) as HTMLInputElement).disabled).toBe(false);
      expect((screen.getByRole("button", { name: "重新生成 Preview" }) as HTMLButtonElement).disabled).toBe(false);
    });
    expect(picker.pickFolders).not.toHaveBeenCalled();
  });

  it("取消响应和后端终态分别推进 Cancelling 与 Cancelled", async function cancelExecution() {
    const fixture = createCommandGatewayFixture();
    renderConnectedWorkspace(fixture);

    await startConfirmedExecution();
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
        data: { executionId: fixture.executionId, sequence: 1, outcome: "none", durationMs: 950, droppedOutputBytes: 0 },
      });
    });
    expect(screen.getByText("任务已取消")).toBeDefined();
    expect(screen.getByRole("button", { name: "重新生成 Preview" })).toBeDefined();
    expect(screen.queryByRole("button", { name: "执行当前命令" })).toBeNull();
  });

  it("Starting 获得响应 ID 后双击只取消一次且 Started 不倒退 Cancelling", async function cancelDuringStarting() {
    const fixture = createCommandGatewayFixture({
      deferCancel: true,
      cancelResponse: { accepted: false, state: "cancelling" },
    });
    renderConnectedWorkspace(fixture);
    await startConfirmedExecution();
    const cancelButton = await screen.findByRole("button", {
      name: "终止任务",
    });

    await act(async () => {
      cancelButton.click();
      cancelButton.click();
    });
    expect(fixture.gateway.cancelExecution).toHaveBeenCalledOnce();
    expect(fixture.cancelledExecutionIds).toEqual([fixture.executionId]);

    await act(async () => fixture.resolveCancel());
    expect(screen.getAllByText("正在终止进程树").length).toBeGreaterThan(0);
    await act(async () => {
      fixture.emit({
        event: "started",
        data: { executionId: fixture.executionId, sequence: 0 },
      });
    });
    expect(screen.getAllByText("正在终止进程树").length).toBeGreaterThan(0);
    expect(screen.queryByText("运行中")).toBeNull();
  });

  it("Cancel null 保持生命周期并释放同步 token 允许重试", async function retryNullCancel() {
    const fixture = createCommandGatewayFixture({
      cancelResponse: { accepted: false, state: null },
    });
    renderConnectedWorkspace(fixture);
    await startConfirmedExecution();
    const firstCancelButton = await screen.findByRole("button", {
      name: "终止任务",
    });
    fireEvent.click(firstCancelButton);
    await waitFor(() =>
      expect(fixture.gateway.cancelExecution).toHaveBeenCalledTimes(1),
    );
    const retryButton = await screen.findByRole("button", {
      name: "终止任务",
    });
    await waitFor(() => expect(retryButton).toHaveProperty("disabled", false));
    fireEvent.click(retryButton);
    await waitFor(() =>
      expect(fixture.gateway.cancelExecution).toHaveBeenCalledTimes(2),
    );

    expect(screen.getAllByText("正在建立执行").length).toBeGreaterThan(0);
    expect(screen.queryByText("正在终止进程树")).toBeNull();
  });

  it("Cancel 失败只显示固定公开说明并保持 Running 可重试", async function retryFailedCancel() {
    const fixture = createCommandGatewayFixture({
      cancelFailure: {
        code: "CANCEL_FAILED",
        message: String.raw`C:\private\cancel-secret`,
      },
    });
    renderConnectedWorkspace(fixture);
    await startConfirmedExecution();
    await waitFor(() => expect(screen.getByText(fixture.executionId)).toBeDefined());
    await act(async () => {
      fixture.emit({
        event: "started",
        data: { executionId: fixture.executionId, sequence: 0 },
      });
    });
    fireEvent.click(screen.getByRole("button", { name: "终止任务" }));
    await waitFor(() =>
      expect(screen.getByRole("alert")).toHaveProperty(
        "textContent",
        "CANCEL_FAILED无法终止当前 Execution",
      ),
    );
    expect(screen.queryByText(/cancel-secret/)).toBeNull();
    expect(screen.getAllByText("运行中").length).toBeGreaterThan(0);
    const retryButton = screen.getByRole("button", { name: "终止任务" });
    await waitFor(() => expect(retryButton).toHaveProperty("disabled", false));
    fireEvent.click(retryButton);
    await waitFor(() =>
      expect(fixture.gateway.cancelExecution).toHaveBeenCalledTimes(2),
    );
  });

  it("新 Run generation 拒绝旧 Channel 即使复用同一 Execution ID", async function isolateRunGenerations() {
    const fixture = createCommandGatewayFixture();
    renderConnectedWorkspace(fixture);
    await startConfirmedExecution();
    await waitFor(() => expect(fixture.eventHandlers).toHaveLength(1));
    await act(async () => {
      fixture.emit({
        event: "started",
        data: { executionId: fixture.executionId, sequence: 0 },
      });
      fixture.emit({
        event: "finished",
        data: {
          executionId: fixture.executionId,
          sequence: 1,
          exitCode: 0,
          outcome: "success",
          durationMs: 5,
          droppedOutputBytes: 0,
        },
      });
    });
    await clickAvailablePreview("重新生成 Preview");
    const secondRunButton = await screen.findByRole("button", {
      name: "执行当前命令",
    });
    fireEvent.click(secondRunButton);
    await waitFor(() => expect(fixture.eventHandlers).toHaveLength(2));

    await act(async () => {
      fixture.emit(
        {
          event: "output",
          data: {
            executionId: fixture.executionId,
            sequence: 100,
            fragments: [
              {
                fragmentSequence: 100,
                stream: "stderr",
                text: "旧 generation 输出",
              },
            ],
            droppedBytesBefore: 0,
          },
        },
        0,
      );
      fixture.emit(
        {
          event: "failed",
          data: {
            executionId: fixture.executionId,
            sequence: 101,
            message: "旧 generation 终态",
            outcome: "none",
            durationMs: 99,
            droppedOutputBytes: 0,
          },
        },
        0,
      );
      fixture.emit(
        {
          event: "started",
          data: { executionId: fixture.executionId, sequence: 0 },
        },
        1,
      );
      fixture.emit(
        {
          event: "output",
          data: {
            executionId: fixture.executionId,
            sequence: 1,
            fragments: [
              {
                fragmentSequence: 1,
                stream: "stdout",
                text: "当前 generation 输出",
              },
            ],
            droppedBytesBefore: 0,
          },
        },
        1,
      );
    });

    expect(screen.queryByText("旧 generation 输出")).toBeNull();
    expect(screen.queryByText("旧 generation 终态")).toBeNull();
    expect(screen.getByText("当前 generation 输出")).toBeDefined();
    expect(screen.queryByText("任务内部失败")).toBeNull();
  });

  it("终态不被较晚的取消响应倒退", async function isolateLateCancelResponse() {
    const fixture = createCommandGatewayFixture({ deferCancel: true });
    renderConnectedWorkspace(fixture);

    await startConfirmedExecution();
    await waitFor(() => expect(screen.getByText(fixture.executionId)).toBeDefined());
    await act(async () => fixture.emit({ event: "started", data: { executionId: fixture.executionId, sequence: 0 } }));
    fireEvent.click(screen.getByRole("button", { name: "终止任务" }));
    await act(async () => fixture.emit({ event: "cancelled", data: { executionId: fixture.executionId, sequence: 1, outcome: "none", durationMs: 50, droppedOutputBytes: 0 } }));
    expect(screen.getByText("任务已取消")).toBeDefined();

    await act(async () => fixture.resolveCancel());
    expect(screen.queryByText("正在终止进程树")).toBeNull();
    expect(screen.getByText("任务已取消")).toBeDefined();
  });

  it("下一次运行不受旧取消错误污染", async function isolateLateCancelError() {
    const fixture = createCommandGatewayFixture({
      deferCancel: true,
      cancelFailure: { code: "CANCEL_FAILED", message: "private" },
    });
    renderConnectedWorkspace(fixture);

    await startConfirmedExecution();
    await waitFor(() => expect(screen.getByText(fixture.executionId)).toBeDefined());
    await act(async () => fixture.emit({ event: "started", data: { executionId: fixture.executionId, sequence: 0 } }));
    fireEvent.click(screen.getByRole("button", { name: "终止任务" }));
    await act(async () => fixture.emit({ event: "cancelled", data: { executionId: fixture.executionId, sequence: 1, outcome: "none", durationMs: 50, droppedOutputBytes: 0 } }));
    await clickAvailablePreview("重新生成 Preview");
    fireEvent.click(
      await screen.findByRole("button", { name: "执行当前命令" }),
    );
    await waitFor(() => expect(fixture.eventHandlers).toHaveLength(2));
    await act(async () => fixture.resolveCancel());

    expect(screen.queryByRole("alert")).toBeNull();
    expect(screen.queryByText("任务已取消")).toBeNull();
    expect(screen.getAllByText("正在建立执行").length).toBeGreaterThan(0);
    expect(screen.getByRole("button", { name: "终止任务" })).toBeDefined();
  });

  it("表驱动收敛全部 Run 公开拒绝与未知值并闭合迟到 Channel", async function normalizeRunRejections() {
    /** Run 当前公开的 11 个拒绝码及其固定前端说明。 */
    const publishedRunErrors = [
      ["COMMAND_BLOCK_NOT_FOUND", "未找到指定的 Command Block"],
      ["REVISION_CONFLICT", "Command Block 已更新，请重新载入"],
      ["VALIDATION_FAILED", "请求参数未通过校验"],
      ["INVALID_TEMPLATE", "Command Block 模板无效"],
      ["UNSUPPORTED_RUNNER", "当前 Runner 尚不支持"],
      ["RUNNER_UNAVAILABLE", "系统 Runner 不可用"],
      ["INTERNAL_CONTRACT", "Command Block 内部契约无效"],
      ["STALE_PREVIEW", "Preview 已失效，请重新生成"],
      ["ARTIFACT_PREPARATION_FAILED", "无法准备 Execution 临时脚本"],
      ["PROCESS_START_FAILED", "无法启动 Execution 进程"],
      ["EXECUTION_START_FAILED", "无法建立 Execution 后台任务"],
    ] as const;
    /** 未发布拒绝必须统一收敛到本地 IPC 失败。 */
    const cases: ReadonlyArray<{
      /** Gateway 实际拒绝值。 */
      readonly rejection: unknown;
      /** UI 允许显示的固定错误码。 */
      readonly expectedCode: string;
      /** UI 允许显示的固定说明。 */
      readonly expectedMessage: string;
      /** 是否必须重新读取 Summary 与 Details 身份。 */
      readonly reloadIdentity: boolean;
    }> = [
      ...publishedRunErrors.map(([code, expectedMessage]) => ({
        rejection: {
          code,
          message: String.raw`C:\private\run-secret`,
        },
        expectedCode: code,
        expectedMessage,
        reloadIdentity:
          code === "COMMAND_BLOCK_NOT_FOUND" || code === "REVISION_CONFLICT",
      })),
      {
        rejection: { code: "UNKNOWN_RUN", message: "private unknown" },
        expectedCode: "IPC_FAILED",
        expectedMessage: "CmdBox 无法完成桌面宿主调用",
        reloadIdentity: false,
      },
    ];

    for (const testCase of cases) {
      cleanup();
      const fixture = createCommandGatewayFixture({
        deferRun: true,
        runFailure: testCase.rejection,
      });
      renderConnectedWorkspace(fixture);
      await startConfirmedExecution();
      await act(async () => {
        fixture.emit({
          event: "started",
          data: { executionId: fixture.executionId, sequence: 0 },
        });
        fixture.emit({
          event: "output",
          data: {
            executionId: fixture.executionId,
            sequence: 1,
            fragments: [
              {
                fragmentSequence: 1,
                stream: "stdout",
                text: `拒绝前缓存-${testCase.expectedCode}`,
              },
            ],
            droppedBytesBefore: 0,
          },
        });
        fixture.resolveRun();
      });
      await waitFor(() =>
        expect(screen.getByRole("alert")).toHaveProperty(
          "textContent",
          `${testCase.expectedCode}${testCase.expectedMessage}`,
        ),
      );
      await act(async () => {
        fixture.emit({
          event: "output",
          data: {
            executionId: fixture.executionId,
            sequence: 2,
            fragments: [
              {
                fragmentSequence: 2,
                stream: "stderr",
                text: `拒绝后迟到-${testCase.expectedCode}`,
              },
            ],
            droppedBytesBefore: 0,
          },
        });
        fixture.emit({
          event: "finished",
          data: {
            executionId: fixture.executionId,
            sequence: 3,
            exitCode: 0,
            outcome: "success",
            durationMs: 3,
            droppedOutputBytes: 0,
          },
        });
      });

      expect(screen.queryByText(/拒绝前缓存-/)).toBeNull();
      expect(screen.queryByText(/拒绝后迟到-/)).toBeNull();
      expect(screen.queryByText("任务自然结束")).toBeNull();
      expect(screen.queryByText("a".repeat(64))).toBeNull();
      expect(screen.queryByRole("button", { name: "执行当前命令" })).toBeNull();
      expect(screen.queryByText(/run-secret|private unknown/)).toBeNull();
      expect(screen.getByText("尚未创建")).toBeDefined();
      expect(
        (screen.getByRole("searchbox", {
          name: "搜索命令块",
        }) as HTMLInputElement).disabled,
      ).toBe(false);
      await waitFor(() =>
        expect(fixture.gateway.listCommandBlocks).toHaveBeenCalledTimes(
          testCase.reloadIdentity ? 2 : 1,
        ),
      );
      await waitFor(() =>
        expect(fixture.gateway.getCommandBlock).toHaveBeenCalledTimes(
          testCase.reloadIdentity ? 2 : 1,
        ),
      );
      const previewButton = await screen.findByRole("button", {
        name: "生成 Preview",
      });
      await waitFor(() =>
        expect(previewButton).toHaveProperty("disabled", false),
      );
    }
  }, 15_000);

  it("身份拒绝 reload 无参数 Definition 后仍要求用户手动重新 Preview", async function suppressReloadAutoPreview() {
    /** 当前测试唯一的无参数 Command Block 摘要。 */
    const summary: CommandBlockSummary = {
      ...commandSummaries[0],
      id: "builtin.parameterless-reload",
      name: "无参数身份恢复",
    };
    /** 与摘要身份完全一致的无参数详情。 */
    const details: CommandBlockDetails = { ...summary, parameters: [] };
    const fixture = createCommandGatewayFixture({
      summaries: [summary],
      details: new Map([[summary.id, details]]),
      runFailure: { code: "REVISION_CONFLICT", message: "private" },
    });
    renderConnectedWorkspace(fixture);
    const firstRunButton = await screen.findByRole("button", {
      name: "执行当前命令",
    });
    expect(fixture.gateway.previewCommandBlock).toHaveBeenCalledOnce();
    fireEvent.click(firstRunButton);

    await waitFor(() =>
      expect(fixture.gateway.getCommandBlock).toHaveBeenCalledTimes(2),
    );
    expect(fixture.gateway.previewCommandBlock).toHaveBeenCalledOnce();
    const manualPreviewButton = await screen.findByRole("button", {
      name: "生成 Preview",
    });
    await waitFor(() =>
      expect(manualPreviewButton).toHaveProperty("disabled", false),
    );
    fireEvent.click(manualPreviewButton);
    await waitFor(() =>
      expect(fixture.gateway.previewCommandBlock).toHaveBeenCalledTimes(2),
    );
    expect(
      await screen.findByRole("button", { name: "执行当前命令" }),
    ).toBeDefined();
  });

  it("身份恢复 List 迟到时不覆盖用户期间手动选择的新命令", async function preserveManualSelectionDuringIdentityReload() {
    /** 让 Run 身份拒绝触发的第二次 List 保持挂起，精确复现恢复与手动选择竞态。 */
    const recoveryList = createDeferred<CommandBlockSummary[]>();
    let listRequestIndex = 0;
    const fixture = createCommandGatewayFixture({
      runFailure: { code: "REVISION_CONFLICT", message: "private" },
    });
    fixture.gateway.listCommandBlocks = vi.fn(() => {
      listRequestIndex += 1;
      return listRequestIndex === 1
        ? Promise.resolve([...commandSummaries])
        : recoveryList.promise;
    });
    renderConnectedWorkspace(fixture);
    await startConfirmedExecution();
    await waitFor(() =>
      expect(fixture.gateway.listCommandBlocks).toHaveBeenCalledTimes(2),
    );

    fireEvent.click(screen.getByRole("button", { name: /CMD 参数回显/ }));
    await waitFor(() =>
      expect(
        screen.getByRole("heading", { name: "CMD 参数回显", level: 1 }),
      ).toBeDefined(),
    );
    expect(screen.getByRole("textbox", { name: /文本/ })).toHaveProperty(
      "value",
      "CMD 默认",
    );

    await act(async () => recoveryList.resolve([...commandSummaries]));

    expect(
      screen.getByRole("heading", { name: "CMD 参数回显", level: 1 }),
    ).toBeDefined();
    expect(screen.getByRole("textbox", { name: /文本/ })).toHaveProperty(
      "value",
      "CMD 默认",
    );
    expect(fixture.gateway.getCommandBlock).toHaveBeenCalledTimes(2);
  });

  it("身份恢复 List 迟到拒绝时不污染用户期间加载的新命令", async function discardIdentityReloadErrorAfterManualSelection() {
    /** 第二次 List 由测试延迟拒绝，用于验证旧恢复 catch 同样受身份所有权门禁。 */
    const recoveryList = createDeferred<CommandBlockSummary[]>();
    let listRequestIndex = 0;
    const fixture = createCommandGatewayFixture({
      runFailure: { code: "REVISION_CONFLICT", message: "private" },
    });
    fixture.gateway.listCommandBlocks = vi.fn(() => {
      listRequestIndex += 1;
      return listRequestIndex === 1
        ? Promise.resolve([...commandSummaries])
        : recoveryList.promise;
    });
    renderConnectedWorkspace(fixture);
    await startConfirmedExecution();
    await waitFor(() =>
      expect(fixture.gateway.listCommandBlocks).toHaveBeenCalledTimes(2),
    );

    fireEvent.click(screen.getByRole("button", { name: /CMD 参数回显/ }));
    await waitFor(() =>
      expect(
        screen.getByRole("heading", { name: "CMD 参数回显", level: 1 }),
      ).toBeDefined(),
    );
    expect(screen.getByRole("textbox", { name: /文本/ })).toHaveProperty(
      "value",
      "CMD 默认",
    );

    await act(async () =>
      recoveryList.reject({
        code: "RUNNER_UNAVAILABLE",
        message: "private old recovery",
      }),
    );

    expect(
      screen.getByRole("heading", { name: "CMD 参数回显", level: 1 }),
    ).toBeDefined();
    expect(screen.getByRole("textbox", { name: /文本/ })).toHaveProperty(
      "value",
      "CMD 默认",
    );
    expect(screen.getAllByRole("alert")).toHaveLength(1);
    expect(screen.getByRole("alert")).toHaveProperty(
      "textContent",
      "REVISION_CONFLICTCommand Block 已更新，请重新载入",
    );
    expect(screen.queryByText("RUNNER_UNAVAILABLE")).toBeNull();
    expect(fixture.gateway.getCommandBlock).toHaveBeenCalledTimes(2);
  });

  it("组件卸载后失效 Run 响应与专属 Channel 的全部迟到事实", async function discardRunAfterUnmount() {
    const fixture = createCommandGatewayFixture({ deferRun: true });
    const rendered = renderConnectedWorkspace(fixture);
    await startConfirmedExecution();
    expect(fixture.eventHandlers).toHaveLength(1);
    rendered.unmount();

    await act(async () => {
      fixture.emit({
        event: "started",
        data: { executionId: fixture.executionId, sequence: 0 },
      });
      fixture.emit({
        event: "output",
        data: {
          executionId: fixture.executionId,
          sequence: 1,
          fragments: [
            {
              fragmentSequence: 1,
              stream: "stdout",
              text: "卸载后迟到输出",
            },
          ],
          droppedBytesBefore: 0,
        },
      });
      fixture.resolveRun();
    });

    expect(rendered.container).toHaveProperty("textContent", "");
    expect(fixture.gateway.cancelExecution).not.toHaveBeenCalled();
  });

  it("启动失败显示公开错误并恢复可运行状态", async function showStartFailure() {
    const fixture = createCommandGatewayFixture({
      runFailure: { code: "PROCESS_START_FAILED", message: "private" },
    });
    renderConnectedWorkspace(fixture);

    await startConfirmedExecution();
    await waitFor(() => {
      expect(screen.getByRole("alert")).toHaveProperty("textContent", expect.stringContaining("PROCESS_START_FAILED"));
    });
    expect(screen.getByRole("button", { name: "生成 Preview" })).toBeDefined();
    expect(screen.queryByRole("button", { name: "执行当前命令" })).toBeNull();
  });
});
