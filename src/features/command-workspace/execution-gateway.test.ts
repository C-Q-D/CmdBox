/** 固定 Execution Gateway 的命令、Channel 与错误契约测试。 */
import { describe, expect, it } from "vitest";
import {
  createFixedExecutionGateway,
  normalizeApiError,
  type ExecutionStreamEvent,
  type ExecutionTransport,
} from "./execution-gateway";

/** 创建可观察命令和 Channel 的无副作用 Transport。 */
function createTransport(available = true) {
  /** 最近一次创建的 Channel 回调。 */
  let channelHandler: ((event: ExecutionStreamEvent) => void) | undefined;
  /** Transport 收到的命令记录。 */
  const calls: Array<{
    /** Rust Command 名称。 */
    command: string;
    /** 传给 invoke 的结构化参数。 */
    arguments_: Record<string, unknown>;
  }> = [];
  /** 测试使用的最小 Transport。 */
  const transport: ExecutionTransport = {
    /** 返回测试指定的宿主可用性。 */
    isAvailable(): boolean {
      return available;
    },
    /** 保存 Channel 回调，并返回可识别的替身。 */
    createChannel<T>(onMessage: (message: T) => void): unknown {
      channelHandler = onMessage as (event: ExecutionStreamEvent) => void;
      return { testChannel: true };
    },
    /** 记录命令并返回与命令匹配的响应。 */
    async invoke<T>(
      command: string,
      arguments_: Record<string, unknown>,
    ): Promise<T> {
      calls.push({ command, arguments_ });
      const response =
        command === "start_fixed_execution"
          ? { executionId: "9be8ec5d-ef8c-4c2a-a7f5-12069b2ad555" }
          : { accepted: true, state: "cancelling" };
      return response as T;
    },
  };
  return {
    /** 测试 Transport。 */
    transport,
    /** 已记录的命令调用。 */
    calls,
    /** 返回最近一次 Channel 回调。 */
    getChannelHandler(): ((event: ExecutionStreamEvent) => void) | undefined {
      return channelHandler;
    },
  };
}

describe("固定 Execution Gateway", function describeGateway() {
  it("只用专属 Channel 调用固定启动命令", async function startFixedTask() {
    const fixture = createTransport();
    const gateway = createFixedExecutionGateway(fixture.transport);
    const observed: ExecutionStreamEvent[] = [];

    expect(gateway).not.toBeNull();
    if (!gateway) {
      throw new Error("可用 Transport 应创建 Gateway");
    }

    const response = await gateway.startFixedExecution((event) => {
      observed.push(event);
    });
    const startedEvent: ExecutionStreamEvent = {
      event: "started",
      data: { executionId: response.executionId, sequence: 0 },
    };
    fixture.getChannelHandler()?.(startedEvent);

    expect(response.executionId).toBe(
      "9be8ec5d-ef8c-4c2a-a7f5-12069b2ad555",
    );
    expect(fixture.calls).toHaveLength(1);
    expect(fixture.calls[0]?.command).toBe("start_fixed_execution");
    expect(fixture.calls[0]?.arguments_).toEqual({
      onEvent: { testChannel: true },
    });
    expect(observed).toEqual([startedEvent]);
  });

  it("只把 Execution ID 传给取消命令", async function cancelByExecutionId() {
    const fixture = createTransport();
    const gateway = createFixedExecutionGateway(fixture.transport);

    expect(gateway).not.toBeNull();
    if (!gateway) {
      throw new Error("可用 Transport 应创建 Gateway");
    }

    const response = await gateway.cancelExecution(
      "9be8ec5d-ef8c-4c2a-a7f5-12069b2ad555",
    );

    expect(response).toEqual({ accepted: true, state: "cancelling" });
    expect(fixture.calls).toEqual([
      {
        command: "cancel_execution",
        arguments_: {
          executionId: "9be8ec5d-ef8c-4c2a-a7f5-12069b2ad555",
        },
      },
    ]);
  });

  it("在纯浏览器环境不创建 Gateway 或 Channel", function rejectWebHost() {
    const fixture = createTransport(false);
    const gateway = createFixedExecutionGateway(fixture.transport);

    expect(gateway).toBeNull();
    expect(fixture.calls).toHaveLength(0);
    expect(fixture.getChannelHandler()).toBeUndefined();
  });

  it("保留结构化错误并收敛未知拒绝值", function normalizeErrors() {
    expect(
      normalizeApiError({ code: "PROCESS_START_FAILED", message: "无法启动" }),
    ).toEqual({
      code: "PROCESS_START_FAILED",
      message: "无法启动固定 PowerShell 任务",
    });
    expect(
      normalizeApiError({
        code: "UNEXPECTED",
        message: String.raw`C:\Users\Private\secret`,
      }),
    ).toEqual({
      code: "IPC_FAILED",
      message: "CmdBox 无法完成桌面宿主调用",
    });
    expect(normalizeApiError(new Error("private detail"))).toEqual({
      code: "IPC_FAILED",
      message: "CmdBox 无法完成桌面宿主调用",
    });
  });
});
