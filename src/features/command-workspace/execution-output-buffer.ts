/**
 * Execution Output 的独立有界 Chunk Buffer。
 *
 * Buffer 只保存最近的纯文本，不进入全局业务状态；达到上限时从最旧内容开始裁剪。
 */
import type {
  ExecutionOutputFragment,
  ExecutionStreamEvent,
} from "./execution-gateway";

/** 当前 UI 最多保留的 UTF-8 Output 字节数。 */
export const EXECUTION_OUTPUT_LIMIT_BYTES = 512 * 1024;

/** 当前 UI 最多保留的非空 Output Chunk 数。 */
export const EXECUTION_OUTPUT_LIMIT_CHUNKS = 2048;

/** Run 响应前最多缓存的 Channel 事件数。 */
export const PENDING_EXECUTION_EVENT_LIMIT = 2048;

/** Run 响应前最多缓存的非空 Output Fragment 数。 */
export const PENDING_EXECUTION_FRAGMENT_LIMIT = 2048;

/** 预响应淘汰账本最多跟踪的不同 Execution ID 数量。 */
const PENDING_DROPPED_EXECUTION_LIMIT = 64;

/** 一个可单独渲染的 Output Chunk。 */
export interface ExecutionOutputChunk extends ExecutionOutputFragment {
  /** React 列表使用的稳定事件内标识。 */
  key: string;
  /** 当前文本的 UTF-8 字节数。 */
  bytes: number;
}

/** 当前 Execution 的有界实时输出快照。 */
export interface ExecutionOutputBuffer {
  /** 按后端顺序保留的最近文本 Chunk。 */
  chunks: ExecutionOutputChunk[];
  /** 当前 Buffer 内文本的 UTF-8 字节数。 */
  totalBytes: number;
  /** Rust 队列和 UI Buffer 合计声明的丢弃字节数。 */
  droppedBytes: number;
}

/** 可由测试缩小的预响应事件、Fragment 与文本容量。 */
export interface PendingExecutionEventLimits {
  /** 最多保留的 Channel 事件数。 */
  readonly eventLimit: number;
  /** 最多保留的非空 Output Fragment 数。 */
  readonly fragmentLimit: number;
  /** 最多保留的 Output UTF-8 文本字节数。 */
  readonly byteLimit: number;
}

/** Run 响应建立 Execution ID 认证前的有界事件缓存。 */
export interface PendingExecutionEventBuffer {
  /** 按 Channel 到达顺序保留的事件。 */
  readonly events: ExecutionStreamEvent[];
  /** 当前事件携带的 Output UTF-8 文本字节数。 */
  totalBytes: number;
  /** 当前事件携带的非空 Output Fragment 数。 */
  fragmentCount: number;
  /** 按 Execution ID 隔离的已淘汰 Output 字节台账。 */
  readonly droppedOutputBytesByExecution: Map<string, number>;
}

/** 建立响应 ID 后一次性取出的事件与认证丢弃字节。 */
export interface DrainedPendingExecutionEvents {
  /** 仍需由 Workspace 按 ID、sequence 和 generation 认证的事件。 */
  readonly events: ExecutionStreamEvent[];
  /** 只属于响应 Execution ID 的预响应丢弃字节。 */
  readonly droppedOutputBytes: number;
}

/** 默认预响应缓存容量。 */
const DEFAULT_PENDING_LIMITS: PendingExecutionEventLimits = {
  eventLimit: PENDING_EXECUTION_EVENT_LIMIT,
  fragmentLimit: PENDING_EXECUTION_FRAGMENT_LIMIT,
  byteLimit: EXECUTION_OUTPUT_LIMIT_BYTES,
};

/** 创建空的 Output Buffer。 */
export function createExecutionOutputBuffer(): ExecutionOutputBuffer {
  return { chunks: [], totalBytes: 0, droppedBytes: 0 };
}

