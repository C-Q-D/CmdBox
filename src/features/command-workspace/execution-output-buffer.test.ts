/** Execution Output 有界 Buffer 的字节裁剪与顺序测试。 */
import { describe, expect, it } from "vitest";
import {
  appendExecutionOutput,
  createExecutionOutputBuffer,
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
});
