/**
 * 固定 Execution 的前端 Typed IPC Gateway。
 *
 * 本模块是 Command Workspace 唯一知道 Tauri 命令名和 Channel 构造方式的边界。
 * 它只允许启动 Rust 内置验收任务并按 Execution ID 取消，不接受脚本、路径、PID 或可执行文件。
 */
import { Channel, invoke, isTauri } from "@tauri-apps/api/core";

/** Rust IPC 当前发布的稳定错误码。 */
export type ApiErrorCode =
  | "VALIDATION_FAILED"
  | "CANCEL_FAILED"
  | "RUNNER_UNAVAILABLE"
  | "ARTIFACT_PREPARATION_FAILED"
  | "PROCESS_START_FAILED"
  | "EXECUTION_START_FAILED"
  | "IPC_FAILED";

/** Rust IPC 返回的稳定错误。 */
export interface ApiError {
  /** 供界面按稳定语义处理的错误码。 */
  code: ApiErrorCode;
  /** 不包含本机私有路径或底层错误细节的公开说明。 */
  message: string;
}

/** 固定任务启动响应。 */
export interface StartFixedExecutionResponse {
  /** Rust Core 分配的 Execution UUID。 */
  executionId: string;
}

/** Rust Core 暴露的 Active 状态。 */
export type ActiveExecutionState = "running" | "cancelling";

/** 取消请求响应。 */
export interface CancelExecutionResponse {
  /** 本次调用是否首次接受取消请求。 */
  accepted: boolean;
  /** Execution 不存在或已经终止时为 null。 */
  state: ActiveExecutionState | null;
}

/** 输出片段来自哪个标准流。 */
export type ExecutionOutputStream = "stdout" | "stderr";

/** Rust Output Coordinator 生成的一个纯文本片段。 */
export interface ExecutionOutputFragment {
  /** Output Coordinator 分配的片段级顺序。 */
  fragmentSequence: number;
  /** 片段所属标准流。 */
  stream: ExecutionOutputStream;
  /** 已由 Rust 增量解码的不可信纯文本。 */
  text: string;
}

/** Session 已登记且受管进程即将恢复。 */
export interface ExecutionStartedEvent {
  /** 事件类型判别字段。 */
  event: "started";
  /** Started 的结构化事实。 */
  data: {
    /** 当前 Execution UUID。 */
    executionId: string;
    /** IPC 转发器分配的事件级顺序。 */
    sequence: number;
  };
}

/** 一个有界实时 Output Batch。 */
export interface ExecutionOutputEvent {
  /** 事件类型判别字段。 */
  event: "output";
  /** Output Batch 的结构化事实。 */
  data: {
    /** 当前 Execution UUID。 */
    executionId: string;
    /** IPC 转发器分配的事件级顺序。 */
    sequence: number;
    /** 保持 Rust Coordinator 观察顺序的纯文本片段。 */
    fragments: ExecutionOutputFragment[];
    /** 当前 Batch 之前因有界队列压力被丢弃的字节数。 */
    droppedBytesBefore: number;
  };
}

/** 根进程自然结束且 Job 已清空。 */
export interface ExecutionFinishedEvent {
  /** 事件类型判别字段。 */
  event: "finished";
  /** Finished 的结构化事实。 */
  data: {
    /** 当前 Execution UUID。 */
    executionId: string;
    /** IPC 转发器分配的事件级顺序。 */
    sequence: number;
    /** Windows PowerShell 原始 Exit Code。 */
    exitCode: number;
    /** Rust Core 从 Resume 到终态的毫秒数。 */
    durationMs: number;
    /** 尚未随 Output Batch 报告的丢弃字节数。 */
    droppedOutputBytes: number;
  };
}

/** 取消已被接受且整个 Job 已确认结束。 */
export interface ExecutionCancelledEvent {
  /** 事件类型判别字段。 */
  event: "cancelled";
  /** Cancelled 的结构化事实。 */
  data: {
    /** 当前 Execution UUID。 */
    executionId: string;
    /** IPC 转发器分配的事件级顺序。 */
    sequence: number;
    /** Rust Core 从 Resume 到终态的毫秒数。 */
    durationMs: number;
    /** 尚未随 Output Batch 报告的丢弃字节数。 */
    droppedOutputBytes: number;
  };
}

