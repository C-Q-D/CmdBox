/** Command Block Execution Gateway 的窄命令、Channel 与错误契约测试。 */
import { readdirSync, readFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";
import type {
  ExecutionStreamEvent,
  PreviewCommandRequest,
  VerifyRunRequest,
} from "../../generated/contracts";
import {
  createCommandExecutionGateway,
  createFixedExecutionGateway,
  normalizeApiError,
  PUBLISHED_API_ERROR_CODES,
  type ExecutionTransport,
} from "./execution-gateway";

/** Preview 测试使用的最小结构化请求。 */
const previewRequest: PreviewCommandRequest = {
  commandBlockId: "builtin.parameter-echo.windows-powershell",
  expectedRevision: 1,
  parameterValues: {
    text: "安全回显",
    count: 1,
    enabled: true,
    mode: "brief",
    folder: String.raw`C:\CmdBox-Test`,
    folders: [String.raw`C:\CmdBox-Test`],
  },
};

/** Run 测试使用的已确认 Preview 请求。 */
const runRequest: VerifyRunRequest = {
  ...previewRequest,
  executionSpecHash: "a".repeat(64),
};

/** 创建可观察命令、参数和 Channel 的无副作用 Transport。 */
function createTransport(available = true) {
  /** Transport 收到的完整调用记录。 */
  const calls: Array<{
    /** Rust Command 名称。 */
    command: string;
    /** 传给 invoke 的精确结构化参数。 */
    arguments_: Record<string, unknown>;
  }> = [];
  /** 已创建的 Channel 回调。 */
  const channelHandlers: Array<(event: ExecutionStreamEvent) => void> = [];
  /** 测试使用的最小 Transport。 */
  const transport: ExecutionTransport = {
    /** 返回测试指定的宿主可用性。 */
    isAvailable(): boolean {
      return available;
    },
    /** 保存 Channel 回调，并返回可识别的替身。 */
    createChannel<T>(onMessage: (message: T) => void): unknown {
      channelHandlers.push(
        onMessage as (event: ExecutionStreamEvent) => void,
      );
      return { testChannel: channelHandlers.length };
    },
    /** 记录命令并返回与五个公开命令匹配的最小响应。 */
    async invoke<T>(
      command: string,
      arguments_: Record<string, unknown>,
    ): Promise<T> {
      calls.push({ command, arguments_ });
      const responses: Record<string, unknown> = {
        list_command_blocks: [],
        get_command_block: {
          id: "builtin.parameter-echo.windows-powershell",
          name: "PowerShell 参数回显",
          description: "安全回显",
          origin: "builtin",
          runner: "windowsPowerShell",
          riskLevel: "normal",
          revision: 1,
          parameters: [],
        },
        preview_command_block: {
          commandBlockId: previewRequest.commandBlockId,
          revision: 1,
          runner: "windowsPowerShell",
          parameterSummaries: [],
          previewText: "Write-Output '安全回显'",
          fullSizeBytes: 32,
          truncated: false,
          riskLevel: "normal",
          actionLabel: "执行回显",
          safety: { state: "notApplicable", summary: null, warnings: [] },
          executionSpecHash: runRequest.executionSpecHash,
        },
        run_command_block: {
          executionId: "9be8ec5d-ef8c-4c2a-a7f5-12069b2ad555",
        },
        cancel_execution: { accepted: true, state: "cancelling" },
      };
      return responses[command] as T;
    },
  };
  return {
    /** 测试 Transport。 */
    transport,
    /** 已记录的命令调用。 */
    calls,
    /** 已创建的 Channel 回调。 */
    channelHandlers,
  };
}

describe("Command Block Execution Gateway", function describeGateway() {
  it("精确调用五个公开命令且只有 Run 创建 Channel", async function exactCommands() {
    const fixture = createTransport();
    const gateway = createCommandExecutionGateway(fixture.transport);
    const observed: ExecutionStreamEvent[] = [];

    expect(gateway).not.toBeNull();
    if (!gateway) {
      throw new Error("可用 Transport 应创建 Gateway");
    }

    await gateway.listCommandBlocks();
    await gateway.getCommandBlock(previewRequest.commandBlockId);
    await gateway.previewCommandBlock(previewRequest);
    await gateway.runCommandBlock(runRequest, function observeEvent(event) {
      observed.push(event);
    });
    await gateway.cancelExecution("9be8ec5d-ef8c-4c2a-a7f5-12069b2ad555");

    expect(fixture.calls).toEqual([
      { command: "list_command_blocks", arguments_: {} },
      {
        command: "get_command_block",
        arguments_: { commandBlockId: previewRequest.commandBlockId },
      },
      {
        command: "preview_command_block",
        arguments_: { request: previewRequest },
      },
      {
        command: "run_command_block",
        arguments_: { request: runRequest, onEvent: { testChannel: 1 } },
      },
      {
        command: "cancel_execution",
        arguments_: {
          executionId: "9be8ec5d-ef8c-4c2a-a7f5-12069b2ad555",
        },
      },
    ]);
    expect(fixture.channelHandlers).toHaveLength(1);

    const started: ExecutionStreamEvent = {
      event: "started",
      data: {
        executionId: "9be8ec5d-ef8c-4c2a-a7f5-12069b2ad555",
        sequence: 0,
      },
    };
    fixture.channelHandlers[0]?.(started);
    expect(observed).toEqual([started]);
  });

  it("请求参数没有脚本、可执行文件、选项、工作目录或 PID 旁路", async function noBypassFields() {
    const fixture = createTransport();
    const gateway = createCommandExecutionGateway(fixture.transport);
    if (!gateway) {
      throw new Error("可用 Transport 应创建 Gateway");
    }

    await gateway.previewCommandBlock(previewRequest);
    await gateway.runCommandBlock(runRequest, function ignoreEvent() {});
    const serialized = JSON.stringify(fixture.calls);

    for (const forbidden of [
      "script",
      "executable",
      "options",
      "workingDirectory",
      "environment",
      "pid",
    ]) {
      expect(serialized).not.toContain(`\"${forbidden}\"`);
    }
  });

  it("在纯浏览器环境不创建 Gateway 或 Channel", function rejectWebHost() {
    const fixture = createTransport(false);

    expect(createCommandExecutionGateway()).toBeNull();
    expect(createCommandExecutionGateway(fixture.transport)).toBeNull();
    expect(fixture.calls).toHaveLength(0);
    expect(fixture.channelHandlers).toHaveLength(0);
  });

  it("完整识别已发布错误码并只保留字符串定位字段", function normalizePublishedErrors() {
    for (const code of PUBLISHED_API_ERROR_CODES) {
      const normalized = normalizeApiError({
        code,
        message: String.raw`C:\Users\Private\secret`,
        parameterKey: "text",
        detailCode: "invalidType",
        privateObject: { token: "secret" },
      });

      expect(normalized.code).toBe(code);
      expect(normalized.message).not.toContain("Private");
      expect(normalized).toMatchObject({
        parameterKey: "text",
        detailCode: "invalidType",
      });
      expect(normalized).not.toHaveProperty("privateObject");
    }
  });

  it("未知或错误类型的拒绝值收敛且不回显内容", function normalizeUnknownErrors() {
    for (const error of [
      {
        code: "UNEXPECTED",
        message: String.raw`C:\Users\Private\secret`,
      },
      { code: "STALE_PREVIEW", parameterKey: { private: true } },
      new Error("private detail"),
      "private detail",
    ]) {
      expect(normalizeApiError(error)).toEqual({
        code: "IPC_FAILED",
        message: "CmdBox 无法完成桌面宿主调用",
      });
    }
  });

  it("旧 Factory 无参数固定返回 null 且生产前端不存在已删除命令", function removedLegacyCommand() {
    const fixture = createTransport();

    expect(createFixedExecutionGateway()).toBeNull();
    expect(createFixedExecutionGateway.length).toBe(0);
    expect(fixture.calls).toHaveLength(0);
    expect(readProductionFrontendSource()).not.toContain(
      ["start", "fixed", "execution"].join("_"),
    );
  });
});

/** 读取 `src/` 下除测试以外的前端生产 TypeScript 源码。 */
function readProductionFrontendSource(): string {
  const currentDirectory = dirname(fileURLToPath(import.meta.url));
  const sourceRoot = resolve(currentDirectory, "../..");
  return readProductionDirectory(sourceRoot).join("\n");
}

/** 递归读取生产 `.ts`/`.tsx` 文件，排除测试文件。 */
function readProductionDirectory(directory: string): string[] {
  const contents: string[] = [];
  for (const entry of readdirSync(directory, { withFileTypes: true })) {
    const path = join(directory, entry.name);
    if (entry.isDirectory()) {
      contents.push(...readProductionDirectory(path));
    } else if (
      /\.tsx?$/.test(entry.name) &&
      !/\.test\.tsx?$/.test(entry.name)
    ) {
      contents.push(readFileSync(path, "utf8"));
    }
  }
  return contents;
}
