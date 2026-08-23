/**
 * 真实 Command Block Definition 驱动的统一 Command Workspace。
 *
 * 组件继承用户确认的 editorial-field-notes 视觉语法，读取真实 Summary/Details 并渲染
 * 统一 Parameter Form、Rust Preview 与通用 Execution 生命周期。Preview 和 Run 都只消费
 * Rust 事实，本模块不渲染 Shell、不计算 Hash，也不从 Exit Code 推导业务 Outcome。
 */
import { ArrowBendRightDownIcon } from "@phosphor-icons/react/dist/csr/ArrowBendRightDown";
import { CalendarBlankIcon } from "@phosphor-icons/react/dist/csr/CalendarBlank";
import { ClockCounterClockwiseIcon } from "@phosphor-icons/react/dist/csr/ClockCounterClockwise";
import { FileTextIcon } from "@phosphor-icons/react/dist/csr/FileText";
import { GearIcon } from "@phosphor-icons/react/dist/csr/Gear";
import { GitBranchIcon } from "@phosphor-icons/react/dist/csr/GitBranch";
import { MagnifyingGlassIcon } from "@phosphor-icons/react/dist/csr/MagnifyingGlass";
import { MinusIcon } from "@phosphor-icons/react/dist/csr/Minus";
import { PencilSimpleLineIcon } from "@phosphor-icons/react/dist/csr/PencilSimpleLine";
import { PlusIcon } from "@phosphor-icons/react/dist/csr/Plus";
import { ShieldCheckIcon } from "@phosphor-icons/react/dist/csr/ShieldCheck";
import { SquareIcon } from "@phosphor-icons/react/dist/csr/Square";
import { TerminalWindowIcon } from "@phosphor-icons/react/dist/csr/TerminalWindow";
import { TrashIcon } from "@phosphor-icons/react/dist/csr/Trash";
import { WarningIcon } from "@phosphor-icons/react/dist/csr/Warning";
import { XIcon } from "@phosphor-icons/react/dist/csr/X";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import cmdboxIcon from "../../../src-tauri/icons/icon.png";
import {
  createCommandExecutionGateway,
  normalizeApiError,
  type ApiError,
  type CommandBlockDetails,
  type CommandBlockSummary,
  type CommandExecutionGateway,
  type ExecutionStreamEvent,
  type PreviewCommandResponse,
} from "./execution-gateway";
import {
  appendExecutionOutput,
  createPendingExecutionEventBuffer,
  createExecutionOutputBuffer,
  drainPendingExecutionEventBuffer,
  queuePendingExecutionEvent,
  resetPendingExecutionEventBuffer,
} from "./execution-output-buffer";
import {
  createDesktopWindowControls,
  type DesktopWindowControls,
} from "./desktop-window-controls";
import { createFolderPicker, type FolderPicker } from "./folder-picker";
import {
  ParameterForm,
  type ExternalFieldError,
  type ParameterFormSnapshot,
} from "./ParameterForm";
import {
  acceptPreviewResponse,
  createPreviewAttempt,
  createVerifyRunRequest,
  type ConfirmedPreview,
  type DeepReadonly,
  type PreviewAttempt,
} from "./preview-state";

/** 浏览器加载时解析一次通用 Command Block Gateway；非 Tauri 环境保持 `null`。 */
const defaultCommandGateway = createCommandExecutionGateway();

/** 浏览器加载时解析一次生产目录选择接缝；非 Tauri 环境保持 `null`。 */
const defaultFolderPicker = createFolderPicker();

/** 浏览器加载时解析一次当前窗口控制；非 Tauri 环境保持 `null`。 */
const defaultWindowControls = createDesktopWindowControls();

/** Workspace 可由测试注入无副作用 Gateway。 */
export interface CommandWorkspaceProps {
  /** Summary、Details、Preview、Run 与 Cancel 的唯一窄业务 Gateway。 */
  commandGateway?: CommandExecutionGateway | null;
  /** Parameter Form 使用的原生目录 Picker；浏览器环境为 `null`。 */
  folderPicker?: FolderPicker | null;
  /** 当前桌面主窗口控制；`null` 表示普通浏览器预览。 */
  windowControls?: DesktopWindowControls | null;
}

/** 前端根据后端事实派生的当前界面阶段。 */
type WorkspacePhase =
  | "ready"
  | "starting"
  | "running"
  | "cancelling"
  | "finished";

/** 唯一终态的用户可见摘要。 */
interface ExecutionResult {
  /** Rust Core 发布的终态类型。 */
  kind: "finished" | "cancelled" | "failed";
  /** 自然结束时存在的原始 Exit Code。 */
  exitCode?: number;
  /** Rust Core 记录的执行耗时。 */
  durationMs: number;
  /** 后端内部失败时的稳定公开说明。 */
  message?: string;
  /** Session 终态仍未随 Output 报告的丢弃字节数。 */
  droppedOutputBytes: number;
}

/** 已接受并带独立 generation 的当前 Definition。 */
interface LoadedDefinition {
  /** 后端返回的真实公开详情。 */
  details: CommandBlockDetails;
  /** 当前选择请求的独立 generation。 */
  generation: number;
}

/** 选择 Definition 时用于恢复身份而不越过用户 Preview 动作的选项。 */
interface SelectCommandOptions {
  /** 禁止无参数 Definition 在本 generation 自动 Preview。 */
  readonly suppressParameterlessAutoPreview?: boolean;
}

/** Preview 与 Execution 正交的前端界面阶段。 */
type PreviewPhase = "configuring" | "previewing" | "ready";

/** Workspace 只允许消费当前 generation 的 ready 快照。 */
type ParameterSnapshotState =
  | {
      /** Parameter Form 尚未交付当前 generation 的完整 render 快照。 */
      readonly status: "pending";
      /** 当前等待的配置 generation。 */
      readonly configurationGeneration: number;
    }
  | {
      /** 当前 generation 已具有可校验的完整 render 快照。 */
      readonly status: "ready";
      /** Parameter Form 交付的防御性 wire 快照。 */
      readonly snapshot: ParameterFormSnapshot;
    };

/** 当前仍有资格落地的 Preview 请求身份。 */
interface ActivePreviewRequest {
  /** Workspace 内单调请求 token。 */
  readonly token: number;
  /** 请求开始时冻结的 Definition、配置和参数。 */
  readonly attempt: PreviewAttempt;
}

/** 当前仍有资格落地的 Cancel 请求身份。 */
interface ActiveCancelRequest {
  /** Workspace 内单调 Cancel token。 */
  readonly token: number;
  /** Cancel 所属的 Run generation。 */
  readonly runGeneration: number;
  /** Cancel 精确绑定的 Execution UUID。 */
  readonly executionId: string;
}

/** 当前响应身份异常时使用的固定安全前端错误。 */
const PREVIEW_IDENTITY_ERROR: ApiError = {
  code: "IPC_FAILED",
  message: "CmdBox 无法完成桌面宿主调用",
};