/** Resume 后发生的 Rust Core 内部失败。 */
export interface ExecutionFailedEvent {
  /** 事件类型判别字段。 */
  event: "failed";
  /** Failed 的结构化事实。 */
  data: {
    /** 当前 Execution UUID。 */
    executionId: string;
    /** IPC 转发器分配的事件级顺序。 */
    sequence: number;
    /** Rust Core 提供的稳定公开失败说明。 */
    message: string;
    /** Rust Core 从 Resume 到终态的毫秒数。 */
    durationMs: number;
    /** 尚未随 Output Batch 报告的丢弃字节数。 */
    droppedOutputBytes: number;
  };
}

/** 专属 Tauri Channel 上可能出现的全部 Execution 事件。 */
export type ExecutionStreamEvent =
  | ExecutionStartedEvent
  | ExecutionOutputEvent
  | ExecutionFinishedEvent
  | ExecutionCancelledEvent
  | ExecutionFailedEvent;

/** Command Workspace 使用的最小固定任务 Gateway。 */
export interface FixedExecutionGateway {
  /** 启动内置固定任务，并在专属 Channel 到达事件时同步通知调用方。 */
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
 * 创建固定任务 Gateway。
 *
 * @param transport 生产环境默认使用官方 Tauri Transport；测试传入无副作用替身。
 * @returns Tauri 宿主中返回窄业务边界，纯浏览器环境返回 `null`。
 */
export function createFixedExecutionGateway(
  transport: ExecutionTransport = tauriTransport,
): FixedExecutionGateway | null {
  if (!transport.isAvailable()) {
    return null;
  }
  return {
    /** 创建专属事件 Channel 后启动 Rust 内置任务。 */
    async startFixedExecution(
      onEvent: (event: ExecutionStreamEvent) => void,
    ): Promise<StartFixedExecutionResponse> {
      const onEventChannel = transport.createChannel(onEvent);
      try {
        return await transport.invoke<StartFixedExecutionResponse>(
          "start_fixed_execution",
          { onEvent: onEventChannel },
        );
      } catch (error: unknown) {
        throw normalizeApiError(error);
      }
    },
    /** 把 UUID 作为唯一取消标识传给 Rust，不接受 PID。 */
    async cancelExecution(
      executionId: string,
    ): Promise<CancelExecutionResponse> {
      try {
        return await transport.invoke<CancelExecutionResponse>(
          "cancel_execution",
          { executionId },
        );
      } catch (error: unknown) {
        throw normalizeApiError(error);
      }
    },
  };
}

/** 把未知拒绝值收敛为不会泄露任意对象内容的稳定前端错误。 */
export function normalizeApiError(error: unknown): ApiError {
  const code = publishedApiErrorCode(error);
  if (code) {
    return { code, message: PUBLIC_ERROR_MESSAGES[code] };
  }
  return {
    code: "IPC_FAILED",
    message: "CmdBox 无法完成桌面宿主调用",
  };
}

/** Rust 已发布错误码到固定安全文案的唯一映射。 */
const PUBLIC_ERROR_MESSAGES = {
  VALIDATION_FAILED: "Execution ID 无效",
  CANCEL_FAILED: "无法终止当前 Execution",
  RUNNER_UNAVAILABLE: "系统 Windows PowerShell 不可用",
  ARTIFACT_PREPARATION_FAILED: "无法准备固定任务临时脚本",
  PROCESS_START_FAILED: "无法启动固定 PowerShell 任务",
  EXECUTION_START_FAILED: "无法建立 Execution 后台任务",
} as const satisfies Record<Exclude<ApiErrorCode, "IPC_FAILED">, string>;

/** 只从未知拒绝值中接受 Rust 当前明确发布的错误码。 */
function publishedApiErrorCode(
  error: unknown,
): Exclude<ApiErrorCode, "IPC_FAILED"> | null {
  if (typeof error !== "object" || error === null) {
    return null;
  }
  const candidate = error as Record<string, unknown>;
  if (
    typeof candidate.code === "string" &&
    Object.prototype.hasOwnProperty.call(PUBLIC_ERROR_MESSAGES, candidate.code)
  ) {
    return candidate.code as Exclude<ApiErrorCode, "IPC_FAILED">;
  }
  return null;
}
