/**
 * Execution Output 的独立有界 Chunk Buffer。
 *
 * Buffer 只保存最近的纯文本，不进入全局业务状态；达到上限时从最旧内容开始裁剪。
 */
import type { ExecutionOutputFragment } from "./execution-gateway";

/** 当前 UI 最多保留的 UTF-8 Output 字节数。 */
export const EXECUTION_OUTPUT_LIMIT_BYTES = 512 * 1024;

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

/** 创建空的 Output Buffer。 */
export function createExecutionOutputBuffer(): ExecutionOutputBuffer {
  return { chunks: [], totalBytes: 0, droppedBytes: 0 };
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
): ExecutionOutputBuffer {
  const appended = fragments.map((fragment) => {
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

  while (totalBytes > limitBytes && chunks.length > 0) {
    const removed = chunks.shift();
    if (removed) {
      totalBytes -= removed.bytes;
      droppedBytes += removed.bytes;
    }
  }

  return { chunks, totalBytes, droppedBytes };
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