/** 为真实命令摘要选择稳定图标，不从后端接受任意组件名。 */
function CommandSummaryIcon({ summary }: { summary: CommandBlockSummary }) {
  const props = { "aria-hidden": true, size: 19, weight: "light" } as const;
  return summary.riskLevel === "destructive" ? (
    <TrashIcon {...props} />
  ) : (
    <TerminalWindowIcon {...props} />
  );
}

/** 把 Runner contract 值转换为用户可读但不添加版本猜测的名称。 */
function runnerLabel(runner: CommandBlockSummary["runner"]): string {
  return runner === "windowsPowerShell" ? "Windows PowerShell" : "CMD";
}

/** 判断当前 Execution phase 是否必须锁定全部命令配置。 */
function isExecutionActive(phase: WorkspacePhase): boolean {
  return phase === "starting" || phase === "running" || phase === "cancelling";
}

/** 渲染并控制真实 Command Block 的 Preview 与 Execution 工作区。 */
export function CommandWorkspace({
  commandGateway = defaultCommandGateway,
  folderPicker = defaultFolderPicker,
  windowControls = defaultWindowControls,
}: CommandWorkspaceProps) {
  /** 当前命令索引搜索词。 */
  const [query, setQuery] = useState("");
  /** 后端按固定顺序返回的真实 Command Block 摘要。 */
  const [commandSummaries, setCommandSummaries] = useState<CommandBlockSummary[]>([]);
  /** 当前用户选中的真实 Command Block ID。 */
  const [selectedCommandId, setSelectedCommandId] = useState<string | null>(null);
  /** 当前经 identity/generation 检查接受的 Definition。 */
  const [loadedDefinition, setLoadedDefinition] = useState<LoadedDefinition | null>(null);
  /** 当前 Summary/Details 加载状态。 */
  const [commandLoading, setCommandLoading] = useState(commandGateway !== null);
  /** 当前列表或详情加载的安全公开错误。 */
  const [commandError, setCommandError] = useState<ApiError | null>(null);
  /** 当前 Parameter Form 是否已交付与配置 generation 一致的完整 render 快照。 */
  const [parameterState, setParameterState] = useState<ParameterSnapshotState>({
    status: "pending",
    configurationGeneration: 0,
  });
  /** 当前 Preview 的正交界面阶段。 */
  const [previewPhase, setPreviewPhase] = useState<PreviewPhase>("configuring");
  /** 当前可展示的 Rust Preview；blocked 响应也保留为只读证据。 */
  const [previewResponse, setPreviewResponse] = useState<
    DeepReadonly<PreviewCommandResponse> | null
  >(null);
  /** 后续 UI-RUN 原子唯一允许消费的不可变 Preview 授权。 */
  const [confirmedPreview, setConfirmedPreview] = useState<
    ConfirmedPreview | null
  >(null);
  /** 当前 Preview 请求的工作区级安全错误。 */
  const [previewError, setPreviewError] = useState<ApiError | null>(null);
  /** 当前且仅当前双 generation 可传给 Parameter Form 的 Rust 字段错误。 */
  const [externalFieldError, setExternalFieldError] =
    useState<ExternalFieldError | null>(null);
  /** 当前由后端事实驱动的界面阶段。 */
  const [phase, setPhase] = useState<WorkspacePhase>("ready");
  /** 当前由启动响应确认的 Execution UUID。 */
  const [executionId, setExecutionId] = useState<string | null>(null);
  /** 当前任务的独立有界 Output Buffer。 */
  const [output, setOutput] = useState(createExecutionOutputBuffer);
  /** 当前唯一终态摘要。 */
  const [result, setResult] = useState<ExecutionResult | null>(null);
  /** 当前公开 IPC 错误。 */
  const [error, setError] = useState<ApiError | null>(null);
  /** 防止重复提交取消请求的局部请求状态。 */
  const [cancelRequestPending, setCancelRequestPending] = useState(false);
  /** Run 调用入口的同步互斥门禁，阻止同一 commit 内重复点击。 */
  const runRequestPending = useRef(false);
  /** 每次 Run 递增，用于隔离旧 Channel 回调。 */
  const runGeneration = useRef(0);
  /** 当前 Channel 已绑定的 Execution UUID。 */
  const expectedExecutionId = useRef<string | null>(null);
  /** 当前 Execution 已接受的最大事件级 sequence。 */
  const lastSequence = useRef(-1);
  /** Run 响应返回前的事件、Fragment、字节与按 ID 淘汰账本。 */
  const pendingExecutionEvents = useRef(createPendingExecutionEventBuffer());
  /** 当前 Execution 是否已经接受唯一终态。 */
  const terminalAccepted = useRef(false);
  /** 与 React 状态同步的阶段快照，供异步响应防止状态倒退。 */
  const phaseSnapshot = useRef<WorkspacePhase>("ready");
  /** 为 Cancel 请求分配组件内唯一的单调 token。 */
  const cancelRequestSequence = useRef(0);
  /** 当前唯一仍有资格落地的 Cancel 请求。 */
  const activeCancelRequest = useRef<ActiveCancelRequest | null>(null);
  /** 组件卸载后拒绝 List/Get 迟到结果。 */
  const mounted = useRef(false);
  /** 独立于 Execution 的 List 请求 generation。 */
  const listGeneration = useRef(0);
  /** 每次命令选择递增，隔离 Get Details 迟到响应。 */
  const definitionGeneration = useRef(0);
  /** 每次真实参数写入或命令切换递增，立即撤销旧 Preview 授权。 */
  const configurationGeneration = useRef(0);
  /** 与 React 状态同步的当前 Parameter 快照门禁。 */
  const parameterStateSnapshot = useRef<ParameterSnapshotState>({
    status: "pending",
    configurationGeneration: 0,
  });
  /** 为 Preview 请求分配当前组件内唯一的单调 token。 */
  const previewRequestSequence = useRef(0);
  /** 当前唯一仍有资格落地的 Preview 请求。 */
  const activePreviewRequest = useRef<ActivePreviewRequest | null>(null);
  /** Strict Mode 下记录已经自动 Preview 的 Parameterless Definition generation。 */
  const parameterlessAutoPreviewGeneration = useRef<number | null>(null);
  /** 只允许一次同步消费的当前 ConfirmedPreview 执行授权。 */
  const executablePreview = useRef<ConfirmedPreview | null>(null);
  /** 当前选择的 id/revision/generation 身份快照。 */
  const selectedDefinitionIdentity = useRef<{
    id: string;
    revision: number;
    generation: number;
  } | null>(null);

  /** 清除当前 Preview 展示、授权和错误，不改变 Definition 或 Execution。 */
  const clearPreviewState = useCallback(() => {
    activePreviewRequest.current = null;
    executablePreview.current = null;
    setConfirmedPreview(null);
    setPreviewResponse(null);
    setPreviewError(null);
    setExternalFieldError(null);
    setPreviewPhase("configuring");
  }, []);

  /**
   * 在 Parameter Form 写入任一真实值前同步撤销旧授权，并返回新的配置 generation。
   */
  const beginConfigurationChange = useCallback((): number => {
    /** 本次真实用户写入所属的新配置 generation。 */
    const nextGeneration = configurationGeneration.current + 1;
    configurationGeneration.current = nextGeneration;
    /** 在 RHF 提交新 render 快照前保持显式 pending，禁止组合旧值发起 Preview。 */
    const pendingState: ParameterSnapshotState = {
      status: "pending",
      configurationGeneration: nextGeneration,
    };
    parameterStateSnapshot.current = pendingState;
    setParameterState(pendingState);
    clearPreviewState();
    return nextGeneration;
  }, [clearPreviewState]);

  /** 只接收与当前 configuration generation 完全一致的 Parameter Form 快照。 */
  const acceptParameterState = useCallback((state: ParameterFormSnapshot) => {
    if (state.configurationGeneration !== configurationGeneration.current) {
      return;
    }
    /** 已通过 generation 门禁的当前完整 render 快照。 */
    const readyState: ParameterSnapshotState = {
      status: "ready",
      snapshot: {
        values: Object.fromEntries(
          Object.entries(state.values).map(([key, value]) => [
            key,
            Array.isArray(value) ? [...value] : value,
          ]),
        ),
        isValid: state.isValid,
        configurationGeneration: state.configurationGeneration,
      },
    };
    parameterStateSnapshot.current = readyState;
    setParameterState(readyState);
  }, []);

  /** 选择 Summary、建立独立 generation，并只接受完全匹配的 Details。 */
  async function selectCommand(
    summary: CommandBlockSummary,
    options: SelectCommandOptions = {},
  ) {
    if (!commandGateway || isExecutionActive(phaseSnapshot.current)) {
      return;
    }
    const generation = definitionGeneration.current + 1;
    definitionGeneration.current = generation;
    selectedDefinitionIdentity.current = {
      id: summary.id,
      revision: summary.revision,
      generation,
    };
    /** 命令切换独立推进配置 generation，确保 A→B→A 不复活第一次 A。 */
    const nextConfigurationGeneration = configurationGeneration.current + 1;
    configurationGeneration.current = nextConfigurationGeneration;
    /** 新 Definition 返回前不存在可用于 Preview 的参数快照。 */
    const pendingParameterState: ParameterSnapshotState = {
      status: "pending",
      configurationGeneration: nextConfigurationGeneration,
    };
    parameterStateSnapshot.current = pendingParameterState;
    setParameterState(pendingParameterState);
    clearPreviewState();
    setSelectedCommandId(summary.id);
    setLoadedDefinition(null);
    setCommandError(null);
    setCommandLoading(true);
    try {
      const details = await commandGateway.getCommandBlock(summary.id);
      const identity = selectedDefinitionIdentity.current;
      if (
        !mounted.current ||
        !identity ||
        identity.id !== summary.id ||
        identity.revision !== summary.revision ||
        identity.generation !== generation
      ) {
        return;
      }
      if (details.id !== summary.id) {
        setCommandError({
          code: "IPC_FAILED",
          message: "CmdBox 无法完成桌面宿主调用",
        });
        setCommandLoading(false);
        return;
      }
      if (details.revision !== summary.revision) {
        setCommandError({
          code: "REVISION_CONFLICT",
          message: "Command Block 已更新，请重新载入",
        });
        setCommandLoading(false);
        return;
      }
      /** 当前身份已完整校验的 Definition。 */
      const acceptedDefinition = { details, generation };
      if (options.suppressParameterlessAutoPreview) {
        parameterlessAutoPreviewGeneration.current = generation;
      }
      setLoadedDefinition(acceptedDefinition);
      if (details.parameters.length === 0) {
        /** Parameterless Definition 由 Workspace 建立当前双 generation 的空快照。 */
        const parameterlessSnapshot: ParameterFormSnapshot = {
          values: {},
          isValid: true,
          configurationGeneration: configurationGeneration.current,
        };
        /** Parameterless 快照无需等待 Parameter Form effect。 */
        const readyParameterState: ParameterSnapshotState = {
          status: "ready",
          snapshot: parameterlessSnapshot,
        };
        parameterStateSnapshot.current = readyParameterState;
        setParameterState(readyParameterState);
      }
      setCommandLoading(false);
    } catch (loadError: unknown) {
      const identity = selectedDefinitionIdentity.current;
      if (
        mounted.current &&
        identity?.id === summary.id &&
        identity.revision === summary.revision &&
        identity.generation === generation
      ) {
        setCommandError(loadError as ApiError);
        setCommandLoading(false);
      }
    }
  }

  /** 重新读取 Summary，并按首选 ID 读取最新 Details 与 revision。 */
  async function reloadCommandDefinitions(
    preferredCommandId?: string,
    suppressParameterlessAutoPreview = false,
  ): Promise<void> {
    if (!commandGateway) {
      setCommandLoading(false);
      return;
    }
    /** 本次 List 调用的独立 generation。 */
    const generation = listGeneration.current + 1;
    listGeneration.current = generation;
    /** 身份恢复只在发起时的选择仍归它所有时才可重新选择，用户后续选择始终优先。 */
    const recoveryOwnerIdentity =
      preferredCommandId === undefined
        ? null
        : selectedDefinitionIdentity.current;
    /** 统一判断当前 List 的成功或失败是否仍有资格修改工作区。 */
    const canApplyReloadResult = (): boolean => {
      /** 判断时重新读取当前身份，避免闭包保留发起恢复时的旧选择。 */
      const currentIdentity = selectedDefinitionIdentity.current;
      return (
        mounted.current &&
        listGeneration.current === generation &&
        (recoveryOwnerIdentity === null ||
          (currentIdentity?.id === recoveryOwnerIdentity.id &&
            currentIdentity.revision === recoveryOwnerIdentity.revision &&
            currentIdentity.generation === recoveryOwnerIdentity.generation))
      );
    };
    setCommandLoading(true);
    try {
      /** Rust 固定顺序返回的最新公开摘要。 */
      const summaries = await commandGateway.listCommandBlocks();
      /** List generation 隔离新 List；身份快照隔离期间发生的手动命令切换。 */
      if (!canApplyReloadResult()) {
        return;
      }
      setCommandSummaries([...summaries]);
      setCommandLoading(false);
      /** 身份错误优先恢复原 ID，否则选择 Rust 当前第一项。 */
      const nextSummary =
        summaries.find((summary) => summary.id === preferredCommandId) ??
        summaries[0];
      if (nextSummary) {
        void selectCommand(nextSummary, {
          suppressParameterlessAutoPreview,
        });
      } else {
        selectedDefinitionIdentity.current = null;
        setSelectedCommandId(null);
        setLoadedDefinition(null);
        clearPreviewState();
      }
    } catch (loadError: unknown) {
      if (canApplyReloadResult()) {
        setCommandError(normalizeApiError(loadError));
        setCommandLoading(false);
      }
    }
  }

  /** 初次挂载只加载一次真实列表并选择第一项；卸载后 List/Get 均失效。 */
  useEffect(() => {
    mounted.current = true;
    if (!commandGateway) {
      setCommandLoading(false);
      return () => {
        mounted.current = false;
        listGeneration.current += 1;
        definitionGeneration.current += 1;
        configurationGeneration.current += 1;
        selectedDefinitionIdentity.current = null;
        activePreviewRequest.current = null;
        executablePreview.current = null;
        runRequestPending.current = false;
        runGeneration.current += 1;
        expectedExecutionId.current = null;
        resetPendingExecutionEventBuffer(pendingExecutionEvents.current);
        terminalAccepted.current = true;
        activeCancelRequest.current = null;
        cancelRequestSequence.current += 1;
      };
    }
    void reloadCommandDefinitions();
    return () => {
      mounted.current = false;
      listGeneration.current += 1;
      definitionGeneration.current += 1;
      configurationGeneration.current += 1;
      selectedDefinitionIdentity.current = null;
      activePreviewRequest.current = null;
      executablePreview.current = null;
      runRequestPending.current = false;
      runGeneration.current += 1;
      expectedExecutionId.current = null;
      resetPendingExecutionEventBuffer(pendingExecutionEvents.current);
      terminalAccepted.current = true;
      activeCancelRequest.current = null;
      cancelRequestSequence.current += 1;
    };
  }, [commandGateway]);

  /** 同步更新界面阶段和异步阶段快照。 */
  function transitionPhase(nextPhase: WorkspacePhase) {
    phaseSnapshot.current = nextPhase;
    setPhase(nextPhase);
  }

  /** 按名称和说明过滤当前命令摘要。 */
  const filteredCommands = useMemo(() => {
    const normalizedQuery = query.trim().toLocaleLowerCase("zh-CN");
    if (!normalizedQuery) {
      return commandSummaries;
    }
    return commandSummaries.filter((command) =>
      `${command.name} ${command.description}`
        .toLocaleLowerCase("zh-CN")
        .includes(normalizedQuery),
    );
  }, [commandSummaries, query]);

  /** 判断一个异步 Preview 结果是否仍属于当前组件、请求和双 generation。 */
  function isPreviewRequestCurrent(request: ActivePreviewRequest): boolean {
    /** 当前用户选择的 Definition 身份。 */
    const identity = selectedDefinitionIdentity.current;
    return (
      mounted.current &&
      activePreviewRequest.current?.token === request.token &&
      activePreviewRequest.current.attempt === request.attempt &&
      definitionGeneration.current === request.attempt.definitionGeneration &&
      configurationGeneration.current ===
        request.attempt.configurationGeneration &&
      identity?.id === request.attempt.commandBlockId &&
      identity.revision === request.attempt.revision &&
      identity.generation === request.attempt.definitionGeneration
    );
  }

  /**
   * 用一个与当前双 generation 完全匹配的完整 Parameter 快照请求 Rust Preview。
   */
  async function requestPreviewFor(
    definition: LoadedDefinition,
    parameters: ParameterSnapshotState,
  ): Promise<void> {
    if (
      !commandGateway ||
      activePreviewRequest.current !== null ||
      isExecutionActive(phaseSnapshot.current) ||
      parameters.status !== "ready" ||
      !parameters.snapshot.isValid ||
      parameters.snapshot.configurationGeneration !==
        configurationGeneration.current ||
      definition.generation !== definitionGeneration.current
    ) {
      return;
    }
    /** 当前用户选择必须仍与待请求 Definition 完全一致。 */
    const identity = selectedDefinitionIdentity.current;
    if (
      identity?.id !== definition.details.id ||
      identity.revision !== definition.details.revision ||
      identity.generation !== definition.generation
    ) {
      return;
    }
    /** 请求值与后续确认值由模块分别深复制并冻结。 */
    const attempt = createPreviewAttempt(
      definition.details,
      definition.generation,
      parameters.snapshot.configurationGeneration,
      parameters.snapshot.values,
    );
    /** 当前组件内单调增长的 Preview token。 */
    const token = previewRequestSequence.current + 1;
    previewRequestSequence.current = token;
    /** 当前请求的全部不可变身份。 */
    const request: ActivePreviewRequest = { token, attempt };
    activePreviewRequest.current = request;
    executablePreview.current = null;
    setConfirmedPreview(null);
    setPreviewResponse(null);
    setPreviewError(null);
    setExternalFieldError(null);
    setPreviewPhase("previewing");
    try {
      /** Rust Core 返回的 Preview 仍需经过当前 token 与响应身份检查。 */
      const response = await commandGateway.previewCommandBlock(attempt.request);
      if (!isPreviewRequestCurrent(request)) {
        return;
      }
      activePreviewRequest.current = null;
      /** 响应接纳模块负责身份校验、深复制和只读冻结。 */
      const acceptance = acceptPreviewResponse(attempt, response);
      if (acceptance.kind === "identityMismatch") {
        setPreviewError(PREVIEW_IDENTITY_ERROR);
        setPreviewPhase("configuring");
        return;
      }
      if (acceptance.kind === "blocked") {
        setPreviewResponse(acceptance.response);
        setPreviewPhase("configuring");
        return;
      }
      executablePreview.current = acceptance.confirmedPreview;
      setConfirmedPreview(acceptance.confirmedPreview);
      setPreviewResponse(acceptance.confirmedPreview.response);
      setPreviewPhase("ready");
      if (phaseSnapshot.current === "finished") {
        transitionPhase("ready");
      }
    } catch (requestError: unknown) {
      if (!isPreviewRequestCurrent(request)) {
        return;
      }
      activePreviewRequest.current = null;
      /** 任意 Gateway 拒绝先经过既有白名单收敛，避免显示原始拒绝对象。 */
      const normalizedError = normalizeApiError(requestError);
      /** 只有当前 Definition 确实声明的 key 才能进入字段错误接缝。 */
      const currentParameter = normalizedError.parameterKey
        ? definition.details.parameters.find(
            (parameter) => parameter.key === normalizedError.parameterKey,
          )
        : undefined;
      if (
        normalizedError.code === "VALIDATION_FAILED" &&
        currentParameter &&
        normalizedError.parameterKey
      ) {
        setExternalFieldError({
          definitionGeneration: attempt.definitionGeneration,
          configurationGeneration: attempt.configurationGeneration,
          parameterKey: normalizedError.parameterKey,
          message: normalizedError.message,
        });
      } else {
        setPreviewError(normalizedError);
      }
      setPreviewPhase("configuring");
    }
  }

  /** 手动 Preview 只读取当前 render 已交付的快照，不读取 DOM 或可变表单 ref。 */
  function requestCurrentPreview(): void {
    if (!loadedDefinition) {
      return;
    }
    void requestPreviewFor(loadedDefinition, parameterStateSnapshot.current);
  }

  /** Parameterless Command 每个 Definition generation 自动 Preview 恰好一次。 */
  useEffect(() => {
    if (
      !loadedDefinition ||
      loadedDefinition.details.parameters.length !== 0 ||
      parameterState.status !== "ready" ||
      parameterlessAutoPreviewGeneration.current === loadedDefinition.generation
    ) {
      return;
    }
    parameterlessAutoPreviewGeneration.current = loadedDefinition.generation;
    void requestPreviewFor(loadedDefinition, parameterState);
  }, [commandGateway, loadedDefinition, parameterState]);

  /** 立即撤销当前 Cancel 请求资格，供新 Run、终态、拒绝与卸载隔离迟到响应。 */
  function invalidateCancelRequest(): void {
    activeCancelRequest.current = null;
    cancelRequestSequence.current += 1;
    setCancelRequestPending(false);
  }

  /** 接受一个唯一后端终态，并让后续执行重新经过 Preview。 */
  function acceptExecutionTerminal(resultSnapshot: ExecutionResult): void {
    terminalAccepted.current = true;
    runRequestPending.current = false;
    resetPendingExecutionEventBuffer(pendingExecutionEvents.current);
    invalidateCancelRequest();
    clearPreviewState();
    setError(null);
    setResult(resultSnapshot);
    transitionPhase("finished");
  }

  /** 接受当前 Channel 的 generation、ID、sequence 与唯一终态认证事件。 */
  function acceptExecutionEvent(event: ExecutionStreamEvent, generation: number) {
    if (!mounted.current || generation !== runGeneration.current) {
      return;
    }
    if (expectedExecutionId.current === null) {
      queuePendingExecutionEvent(pendingExecutionEvents.current, event);
      return;
    }
    const { executionId: eventExecutionId, sequence } = event.data;
    if (
      eventExecutionId !== expectedExecutionId.current ||
      sequence <= lastSequence.current ||
      terminalAccepted.current
    ) {
      return;
    }
    lastSequence.current = sequence;

    switch (event.event) {
      case "started":
        if (phaseSnapshot.current === "starting") {
          transitionPhase("running");
        }
        return;
      case "output":
        setOutput((current) =>
          appendExecutionOutput(
            current,
            event.data.sequence,
            event.data.fragments,
            event.data.droppedBytesBefore,
          ),
        );
        return;
      case "finished":
        acceptExecutionTerminal({
          kind: "finished",
          exitCode: event.data.exitCode,
          durationMs: event.data.durationMs,
          droppedOutputBytes: event.data.droppedOutputBytes,
        });
        return;
      case "cancelled":
        acceptExecutionTerminal({
          kind: "cancelled",
          durationMs: event.data.durationMs,
          droppedOutputBytes: event.data.droppedOutputBytes,
        });
        return;
      case "failed":
        acceptExecutionTerminal({
          kind: "failed",
          message: event.data.message,
          durationMs: event.data.durationMs,
          droppedOutputBytes: event.data.droppedOutputBytes,
        });
    }
  }

  /** 收敛当前 Run 拒绝，撤销全部执行资格并隔离同 Channel 的迟到事件。 */
  function rejectExecutionRun(
    generation: number,
    rejectedPreview: ConfirmedPreview,
    runError: unknown,
  ): void {
    if (!mounted.current || generation !== runGeneration.current) {
      return;
    }
    /** 所有公开与未知拒绝都先经过固定白名单说明。 */
    const normalizedError = normalizeApiError(runError);
    runGeneration.current = generation + 1;
    runRequestPending.current = false;
    expectedExecutionId.current = null;
    lastSequence.current = -1;
    resetPendingExecutionEventBuffer(pendingExecutionEvents.current);
    terminalAccepted.current = true;
    invalidateCancelRequest();
    clearPreviewState();
    setExecutionId(null);
    setOutput(createExecutionOutputBuffer());
    setResult(null);
    setError(normalizedError);
    transitionPhase("ready");
    if (
      normalizedError.code === "COMMAND_BLOCK_NOT_FOUND" ||
      normalizedError.code === "REVISION_CONFLICT"
    ) {
      void reloadCommandDefinitions(rejectedPreview.commandBlockId, true);
    }
  }

  /** 同步消费当前 ConfirmedPreview，并请求 Rust 复验后创建通用 Execution。 */
  async function startExecution(): Promise<void> {
    /** 当前点击开始时仍未被任何异步边界消费的唯一授权。 */
    const preview = executablePreview.current;
    /** 当前选中 Definition 的完整身份。 */
    const identity = selectedDefinitionIdentity.current;
    if (
      !commandGateway ||
      !preview ||
      runRequestPending.current ||
      phaseSnapshot.current !== "ready" ||
      preview.definitionGeneration !== definitionGeneration.current ||
      preview.configurationGeneration !== configurationGeneration.current ||
      identity?.id !== preview.commandBlockId ||
      identity.revision !== preview.revision ||
      identity.generation !== preview.definitionGeneration
    ) {
      return;
    }
    /** 只从确认快照再次深复制冻结的本次通用 Run 请求。 */
    const request = createVerifyRunRequest(preview);
    executablePreview.current = null;
    runRequestPending.current = true;
    activePreviewRequest.current = null;
    setConfirmedPreview(null);
    setPreviewResponse(null);
    setPreviewError(null);
    setExternalFieldError(null);
    setPreviewPhase("configuring");
    /** 本次 Channel 回调唯一允许落地的 Run generation。 */
    const generation = runGeneration.current + 1;
    runGeneration.current = generation;
    expectedExecutionId.current = null;
    lastSequence.current = -1;
    resetPendingExecutionEventBuffer(pendingExecutionEvents.current);
    terminalAccepted.current = false;
    invalidateCancelRequest();
    setExecutionId(null);
    setOutput(createExecutionOutputBuffer());
    setResult(null);
    setError(null);
    transitionPhase("starting");
    try {
      /** Run 响应只建立可信 Execution ID；Started 只能由 Channel 事件推进。 */
      const response = await commandGateway.runCommandBlock(request, (event) => {
        acceptExecutionEvent(event, generation);
      });
      if (!mounted.current || generation !== runGeneration.current) {
        return;
      }
      runRequestPending.current = false;
      expectedExecutionId.current = response.executionId;
      setExecutionId(response.executionId);
      /** 只把响应 ID 对应的淘汰字节与缓存事件交给当前 Execution。 */
      const buffered = drainPendingExecutionEventBuffer(
        pendingExecutionEvents.current,
        response.executionId,
      );
      setOutput({
        ...createExecutionOutputBuffer(),
        droppedBytes: buffered.droppedOutputBytes,
      });
      for (const event of buffered.events) {
        acceptExecutionEvent(event, generation);
      }
    } catch (runError: unknown) {
      rejectExecutionRun(generation, preview, runError);
    }
  }

  /** 请求 Rust 按已认证的 Execution UUID 终止整个 Job。 */
  async function cancelExecution(): Promise<void> {
    /** 只从 Run 响应建立的可信 ID 读取取消目标。 */
    const targetExecutionId = expectedExecutionId.current;
    /** Cancel 只在 Starting 已有 ID 或 Running 阶段开放。 */
    const currentPhase = phaseSnapshot.current;
    if (
      !commandGateway ||
      !targetExecutionId ||
      activeCancelRequest.current !== null ||
      terminalAccepted.current ||
      (currentPhase !== "starting" && currentPhase !== "running")
    ) {
      return;
    }
    /** 当前 Cancel 严格绑定的 Run generation。 */
    const generation = runGeneration.current;
    /** 本次 Cancel 的组件内唯一 token。 */
    const token = cancelRequestSequence.current + 1;
    cancelRequestSequence.current = token;
    /** 在任何 await 前占用同步门禁，阻止同一 commit 内双击。 */
    const request: ActiveCancelRequest = {
      token,
      runGeneration: generation,
      executionId: targetExecutionId,
    };
    activeCancelRequest.current = request;
    setCancelRequestPending(true);
    setError(null);
    try {
      const response = await commandGateway.cancelExecution(targetExecutionId);
      if (
        activeCancelRequest.current !== request ||
        generation !== runGeneration.current ||
        targetExecutionId !== expectedExecutionId.current ||
        terminalAccepted.current
      ) {
        return;
      }
      if (
        response.state === "cancelling" &&
        (phaseSnapshot.current === "starting" ||
          phaseSnapshot.current === "running")
      ) {
        transitionPhase("cancelling");
      }
    } catch (cancelError: unknown) {
      if (
        activeCancelRequest.current === request &&
        generation === runGeneration.current &&
        targetExecutionId === expectedExecutionId.current &&
        !terminalAccepted.current
      ) {
        setError(normalizeApiError(cancelError));
      }
    } finally {
      if (activeCancelRequest.current === request) {
        activeCancelRequest.current = null;
        setCancelRequestPending(false);
      }
    }
  }

  /** 清空当前 UI Buffer，不影响 Rust Session、日志或终态。 */
  function clearOutput() {
    setOutput(createExecutionOutputBuffer());
  }

  /** 返回当前阶段的用户可读状态。 */
  function phaseLabel(): string {
    if (!commandGateway) return "需要桌面宿主";
    if (phase === "starting") return "正在建立执行";
    if (phase === "running") return "运行中";
    if (phase === "cancelling") return "正在终止进程树";
    if (phase === "finished") return "执行已结束";
    return confirmedPreview ? "可以运行" : "等待 Preview";
  }

  /** 执行窗口外壳动作并隔离其失败，避免污染 Execution 业务状态。 */
  function runWindowAction(action: () => Promise<void>) {
    void action().catch(() => undefined);
  }

  /** 当前工作区标题可使用的真实 Summary 或 Details。 */
  const currentCommand =
    loadedDefinition?.details ??
    commandSummaries.find((summary) => summary.id === selectedCommandId) ??
    null;

  /** 返回当前 Definition/Form 的真实配置状态。 */
  function configurationLabel(): string {
    if (!commandGateway) return "需要桌面宿主";
    if (commandError) return "定义读取失败";
    if (commandLoading || !loadedDefinition) return "正在读取 Definition";
    if (previewPhase === "previewing") return "正在生成 Preview";
    if (previewPhase === "ready") return "Preview 已确认";
    if (previewResponse?.safety.state === "blocked") return "Preview 已拦截";
    if (parameterState.status === "pending") return "正在同步参数";
    if (!parameterState.snapshot.isValid) return "参数需要修正";
    return "等待生成 Preview";
  }

  /** 当前参数快照是否完整、有效且与配置 generation 一致。 */
  const canRequestPreview =
    Boolean(commandGateway && loadedDefinition) &&
    parameterState.status === "ready" &&
    parameterState.snapshot.isValid &&
    parameterState.snapshot.configurationGeneration ===
      configurationGeneration.current &&
    previewPhase !== "previewing" &&
    !isExecutionActive(phase);

  /** normal + notApplicable 是唯一省略 Safety 区域的组合。 */
  const shouldShowSafety =
    previewResponse !== null &&
    !(
      previewResponse.riskLevel === "normal" &&
      previewResponse.safety.state === "notApplicable"
    );

  /** Parameter 快照的当前 wire key 数量，pending 时明确不冒充旧值。 */
  const parameterValueCount =
    parameterState.status === "ready"
      ? Object.keys(parameterState.snapshot.values).length
      : null;

  return (
    <main className="prototype-shell">
      <header className="window-bar" data-tauri-drag-region="deep">
        <div className="brand-lockup" data-tauri-drag-region="deep">
          <img src={cmdboxIcon} alt="" className="brand-mark" />
          <span className="brand-name">CmdBox</span>
          <span className="version-mark">v0.1.0</span>
        </div>
        <div className="window-caption" aria-label="窗口控制">
          <button type="button" className="caption-button" aria-label="最小化窗口" data-tauri-drag-region="false" disabled={!windowControls} onClick={() => windowControls && runWindowAction(windowControls.minimize)}>
            <MinusIcon size={16} weight="light" aria-hidden="true" />
          </button>
          <button type="button" className="caption-button" aria-label="最大化或还原窗口" data-tauri-drag-region="false" disabled={!windowControls} onClick={() => windowControls && runWindowAction(windowControls.toggleMaximize)}>
            <SquareIcon size={13} weight="light" aria-hidden="true" />
          </button>
          <button type="button" className="caption-button caption-button--close" aria-label="关闭窗口" data-tauri-drag-region="false" disabled={!windowControls} onClick={() => windowControls && runWindowAction(windowControls.close)}>
            <XIcon size={16} weight="light" aria-hidden="true" />
          </button>
        </div>
      </header>

      <div className="workspace-grid">
        <nav className="global-navigation" aria-label="主导航">
          <p className="rail-label">导航</p>
          <div className="global-navigation__items">
            <a className="rail-link rail-link--active" href="#command-workspace">
              <TerminalWindowIcon size={21} weight="light" aria-hidden="true" />
              <span>命令</span>
            </a>
            <a className="rail-link" href="#templates"><FileTextIcon size={21} weight="light" aria-hidden="true" /><span>模板</span></a>
            <a className="rail-link" href="#environments"><GitBranchIcon size={21} weight="light" aria-hidden="true" /><span>环境</span></a>
            <a className="rail-link" href="#schedules"><CalendarBlankIcon size={21} weight="light" aria-hidden="true" /><span>计划</span></a>
            <a className="rail-link" href="#history"><ClockCounterClockwiseIcon size={21} weight="light" aria-hidden="true" /><span>历史</span></a>
            <a className="rail-link" href="#settings"><GearIcon size={21} weight="light" aria-hidden="true" /><span>设置</span></a>
          </div>
          <div className="rail-footer">
            <img src={cmdboxIcon} alt="CmdBox" className="rail-footer__mark" />
            <p className="mono-label">CMD BOX</p>
            <p>在命令行确定性之间，建立可重复的一次性桥梁。</p>
            <div className="rail-footer__platform"><span>Windows 优先</span><span>本地执行 · 本地安全</span></div>
          </div>
        </nav>

        <aside className="command-index" aria-label="Command Block 索引">
          <div className="command-index__heading">
            <span>命令块索引</span>
            <button type="button" className="text-action" disabled><PlusIcon size={16} aria-hidden="true" />新建</button>
          </div>
          <label className="search-field">
            <span className="visually-hidden">搜索命令块</span>
            <MagnifyingGlassIcon size={18} aria-hidden="true" />
            <input type="search" placeholder="搜索命令块…" value={query} disabled={isExecutionActive(phase)} onChange={(event) => setQuery(event.currentTarget.value)} />
          </label>
          <p className="index-caption">全部命令块</p>
          <ul className="command-list">
            {filteredCommands.map((command) => (
              <li key={command.id}>
                <button type="button" className={`command-row${command.id === selectedCommandId ? " command-row--selected" : ""}`} aria-current={command.id === selectedCommandId ? "page" : undefined} disabled={isExecutionActive(phase)} onClick={() => void selectCommand(command)}>
                  <span className="command-row__icon"><CommandSummaryIcon summary={command} /></span>
                  <span className="command-row__body"><strong>{command.name}</strong><small>{runnerLabel(command.runner)} <span aria-hidden="true">·</span> {command.origin === "builtin" ? "Built-in" : "User"}</small></span>
                </button>
              </li>
            ))}
            {filteredCommands.length === 0 ? <li className="command-list__empty" role="status">没有匹配的命令块</li> : null}
          </ul>
          <p className="command-count">{query.trim() ? `显示 ${filteredCommands.length} / 已载入 ${commandSummaries.length}` : `已载入 ${commandSummaries.length} 个命令块`}</p>
        </aside>

        <section className="command-workspace" id="command-workspace">
          <header className="workspace-heading">
            <p className="workspace-breadcrumb">命令工作区 <span>/</span> {currentCommand?.name ?? "Command Block"}</p>
            <h1>{currentCommand?.name ?? (commandLoading ? "正在载入命令" : "Command Workspace")}</h1>
            <p>{currentCommand?.description ?? (commandGateway ? "从 Rust Core 读取 Command Block Definition。" : "请在 CmdBox 桌面宿主中读取真实 Command Block。")}</p>
          </header>

          <div className="workspace-content">
            <div className="evidence-column">
              <section className="runner-facts" aria-label="Command Block 配置状态">
                <div><span>执行器</span><strong>{currentCommand ? runnerLabel(currentCommand.runner) : "—"}</strong></div>
                <div><span>状态</span><strong className="state-stale">{configurationLabel()}</strong></div>
              </section>

              {loadedDefinition?.details.parameters.length === 0 ? null : (
                <section className="target-record parameter-record" aria-labelledby="parameter-form-title">
                  <div className="section-heading-row"><h2 id="parameter-form-title">类型化参数</h2>{loadedDefinition ? <span className="fixed-badge">{loadedDefinition.details.parameters.length} 项 Definition</span> : null}</div>
                  {commandError ? <div className="execution-error" role="alert"><strong>{commandError.code}</strong><p>{commandError.message}</p></div> : null}
                  {loadedDefinition ? (
                    <ParameterForm
                      key={`${loadedDefinition.details.id}:${loadedDefinition.details.revision}:${loadedDefinition.generation}`}
                      definition={loadedDefinition.details}
                      disabled={isExecutionActive(phase)}
                      definitionGeneration={loadedDefinition.generation}
                      configurationGeneration={configurationGeneration.current}
                      externalFieldError={externalFieldError}
                      folderPicker={folderPicker}
                      onConfigurationChange={beginConfigurationChange}
                      onStateChange={acceptParameterState}
                    />
                  ) : !commandError ? <p className="parameter-form-empty">{commandLoading ? "正在读取当前 Definition…" : "当前没有可配置的 Command Block。"}</p> : null}
                </section>
              )}

              <section className="preview-record command-preview-record" aria-labelledby="command-preview-title">
                <div className="preview-summary"><span>Preview 状态</span><strong>{previewPhase === "previewing" ? "正在请求 Rust Core" : previewPhase === "ready" ? "当前 Hash 已确认" : previewResponse?.safety.state === "blocked" ? "Safety 已拦截" : "等待当前配置"}</strong></div>
                <div className="section-heading-row"><h2 id="command-preview-title">命令 Preview</h2>{previewResponse ? <span className="fixed-badge">{runnerLabel(previewResponse.runner)}</span> : null}</div>
                {previewError ? <div className="execution-error" role="alert"><strong>{previewError.code}</strong><p>{previewError.message}</p></div> : null}
                {previewResponse ? (
                  <div className="trusted-preview">
                    <dl className="preview-parameters" aria-label="Rust 规范化参数摘要">
                      {previewResponse.parameterSummaries.map((summary) => (
                        <div key={summary.parameterKey}>
                          <dt>{summary.label}</dt>
                          <dd>
                            <span className="preview-values">{summary.displayValues.map((value, index) => <code key={`${summary.parameterKey}:${index}`}>{value}</code>)}</span>
                            <small>{summary.totalCount} 项{summary.truncated ? " · 摘要已截断" : ""}</small>
                          </dd>
                        </div>
                      ))}
                    </dl>
                    <pre aria-label="Rust 生成的 Preview 文本">{previewResponse.previewText}</pre>
                    <dl className="preview-proof">
                      <div><dt>完整大小</dt><dd>{previewResponse.fullSizeBytes} bytes</dd></div>
                      <div><dt>Execution Spec Hash</dt><dd><code>{previewResponse.executionSpecHash}</code></dd></div>
                    </dl>
                    {previewResponse.truncated ? <p className="preview-truncation" role="status">当前可见 Preview 文本已截断；完整大小与 Hash 仍对应 Rust Core 的完整 Artifact。</p> : null}
                  </div>
                ) : previewPhase === "previewing" ? (
                  <div className="preview-stale" role="status"><strong>正在生成当前 Preview</strong><p>Rust Core 正在校验结构化参数并构建完整 Execution Spec。</p></div>
                ) : (
                  <div className="preview-stale"><strong>需要当前 Preview</strong><p>只有当前 Definition 与完整参数快照通过 Rust Core 后，工作区才会保存可执行 Hash。</p></div>
                )}
              </section>

              <section className="preview-record execution-output-record" aria-labelledby="execution-output-title">
                <div className="preview-summary"><span>Execution</span><strong>{executionId ?? "尚未创建"}</strong></div>
                <div className="section-heading-row output-heading-row">
                  <h2 id="execution-output-title">实时输出</h2>
                  <button type="button" className="text-action" onClick={clearOutput} disabled={output.chunks.length === 0}>清空当前显示</button>
                </div>
                {output.droppedBytes > 0 ? <p className="output-truncation" role="status">早期实时输出已有 {output.droppedBytes} 字节未保留；外部任务未因此阻塞。</p> : null}
                <ol className="execution-output" aria-label="Execution 纯文本输出" aria-live="polite">
                  {output.chunks.map((chunk) => <li key={chunk.key} className={`output-line output-line--${chunk.stream}`}><span>{chunk.stream}</span><pre>{chunk.text}</pre></li>)}
                  {output.chunks.length === 0 ? <li className="output-empty">尚无输出。启动后由 Rust Channel 推送纯文本 Batch。</li> : null}
                </ol>
                {error ? <div className="execution-error" role="alert"><strong>{error.code}</strong><p>{error.message}</p></div> : null}
                {result ? <div className={`execution-result execution-result--${result.kind}`} role="status"><strong>{result.kind === "finished" ? "任务自然结束" : result.kind === "cancelled" ? "任务已取消" : "任务内部失败"}</strong><dl><div><dt>耗时</dt><dd>{result.durationMs} ms</dd></div>{result.exitCode !== undefined ? <div><dt>Exit Code</dt><dd>{result.exitCode}</dd></div> : null}<div><dt>终态丢弃</dt><dd>{result.droppedOutputBytes} bytes</dd></div></dl>{result.message ? <p>{result.message}</p> : null}</div> : null}
              </section>
            </div>

            <aside className="annotation-column" aria-label="Definition 与执行边界说明">
              {shouldShowSafety && previewResponse ? (
                <section className={`safety-decision safety-decision--${previewResponse.safety.state}`} aria-label="Safety Decision">
                  <h2>Safety Decision</h2>
                  <div className={`safety-state${previewResponse.safety.state === "blocked" ? " safety-state--stale" : ""}`}>
                    {previewResponse.safety.state === "blocked" ? <WarningIcon size={31} weight="light" aria-hidden="true" /> : <ShieldCheckIcon size={33} weight="light" aria-hidden="true" />}
                    <strong>{previewResponse.safety.state}</strong>
                  </div>
                  {previewResponse.safety.summary ? <p className="safety-summary">{previewResponse.safety.summary}</p> : null}
                  {previewResponse.safety.warnings.length > 0 ? <ul className="safety-warnings">{previewResponse.safety.warnings.map((warning, index) => <li key={`${warning.code}:${index}`}><code>{warning.code}</code><span>{warning.message}</span></li>)}</ul> : null}
                </section>
              ) : null}
              <section className="safety-decision">
                <h2>Definition 身份</h2>
                <div className={`safety-state${loadedDefinition ? "" : " safety-state--stale"}`}>{loadedDefinition ? <ShieldCheckIcon size={33} weight="light" aria-hidden="true" /> : <WarningIcon size={31} weight="light" aria-hidden="true" />}<strong>{loadedDefinition ? "Rust Definition 已载入" : "等待 Rust Definition"}</strong></div>
                <div className="identity-note"><ArrowBendRightDownIcon className="annotation-arrow" size={72} weight="thin" aria-hidden="true" /><div><strong>可信来源</strong><p>名称、说明、Runner、Origin、Risk 和 revision 只来自当前 Summary/Details。</p><p>切换命令会完整重建表单，不继承旧参数。</p></div></div>
              </section>
              <section className="runner-note"><h2>当前 Definition</h2><dl><div><dt>Origin</dt><dd>{currentCommand?.origin ?? "—"}</dd></div><div><dt>Runner</dt><dd>{currentCommand ? runnerLabel(currentCommand.runner) : "—"}</dd></div><div><dt>Risk</dt><dd>{currentCommand?.riskLevel ?? "—"}</dd></div><div><dt>Revision</dt><dd>{currentCommand?.revision ?? "—"}</dd></div></dl><p className="runner-scope">{loadedDefinition && parameterValueCount !== null ? `${parameterValueCount} 个 wire key · ${parameterState.status === "ready" && parameterState.snapshot.isValid ? "UX valid" : "UX invalid"}` : "参数快照正在同步"}</p></section>
              <section className="preflight-note"><h2><PencilSimpleLineIcon size={17} aria-hidden="true" />可信 Preview</h2><ul><li>Zod 只提供即时 UX</li><li>规范化与 Safety 只来自 Rust</li><li>Hash 覆盖完整 Artifact</li></ul><p className="risk-callout">每次 Run 只消费一次当前 Hash；结束或拒绝后必须重新生成 Preview。</p></section>
              <section className="revision-note"><h2>Execution 内核</h2><p>阶段：{phase}</p><p>{phaseLabel()}</p><p>Output：最多 512 KiB</p></section>
            </aside>
          </div>

          <footer className="workspace-actions">
            <div className="execution-summary"><TerminalWindowIcon size={22} weight="light" aria-hidden="true" /><span>{phase === "finished" ? "当前 Execution 已收到唯一后端终态；重新 Preview 后才可再次执行。" : isExecutionActive(phase) ? "当前 Execution 由 Rust Channel 推进；配置保持锁定直到唯一终态。" : confirmedPreview ? "当前 Rust Preview 已绑定不可变参数快照与完整 Execution Spec Hash，可执行一次。" : previewResponse?.safety.state === "blocked" ? "Rust Safety 已拦截当前 Preview；不会形成可执行授权。" : commandGateway ? "填写有效参数并生成 Rust Preview；Run 不接受脚本或进程参数。" : "当前是纯浏览器环境；可以查看降级状态，但不能请求桌面 Preview 或 Run。"}</span></div>
            <div className="action-buttons">
              {commandGateway ? <button type="button" className="secondary-button" onClick={requestCurrentPreview} disabled={!canRequestPreview}>{previewPhase === "previewing" ? "正在生成 Preview" : previewError || previewResponse?.safety.state === "blocked" ? "重试 Preview" : confirmedPreview || phase === "finished" ? "重新生成 Preview" : "生成 Preview"}</button> : null}
              {(phase === "starting" && executionId) || phase === "running" || phase === "cancelling" ? <button type="button" className="cancel-button" onClick={cancelExecution} disabled={phase === "cancelling" || cancelRequestPending}><XIcon size={18} aria-hidden="true" />{phase === "cancelling" ? "正在终止" : cancelRequestPending ? "正在请求" : "终止任务"}</button> : phase === "starting" ? <button type="button" className="primary-button" disabled><TerminalWindowIcon size={19} aria-hidden="true" />正在启动</button> : confirmedPreview && phase === "ready" ? <button type="button" className="primary-button" onClick={startExecution}><TerminalWindowIcon size={19} aria-hidden="true" />{confirmedPreview.response.actionLabel}</button> : !commandGateway ? <button type="button" className="primary-button" disabled><TerminalWindowIcon size={19} aria-hidden="true" />需要桌面宿主</button> : null}
            </div>
          </footer>
        </section>
      </div>
    </main>
  );
}
