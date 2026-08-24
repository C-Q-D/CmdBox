/** Execution Output 有界 Buffer 的字节裁剪与顺序测试。 */
import { describe, expect, it } from "vitest";
import {
  appendExecutionOutput,
  createPendingExecutionEventBuffer,
  createExecutionOutputBuffer,
  drainPendingExecutionEventBuffer,
  EXECUTION_OUTPUT_LIMIT_CHUNKS,
  queuePendingExecutionEvent,
} from "./execution-output-buffer";

describe("Execution Output Buffer", function describeOutputBuffer() {
  it("按事件和片段顺序追加纯文本", function appendOrderedFragments() {
    const result = appendExecutionOutput(
      createExecutionOutputBuffer(),
      4,
      [
        { fragmentSequence: 8, stream: "stdout", text: "A" },
        { fragmentSequence: 9, stream: "stderr", text: "B" },
      ],
      12,
      64,
    );

    expect(result.chunks.map((chunk) => chunk.key)).toEqual(["4:8", "4:9"]);
    expect(result.chunks.map((chunk) => chunk.text)).toEqual(["A", "B"]);
    expect(result.droppedBytes).toBe(12);
  });

  it("超过上限时只保留最新完整 Unicode 文本", function trimOldOutput() {
    const first = appendExecutionOutput(
      createExecutionOutputBuffer(),
      1,
      [{ fragmentSequence: 1, stream: "stdout", text: "older" }],
      0,
      8,
    );
    const result = appendExecutionOutput(
      first,
      2,
      [{ fragmentSequence: 2, stream: "stdout", text: "中间最新" }],
      0,
      8,
    );

    expect(result.totalBytes).toBeLessThanOrEqual(8);
    expect(result.chunks).toHaveLength(1);
    expect(result.chunks[0]?.text).toBe("最新");
    expect(result.droppedBytes).toBeGreaterThan(0);
  });

  it("忽略空片段并同时限制最多保留的非空 Chunk 数", function boundNonEmptyChunks() {
    const result = appendExecutionOutput(
      createExecutionOutputBuffer(),
      3,
      [
        { fragmentSequence: 1, stream: "stdout", text: "A" },
        { fragmentSequence: 2, stream: "stdout", text: "" },
        { fragmentSequence: 3, stream: "stderr", text: "B" },
        { fragmentSequence: 4, stream: "stdout", text: "C" },
      ],
      7,
      64,
      2,
    );

    expect(result.chunks.map((chunk) => chunk.text)).toEqual(["B", "C"]);
    expect(result.chunks).toHaveLength(2);
    expect(result.droppedBytes).toBe(8);
  });

  it("默认最多保留 2048 个最新非空 Chunk", function enforceDefaultChunkLimit() {
    /** 比默认上限多一个的最小纯文本 Fragment 集合。 */
    const fragments = Array.from(
      { length: EXECUTION_OUTPUT_LIMIT_CHUNKS + 1 },
      (_, fragmentSequence) => ({
        fragmentSequence,
        stream: "stdout" as const,
        text: "x",
      }),
    );
    const result = appendExecutionOutput(
      createExecutionOutputBuffer(),
      1,
      fragments,
      0,
    );

    expect(result.chunks).toHaveLength(EXECUTION_OUTPUT_LIMIT_CHUNKS);
    expect(result.chunks[0]?.fragmentSequence).toBe(1);
    expect(result.droppedBytes).toBe(1);
  });

  it("预响应 fragment 超限时优先淘汰 Output 并保留 Lifecycle", function boundPendingFragments() {
    const executionId = "9be8ec5d-ef8c-4c2a-a7f5-12069b2ad555";
    const pending = createPendingExecutionEventBuffer();
    queuePendingExecutionEvent(
      pending,
      { event: "started", data: { executionId, sequence: 0 } },
      { eventLimit: 4, fragmentLimit: 2, byteLimit: 64 },
    );
    queuePendingExecutionEvent(
      pending,
      {
        event: "output",
        data: {
          executionId,
          sequence: 1,
          fragments: [
            { fragmentSequence: 0, stream: "stdout", text: "A" },
            { fragmentSequence: 1, stream: "stdout", text: "B" },
            { fragmentSequence: 2, stream: "stderr", text: "C" },
          ],
          droppedBytesBefore: 5,
        },
      },
      { eventLimit: 4, fragmentLimit: 2, byteLimit: 64 },
    );
    queuePendingExecutionEvent(
      pending,
      {
        event: "finished",
        data: {
          executionId,
          sequence: 2,
          exitCode: 0,
          outcome: "success",
          durationMs: 10,
          droppedOutputBytes: 0,
        },
      },
      { eventLimit: 4, fragmentLimit: 2, byteLimit: 64 },
    );

    const drained = drainPendingExecutionEventBuffer(pending, executionId);

    expect(drained.events.map((event) => event.event)).toEqual([
      "started",
      "finished",
    ]);
    expect(drained.droppedOutputBytes).toBe(8);
    expect(pending.events).toHaveLength(0);
  });

  it("预响应事件数超限时保持固定上限", function boundPendingEvents() {
    const pending = createPendingExecutionEventBuffer();
    for (let sequence = 0; sequence < 3; sequence += 1) {
      queuePendingExecutionEvent(
        pending,
        {
          event: "started",
          data: { executionId: `execution-${sequence}`, sequence },
        },
        { eventLimit: 2, fragmentLimit: 2, byteLimit: 1024 },
      );
    }

    expect(pending.events).toHaveLength(2);
    expect(pending.events.map((event) => event.data.executionId)).toEqual([
      "execution-1",
      "execution-2",
    ]);
  });

  it("预响应缓存不保留空 Fragment 对象", function discardPendingEmptyFragments() {
    const pending = createPendingExecutionEventBuffer();
    queuePendingExecutionEvent(
      pending,
      {
        event: "output",
        data: {
          executionId: "execution-empty",
          sequence: 1,
          fragments: Array.from({ length: 4096 }, (_, fragmentSequence) => ({
            fragmentSequence,
            stream: "stdout" as const,
            text: "",
          })),
          droppedBytesBefore: 3,
        },
      },
      { eventLimit: 2, fragmentLimit: 2, byteLimit: 64 },
    );

    expect(pending.events).toHaveLength(1);
    expect(pending.fragmentCount).toBe(0);
    const [event] = pending.events;
    expect(event?.event).toBe("output");
    if (event?.event !== "output") {
      throw new Error("测试预期保留最小 Output sequence 事实");
    }
    expect(event.data.fragments).toHaveLength(0);
  });
});
