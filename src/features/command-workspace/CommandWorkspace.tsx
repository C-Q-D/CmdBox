/**
 * 真实 Command Block Definition 驱动的统一 Command Workspace。
 *
 * 组件继承用户确认的 editorial-field-notes 视觉语法，读取真实 Summary/Details 并渲染
 * 统一 Parameter Form 与 Rust Preview，同时完整保留已经验证的 CMD-01 Execution
 * 生命周期与 Output 内核。Preview 只消费 Rust 事实，本模块不渲染 Shell、不计算 Hash，
 * 也不会在 UI-RUN 原子前调用通用 Run 或创建 Execution Channel。
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
  createFixedExecutionGateway,
  normalizeApiError,
  type ApiError,
  type CommandBlockDetails,
  type CommandBlockSummary,
  type CommandExecutionGateway,
  type ExecutionStreamEvent,
  type FixedExecutionGateway,
  type PreviewCommandResponse,
} from "./execution-gateway";
import {
  appendExecutionOutput,
  createExecutionOutputBuffer,
  EXECUTION_OUTPUT_LIMIT_BYTES,
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
  type ConfirmedPreview,
  type DeepReadonly,
  type PreviewAttempt,
} from "./preview-state";

/** 旧固定任务 Gateway 已停止生产装配，仅保留测试注入 seam。 */
const defaultGateway = createFixedExecutionGateway();

/** 浏览器加载时解析一次通用 Command Block Gateway；非 Tauri 环境保持 `null`。 */
const defaultCommandGateway = createCommandExecutionGateway();

/** 浏览器加载时解析一次生产目录选择接缝；非 Tauri 环境保持 `null`。 */
const defaultFolderPicker = createFolderPicker();

/** 浏览器加载时解析一次当前窗口控制；非 Tauri 环境保持 `null`。 */
const defaultWindowControls = createDesktopWindowControls();

/** 启动响应前最多缓存的事件数，限制无文本元数据事件的内存占用。 */
const PENDING_EVENT_LIMIT = 2048;

/** 预响应淘汰账本最多跟踪的不同 Execution ID 数量。 */
const PENDING_DROPPED_EXECUTION_LIMIT = 64;

/** 估算一个待认证事件占用的 UTF-8 负载字节数，并给元数据预留固定预算。 */
function pendingEventBytes(event: ExecutionStreamEvent): number {
  const metadataBudget = 128;
  if (event.event !== "output") {
    return metadataBudget;
  }
  return metadataBudget + event.data.fragments.reduce(
    (total, fragment) => total + new TextEncoder().encode(fragment.text).byteLength,
    0,
  );
}

/** 计算淘汰一个预响应 Output 事件时未保留的文本字节数。 */
function pendingOutputBytes(event: ExecutionStreamEvent): number {
  if (event.event !== "output") {
    return 0;
  }
  return event.data.fragments.reduce(
    (total, fragment) => total + new TextEncoder().encode(fragment.text).byteLength,
    0,
  );
}