/** 创建一个不含待认证事件和淘汰台账的预响应缓存。 */
export function createPendingExecutionEventBuffer(): PendingExecutionEventBuffer {
  return {
    events: [],
    totalBytes: 0,
    fragmentCount: 0,
    droppedOutputBytesByExecution: new Map<string, number>(),
  };
}

/**
 * 追加一个 Rust Output Batch，并把结果限制在指定 UTF-8 字节数内。
 *
 * @param current 当前不可变 Buffer 快照。
 * @param eventSequence 当前 Output 事件的事件级顺序。
 * @param fragments Rust Coordinator 已排序的文本片段。
 * @param droppedBytesBefore Rust 在当前 Batch 之前声明的丢弃字节数。
 * @param limitBytes UI Buffer 上限。
 * @returns 新的有界 Buffer 快照。
 */
export function appendExecutionOutput(
  current: ExecutionOutputBuffer,
  eventSequence: number,
  fragments: readonly ExecutionOutputFragment[],
  droppedBytesBefore: number,
  limitBytes = EXECUTION_OUTPUT_LIMIT_BYTES,
  limitChunks = EXECUTION_OUTPUT_LIMIT_CHUNKS,
): ExecutionOutputBuffer {
  const appended = fragments
    .filter((fragment) => fragment.text.length > 0)
    .map((fragment) => {
      const text = keepNewestUtf8Bytes(fragment.text, limitBytes);
      return {
        ...fragment,
        text,
        key: `${eventSequence}:${fragment.fragmentSequence}`,
        bytes: utf8Bytes(text),
      };
    });
  const originalAppendedBytes = fragments.reduce(
    (total, fragment) => total + utf8Bytes(fragment.text),
    0,
  );
  const retainedAppendedBytes = appended.reduce(
    (total, chunk) => total + chunk.bytes,
    0,
  );
  const chunks = [...current.chunks, ...appended];
  let totalBytes = current.totalBytes + retainedAppendedBytes;
  let droppedBytes =
    current.droppedBytes +
    droppedBytesBefore +
    (originalAppendedBytes - retainedAppendedBytes);

  while (
    (totalBytes > limitBytes || chunks.length > limitChunks) &&
    chunks.length > 0
  ) {
    const removed = chunks.shift();
    if (removed) {
      totalBytes -= removed.bytes;
      droppedBytes += removed.bytes;
    }
  }

  return { chunks, totalBytes, droppedBytes };
}

/**
 * 追加一个尚未获得响应 ID 认证的 Channel 事件，并维持事件、Fragment 和字节三重上限。
 *
 * 超限时优先淘汰 Output，尽可能保留 Started 与唯一终态等 Lifecycle 事实；淘汰文本按
 * Execution ID 记账，响应返回后只把匹配 ID 的字节计入当前 UI。
 */
export function queuePendingExecutionEvent(
  current: PendingExecutionEventBuffer,
  event: ExecutionStreamEvent,
  limits: PendingExecutionEventLimits = DEFAULT_PENDING_LIMITS,
): void {
  /** 不保留空 Fragment 对象，但仍保留 Output 的 sequence 与 droppedBytesBefore 事实。 */
  const queuedEvent = compactPendingEvent(event);
  current.events.push(queuedEvent);
  current.totalBytes += pendingEventBytes(queuedEvent);
  current.fragmentCount += pendingFragmentCount(queuedEvent);
  while (
    current.events.length > limits.eventLimit ||
    current.fragmentCount > limits.fragmentLimit ||
    current.totalBytes > limits.byteLimit
  ) {
    const outputIndex = current.events.findIndex(
      (candidate) => candidate.event === "output",
    );
    const removalIndex = outputIndex >= 0 ? outputIndex : 0;
    const [removed] = current.events.splice(removalIndex, 1);
    if (!removed) {
      break;
    }
    current.totalBytes -= pendingEventBytes(removed);
    current.fragmentCount -= pendingFragmentCount(removed);
    recordPendingOutputDrop(current, removed);
  }
}

