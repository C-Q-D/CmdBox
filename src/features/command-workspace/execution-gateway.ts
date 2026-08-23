/**
 * Command Block Execution 的前端 Typed IPC Gateway。
 *
 * 本模块是前端唯一知道五个 Rust Command 名称和 Tauri Channel 构造方式的边界。公开请求
 * 直接使用 Rust 生成的 TypeScript Contract，不接受脚本、可执行文件、Runner options、
 * 工作目录、环境、PID 或任意进程终止旁路。
 */
import { Channel, invoke, isTauri } from "@tauri-apps/api/core";
import type {
  ApiError as WireApiError,
  CancelExecutionResponse,
  CommandBlockDetails,
  CommandBlockSummary,
  ExecutionStreamEvent,
  IpcActiveExecutionState,
  IpcOutputFragment,
  PreviewCommandRequest,
  PreviewCommandResponse,
  RunCommandResponse,
  VerifyRunRequest,
} from "../../generated/contracts";

export type {
  CancelExecutionResponse,
  CommandBlockDetails,
  CommandBlockSummary,
  ExecutionStreamEvent,
  PreviewCommandRequest,
  PreviewCommandResponse,
  RunCommandResponse,
  VerifyRunRequest,
} from "../../generated/contracts";

/** Rust IPC 当前明确发布并由前端固定处理的错误码。 */
export const PUBLISHED_API_ERROR_CODES = [
  "COMMAND_BLOCK_NOT_FOUND",
  "REVISION_CONFLICT",
  "VALIDATION_FAILED",
  "INVALID_TEMPLATE",
  "UNSUPPORTED_RUNNER",
  "RUNNER_UNAVAILABLE",
  "INTERNAL_CONTRACT",
  "STALE_PREVIEW",
  "ARTIFACT_PREPARATION_FAILED",
  "PROCESS_START_FAILED",
  "EXECUTION_START_FAILED",
  "CANCEL_FAILED",
] as const;

/** 前端可稳定分支处理的公开错误码，另含本地 IPC 收敛码。 */
export type ApiErrorCode =
  | (typeof PUBLISHED_API_ERROR_CODES)[number]
  | "IPC_FAILED";

/** 经前端白名单收敛后的安全错误，不直接信任拒绝对象中的 message。 */
export type ApiError = Omit<WireApiError, "code" | "message"> & {
  /** 已发布后端码或本地 IPC 收敛码。 */
  code: ApiErrorCode;
  /** 由前端固定映射且不含拒绝对象内容的安全说明。 */
  message: string;
};

/** 兼容现有 Workspace 的固定任务启动响应，与通用 Run 响应结构相同。 */
export type StartFixedExecutionResponse = RunCommandResponse;

/** 兼容现有 Workspace 的 Active 状态别名。 */
export type ActiveExecutionState = IpcActiveExecutionState;

/** 兼容现有 Workspace 的 Output Fragment 别名。 */
export type ExecutionOutputFragment = IpcOutputFragment;

/** Command Workspace 后续通用流程使用的窄业务 Gateway。 */
export interface CommandExecutionGateway {
  /** 按 Rust 固定顺序读取公开 Command Block 摘要。 */
  listCommandBlocks(): Promise<CommandBlockSummary[]>;
  /** 按业务 ID 读取不含内部模板或启动配置的公开详情。 */
  getCommandBlock(commandBlockId: string): Promise<CommandBlockDetails>;
  /** 提交结构化参数并取得 Rust Core 生成的可信 Preview。 */
  previewCommandBlock(
    request: PreviewCommandRequest,
  ): Promise<PreviewCommandResponse>;
  /** 使用已确认 Hash 复验并运行，事件只经本次调用专属 Channel 返回。 */
  runCommandBlock(
    request: VerifyRunRequest,
    onEvent: (event: ExecutionStreamEvent) => void,
  ): Promise<RunCommandResponse>;
  /** 按 Execution UUID 请求终止对应 Job。 */
  cancelExecution(executionId: string): Promise<CancelExecutionResponse>;
}

/** 现有 Workspace 在后续 UI-RUN 原子前继续接受注入的固定任务 Gateway 类型。 */
export interface FixedExecutionGateway {
  /** 仅供现有测试替身保持固定任务调用契约。 */
  startFixedExecution(
    onEvent: (event: ExecutionStreamEvent) => void,
  ): Promise<StartFixedExecutionResponse>;
  /** 按 Execution UUID 请求终止对应 Job。 */
  cancelExecution(executionId: string): Promise<CancelExecutionResponse>;
}

/** Gateway 可注入的最小 Tauri Transport，避免单元测试伪造全局对象。 */
export interface ExecutionTransport {
  /** 当前宿主是否支持 Tauri IPC。 */
  isAvailable(): boolean;
  /** 创建一个可作为 invoke 参数序列化的专属 Channel。 */
  createChannel<T>(onMessage: (message: T) => void): unknown;
  /** 调用一个已经注册的窄 Rust Command。 */
  invoke<T>(command: string, arguments_: Record<string, unknown>): Promise<T>;
}

/** 生产环境使用的 Tauri 2 Transport。 */
const tauriTransport: ExecutionTransport = {
  /** 使用官方宿主检测，不读取或伪造全局 Tauri 对象。 */
  isAvailable(): boolean {
    return isTauri();
  },
  /** 创建官方高吞吐 IPC Channel。 */
  createChannel<T>(onMessage: (message: T) => void): Channel<T> {
    return new Channel<T>(onMessage);
  },
  /** 通过官方 API 调用已注册的 Rust Command。 */
  invoke<T>(command: string, arguments_: Record<string, unknown>): Promise<T> {
    return invoke<T>(command, arguments_);
  },
};