/** Workspace 可由测试注入无副作用 Gateway。 */
export interface CommandWorkspaceProps {
  /** 仅供既有 Execution 回归测试注入的固定任务 IPC；生产默认不再装配。 */
  gateway?: FixedExecutionGateway | null;
  /** Summary/Details/Preview/Run 的窄业务 Gateway；本原子只使用前两项。 */
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

/** 判断既有 Execution phase 是否必须锁定命令配置。 */
function isExecutionActive(phase: WorkspacePhase): boolean {
  return phase === "starting" || phase === "running" || phase === "cancelling";
}

/** 渲染并控制 CMD-01 的真实固定任务工作区。 */
export function CommandWorkspace({
  gateway = defaultGateway,
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
  /** 每次 Run 递增，用于隔离旧 Channel 回调。 */
  const runGeneration = useRef(0);
  /** 当前 Channel 已绑定的 Execution UUID。 */
  const expectedExecutionId = useRef<string | null>(null);
  /** 当前 Execution 已接受的最大事件级 sequence。 */
  const lastSequence = useRef(-1);
  /** 启动响应返回前暂存的 Channel 事件。 */
  const pendingEvents = useRef<ExecutionStreamEvent[]>([]);
  /** 当前预响应缓存的估算 UTF-8 负载字节数。 */
  const pendingEventsBytes = useRef(0);
  /** 按 Execution ID 隔离的预响应 Output 淘汰账本。 */
  const pendingDroppedOutputBytes = useRef(new Map<string, number>());
  /** 当前 Execution 是否已经接受唯一终态。 */
  const terminalAccepted = useRef(false);
  /** 与 React 状态同步的阶段快照，供异步响应防止状态倒退。 */
  const phaseSnapshot = useRef<WorkspacePhase>("ready");
  /** 当前仍有权修改取消请求状态的 generation。 */
  const cancelRequestGeneration = useRef<number | null>(null);
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
  /** 与 React 状态同步的 ConfirmedPreview，供下一原子安全接线。 */
  const confirmedPreviewSnapshot = useRef<ConfirmedPreview | null>(null);
  /** 当前选择的 id/revision/generation 身份快照。 */
  const selectedDefinitionIdentity = useRef<{
    id: string;
    revision: number;
    generation: number;
  } | null>(null);

  /** 清除当前 Preview 展示、授权和错误，不改变 Definition 或 Execution。 */
  const clearPreviewState = useCallback(() => {
    activePreviewRequest.current = null;
    confirmedPreviewSnapshot.current = null;
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
  async function selectCommand(summary: CommandBlockSummary) {
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
      };
    }
    const generation = listGeneration.current + 1;
    listGeneration.current = generation;
    setCommandLoading(true);
    void commandGateway
      .listCommandBlocks()
      .then((summaries) => {
        if (!mounted.current || listGeneration.current !== generation) {
          return;
        }
        setCommandSummaries([...summaries]);
        setCommandLoading(false);
        if (summaries[0]) {
          void selectCommand(summaries[0]);
        }
      })
      .catch((loadError: unknown) => {
        if (mounted.current && listGeneration.current === generation) {
          setCommandError(loadError as ApiError);
          setCommandLoading(false);
        }
      });
    return () => {
      mounted.current = false;
      listGeneration.current += 1;
      definitionGeneration.current += 1;
      configurationGeneration.current += 1;
      selectedDefinitionIdentity.current = null;
      activePreviewRequest.current = null;
    };
  }, [commandGateway]);

  /** 同步更新界面阶段和异步阶段快照。 */
  function transitionPhase(nextPhase: WorkspacePhase) {
    phaseSnapshot.current = nextPhase;
    setPhase(nextPhase);
  }

  /** 在有界分桶中记录一个被淘汰 Output 的文本和 Rust 已报告丢弃字节。 */
  function recordPendingOutputDrop(event: ExecutionStreamEvent) {
    if (event.event !== "output") {
      return;
    }
    const { executionId: droppedExecutionId, droppedBytesBefore } = event.data;
    const ledger = pendingDroppedOutputBytes.current;
    if (!ledger.has(droppedExecutionId) && ledger.size >= PENDING_DROPPED_EXECUTION_LIMIT) {
      const oldestExecutionId = ledger.keys().next().value as string | undefined;
      if (oldestExecutionId) {
        ledger.delete(oldestExecutionId);
      }
    }
    ledger.set(
      droppedExecutionId,
      (ledger.get(droppedExecutionId) ?? 0) + pendingOutputBytes(event) + droppedBytesBefore,
    );
  }

  /** 有界暂存响应前事件；超限时优先淘汰文本事件并保留生命周期事实。 */
  function queuePendingEvent(event: ExecutionStreamEvent) {
    pendingEvents.current.push(event);
    pendingEventsBytes.current += pendingEventBytes(event);
    while (
      pendingEventsBytes.current > EXECUTION_OUTPUT_LIMIT_BYTES ||
      pendingEvents.current.length > PENDING_EVENT_LIMIT
    ) {
      const outputIndex = pendingEvents.current.findIndex(
        (candidate) => candidate.event === "output",
      );
      const removalIndex = outputIndex >= 0 ? outputIndex : 0;
      const [removed] = pendingEvents.current.splice(removalIndex, 1);
      if (!removed) {
        break;
      }
      pendingEventsBytes.current -= pendingEventBytes(removed);
      recordPendingOutputDrop(removed);
    }
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
    confirmedPreviewSnapshot.current = null;
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
      confirmedPreviewSnapshot.current = acceptance.confirmedPreview;
      setConfirmedPreview(acceptance.confirmedPreview);
      setPreviewResponse(acceptance.confirmedPreview.response);
      setPreviewPhase("ready");
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

  /** 接受当前 Channel 的严格有序后端事件并更新可观察状态。 */
  function acceptExecutionEvent(event: ExecutionStreamEvent, generation: number) {
    if (generation !== runGeneration.current) {
      return;
    }
    if (expectedExecutionId.current === null) {
      queuePendingEvent(event);
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
        transitionPhase("running");
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
        terminalAccepted.current = true;
        setResult({
          kind: "finished",
          exitCode: event.data.exitCode,
          durationMs: event.data.durationMs,
          droppedOutputBytes: event.data.droppedOutputBytes,
        });
        transitionPhase("finished");
        return;
      case "cancelled":
        terminalAccepted.current = true;
        setResult({
          kind: "cancelled",
          durationMs: event.data.durationMs,
          droppedOutputBytes: event.data.droppedOutputBytes,
        });
        transitionPhase("finished");
        return;
      case "failed":
        terminalAccepted.current = true;
        setResult({
          kind: "failed",
          message: event.data.message,
          durationMs: event.data.durationMs,
          droppedOutputBytes: event.data.droppedOutputBytes,
        });
        transitionPhase("finished");
    }
  }

  /** 请求 Rust 启动固定验收任务；不传入任何脚本或进程参数。 */
  async function startExecution() {
    if (!gateway || phase === "starting" || phase === "running" || phase === "cancelling") {
      return;
    }
    const generation = runGeneration.current + 1;
    runGeneration.current = generation;
    expectedExecutionId.current = null;
    lastSequence.current = -1;
    pendingEvents.current = [];
    pendingEventsBytes.current = 0;
    pendingDroppedOutputBytes.current.clear();
    terminalAccepted.current = false;
    cancelRequestGeneration.current = null;
    setCancelRequestPending(false);
    setExecutionId(null);
    setOutput(createExecutionOutputBuffer());
    setResult(null);
    setError(null);
    transitionPhase("starting");
    try {
      const response = await gateway.startFixedExecution((event) => {
        acceptExecutionEvent(event, generation);
      });
      if (generation !== runGeneration.current) {
        return;
      }
      expectedExecutionId.current = response.executionId;
      setExecutionId(response.executionId);
      const bufferedEvents = pendingEvents.current;
      pendingEvents.current = [];
      pendingEventsBytes.current = 0;
      const authenticatedDroppedBytes =
        pendingDroppedOutputBytes.current.get(response.executionId) ?? 0;
      setOutput({
        ...createExecutionOutputBuffer(),
        droppedBytes: authenticatedDroppedBytes,
      });
      pendingDroppedOutputBytes.current.clear();
      for (const event of bufferedEvents) {
        acceptExecutionEvent(event, generation);
      }
    } catch (startError: unknown) {
      if (
        generation === runGeneration.current &&
        expectedExecutionId.current === null
      ) {
        pendingEvents.current = [];
        pendingEventsBytes.current = 0;
        pendingDroppedOutputBytes.current.clear();
        setError(startError as ApiError);
        transitionPhase("ready");
      }
    }
  }

  /** 请求 Rust 按当前 Execution UUID 终止整个 Job。 */
  async function cancelExecution() {
    if (!gateway || !executionId || phase !== "running" || cancelRequestPending) {
      return;
    }
    const generation = runGeneration.current;
    const targetExecutionId = executionId;
    cancelRequestGeneration.current = generation;
    setCancelRequestPending(true);
    setError(null);
    try {
      const response = await gateway.cancelExecution(targetExecutionId);
      if (
        generation === runGeneration.current &&
        targetExecutionId === expectedExecutionId.current &&
        !terminalAccepted.current &&
        phaseSnapshot.current === "running" &&
        response.state === "cancelling"
      ) {
        transitionPhase("cancelling");
      }
    } catch (cancelError: unknown) {
      if (
        generation === runGeneration.current &&
        targetExecutionId === expectedExecutionId.current &&
        !terminalAccepted.current
      ) {
        setError(cancelError as ApiError);
      }
    } finally {
      if (cancelRequestGeneration.current === generation) {
        cancelRequestGeneration.current = null;
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
    if (!gateway) return commandGateway ? "Run 尚未接线" : "需要桌面宿主";
    if (phase === "starting") return "正在建立执行";
    if (phase === "running") return "运行中";
    if (phase === "cancelling") return "正在终止进程树";
    if (phase === "finished") return "执行已结束";
    return "可以运行";
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
              <section className="preflight-note"><h2><PencilSimpleLineIcon size={17} aria-hidden="true" />可信 Preview</h2><ul><li>Zod 只提供即时 UX</li><li>规范化与 Safety 只来自 Rust</li><li>Hash 覆盖完整 Artifact</li></ul><p className="risk-callout">本阶段只确认 Preview；通用 Run 与 Channel 尚未接线。</p></section>
              <section className="revision-note"><h2>Execution 内核</h2><p>阶段：{phase}</p><p>{phaseLabel()}</p><p>Output：最多 512 KiB</p></section>
            </aside>
          </div>

          <footer className="workspace-actions">
            <div className="execution-summary"><TerminalWindowIcon size={22} weight="light" aria-hidden="true" /><span>{gateway ? phase === "finished" ? "既有 Execution 内核已收到唯一后端终态，可再次运行回归任务。" : "既有固定 Execution 测试接缝保持回归；通用 Preview 与它正交。" : confirmedPreview ? "当前 Rust Preview 已绑定不可变参数快照与完整 Execution Spec Hash；Run 将在下一原子接线。" : previewResponse?.safety.state === "blocked" ? "Rust Safety 已拦截当前 Preview；不会形成可执行授权。" : commandGateway ? "填写有效参数并生成 Rust Preview；当前阶段不会启动命令。" : "当前是纯浏览器环境；可以查看降级状态，但不能请求桌面 Preview。"}</span></div>
            <div className="action-buttons">
              {gateway ? <button type="button" className="secondary-button" onClick={clearOutput} disabled={output.chunks.length === 0}>清空输出</button> : <button type="button" className="secondary-button" onClick={requestCurrentPreview} disabled={!canRequestPreview}>{previewPhase === "previewing" ? "正在生成 Preview" : previewError || previewResponse?.safety.state === "blocked" ? "重试 Preview" : confirmedPreview ? "重新生成 Preview" : "生成 Preview"}</button>}
              {phase === "running" || phase === "cancelling" ? <button type="button" className="cancel-button" onClick={cancelExecution} disabled={phase === "cancelling" || cancelRequestPending}><XIcon size={18} aria-hidden="true" />{phase === "cancelling" ? "正在终止" : cancelRequestPending ? "正在请求" : "终止任务"}</button> : gateway ? <button type="button" className="primary-button" onClick={startExecution} disabled={phase === "starting"}><TerminalWindowIcon size={19} aria-hidden="true" />{phase === "starting" ? "正在启动" : phase === "finished" ? "再次运行" : "运行验收任务"}</button> : confirmedPreview ? <button type="button" className="primary-button" disabled><TerminalWindowIcon size={19} aria-hidden="true" />{confirmedPreview.response.actionLabel}</button> : !commandGateway ? <button type="button" className="primary-button" disabled><TerminalWindowIcon size={19} aria-hidden="true" />需要桌面宿主</button> : null}
            </div>
          </footer>
        </section>
      </div>
    </main>
  );
}