/** 为预响应缓存复制 Output 事件并移除全部空文本 Fragment。 */
function compactPendingEvent(event: ExecutionStreamEvent): ExecutionStreamEvent {
  if (event.event !== "output") {
    return event;
  }
  return {
    event: "output",
    data: {
      ...event.data,
      fragments: event.data.fragments.filter(
        (fragment) => fragment.text.length > 0,
      ),
    },
  };
}

/** 清空全部待认证事件与淘汰台账，供新 Run、拒绝和卸载失效旧缓存。 */
export function resetPendingExecutionEventBuffer(
  current: PendingExecutionEventBuffer,
): void {
  current.events.splice(0, current.events.length);
  current.totalBytes = 0;
  current.fragmentCount = 0;
  current.droppedOutputBytesByExecution.clear();
}

/**
 * 取得当前到达顺序的待认证事件，并只返回响应 Execution ID 的淘汰字节后清空缓存。
 */
export function drainPendingExecutionEventBuffer(
  current: PendingExecutionEventBuffer,
  executionId: string,
): DrainedPendingExecutionEvents {
  const drained = {
    events: [...current.events],
    droppedOutputBytes:
      current.droppedOutputBytesByExecution.get(executionId) ?? 0,
  };
  resetPendingExecutionEventBuffer(current);
  return drained;
}

/** 计算事件携带的 Output UTF-8 文本长度；Lifecycle 由独立事件数量上限约束。 */
function pendingEventBytes(event: ExecutionStreamEvent): number {
  return pendingOutputBytes(event);
}

/** 计算一个 Output 事件携带的 UTF-8 文本字节数。 */
function pendingOutputBytes(event: ExecutionStreamEvent): number {
  if (event.event !== "output") {
    return 0;
  }
  return event.data.fragments.reduce(
    (total, fragment) => total + utf8Bytes(fragment.text),
    0,
  );
}

/** 计算一个事件携带的非空 Output Fragment 数。 */
function pendingFragmentCount(event: ExecutionStreamEvent): number {
  if (event.event !== "output") {
    return 0;
  }
  return event.data.fragments.filter((fragment) => fragment.text.length > 0)
    .length;
}

/** 把淘汰 Output 的文本与 Rust 已报告丢弃字节记入有界 Execution 分桶。 */
function recordPendingOutputDrop(
  current: PendingExecutionEventBuffer,
  event: ExecutionStreamEvent,
): void {
  if (event.event !== "output") {
    return;
  }
  const { executionId, droppedBytesBefore } = event.data;
  const ledger = current.droppedOutputBytesByExecution;
  if (!ledger.has(executionId) && ledger.size >= PENDING_DROPPED_EXECUTION_LIMIT) {
    const oldestExecutionId = ledger.keys().next().value as string | undefined;
    if (oldestExecutionId) {
      ledger.delete(oldestExecutionId);
    }
  }
  ledger.set(
    executionId,
    (ledger.get(executionId) ?? 0) +
      pendingOutputBytes(event) +
      droppedBytesBefore,
  );
}

/** 返回文本的 UTF-8 字节数。 */
function utf8Bytes(text: string): number {
  return new TextEncoder().encode(text).byteLength;
}

/** 保留不超过上限的最新 Unicode Code Point，避免从中间截断代理对。 */
function keepNewestUtf8Bytes(text: string, limitBytes: number): string {
  if (utf8Bytes(text) <= limitBytes) {
    return text;
  }
  const codePoints = Array.from(text);
  let retainedBytes = 0;
  let start = codePoints.length;
  while (start > 0) {
    const nextBytes = utf8Bytes(codePoints[start - 1] ?? "");
    if (retainedBytes + nextBytes > limitBytes) {
      break;
    }
    retainedBytes += nextBytes;
    start -= 1;
  }
  return codePoints.slice(start).join("");
}