/**
 * 创建通用 Command Block Gateway。
 *
 * @param transport 生产环境默认使用官方 Tauri Transport；测试传入无副作用替身。
 * @returns Tauri 宿主中返回窄业务边界，纯浏览器环境返回 `null`。
 */
export function createCommandExecutionGateway(
  transport: ExecutionTransport = tauriTransport,
): CommandExecutionGateway | null {
  if (!transport.isAvailable()) {
    return null;
  }
  return {
    /** 不携带任何调用参数读取公开摘要。 */
    listCommandBlocks(): Promise<CommandBlockSummary[]> {
      return invokeSafely(transport, "list_command_blocks", {});
    },
    /** 只把业务 ID 传给详情命令。 */
    getCommandBlock(commandBlockId: string): Promise<CommandBlockDetails> {
      return invokeSafely(transport, "get_command_block", {
        commandBlockId,
      });
    },
    /** 只把 Rust 生成契约允许的结构化 Request 传给 Preview。 */
    previewCommandBlock(
      request: PreviewCommandRequest,
    ): Promise<PreviewCommandResponse> {
      return invokeSafely(transport, "preview_command_block", { request });
    },
    /** 仅 Run 创建当前 Execution 专属 Channel，不使用全局事件。 */
    runCommandBlock(
      request: VerifyRunRequest,
      onEvent: (event: ExecutionStreamEvent) => void,
    ): Promise<RunCommandResponse> {
      const onEventChannel = transport.createChannel(onEvent);
      return invokeSafely(transport, "run_command_block", {
        request,
        onEvent: onEventChannel,
      });
    },
    /** 只把 Execution UUID 传给取消命令，不接受 PID。 */
    cancelExecution(executionId: string): Promise<CancelExecutionResponse> {
      return invokeSafely(transport, "cancel_execution", { executionId });
    },
  };
}

/**
 * 保留现有 Workspace 的旧 Factory 出口，但不再构造后端已移除的固定任务 Gateway。
 *
 * @returns 固定返回 `null`；测试需要旧 UI 状态时应直接注入 `FixedExecutionGateway` 替身。
 */
export function createFixedExecutionGateway(): null {
  return null;
}

/** 调用一个固定 Command，并把任意拒绝值收敛成安全前端错误。 */
async function invokeSafely<T>(
  transport: ExecutionTransport,
  command: string,
  arguments_: Record<string, unknown>,
): Promise<T> {
  try {
    return await transport.invoke<T>(command, arguments_);
  } catch (error: unknown) {
    throw normalizeApiError(error);
  }
}

/** 把未知拒绝值收敛为不会泄露任意对象内容的稳定前端错误。 */
export function normalizeApiError(error: unknown): ApiError {
  if (typeof error !== "object" || error === null) {
    return ipcFallbackError();
  }
  const candidate = error as Record<string, unknown>;
  if (!isPublishedApiErrorCode(candidate.code)) {
    return ipcFallbackError();
  }
  if (!isOptionalString(candidate, "parameterKey")) {
    return ipcFallbackError();
  }
  if (!isOptionalString(candidate, "detailCode")) {
    return ipcFallbackError();
  }

  const normalized: ApiError = {
    code: candidate.code,
    message: PUBLIC_ERROR_MESSAGES[candidate.code],
  };
  if (typeof candidate.parameterKey === "string") {
    normalized.parameterKey = candidate.parameterKey;
  }
  if (typeof candidate.detailCode === "string") {
    normalized.detailCode = candidate.detailCode;
  }
  return normalized;
}

/** 返回不包含原始拒绝值的统一 IPC 失败。 */
function ipcFallbackError(): ApiError {
  return {
    code: "IPC_FAILED",
    message: "CmdBox 无法完成桌面宿主调用",
  };
}

/** 判断一个候选值是否为 Rust 当前发布的错误码。 */
function isPublishedApiErrorCode(
  code: unknown,
): code is (typeof PUBLISHED_API_ERROR_CODES)[number] {
  return (
    typeof code === "string" &&
    Object.prototype.hasOwnProperty.call(PUBLIC_ERROR_MESSAGES, code)
  );
}

/** 检查可选错误定位字段不存在或严格为字符串。 */
function isOptionalString(
  candidate: Record<string, unknown>,
  key: "parameterKey" | "detailCode",
): boolean {
  return !(key in candidate) || typeof candidate[key] === "string";
}

/** Rust 已发布错误码到固定安全文案的唯一前端映射。 */
const PUBLIC_ERROR_MESSAGES = {
  COMMAND_BLOCK_NOT_FOUND: "未找到指定的 Command Block",
  REVISION_CONFLICT: "Command Block 已更新，请重新载入",
  VALIDATION_FAILED: "请求参数未通过校验",
  INVALID_TEMPLATE: "Command Block 模板无效",
  UNSUPPORTED_RUNNER: "当前 Runner 尚不支持",
  RUNNER_UNAVAILABLE: "系统 Runner 不可用",
  INTERNAL_CONTRACT: "Command Block 内部契约无效",
  STALE_PREVIEW: "Preview 已失效，请重新生成",
  ARTIFACT_PREPARATION_FAILED: "无法准备 Execution 临时脚本",
  PROCESS_START_FAILED: "无法启动 Execution 进程",
  EXECUTION_START_FAILED: "无法建立 Execution 后台任务",
  CANCEL_FAILED: "无法终止当前 Execution",
} as const satisfies Record<
  (typeof PUBLISHED_API_ERROR_CODES)[number],
  string
>;
