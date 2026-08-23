/**
 * 可信 Preview 快照的深复制、冻结和身份接纳测试。
 *
 * 测试直接验证后续 Run 将消费的 ConfirmedPreview 不与表单、Gateway 请求或 Rust
 * 响应共享可变对象；不在前端重算 Hash，也不解释 Preview 文本。
 */
import { describe, expect, it } from "vitest";
import type {
  CommandBlockDetails,
  ParameterValue,
  PreviewCommandResponse,
} from "../../generated/contracts";
import {
  acceptPreviewResponse,
  createPreviewAttempt,
  createVerifyRunRequest,
} from "./preview-state";

/** 创建不含参数 Schema 的最小 Definition；快照测试直接传入完整 JSON 值。 */
function createDefinition(): CommandBlockDetails {
  return {
    id: "builtin.preview-state",
    name: "Preview State",
    description: "验证可信 Preview 快照",
    origin: "builtin",
    runner: "windowsPowerShell",
    riskLevel: "normal",
    revision: 7,
    parameters: [],
  };
}

/** 创建包含可变嵌套数组的完整 Rust Preview 响应。 */
function createResponse(
  overrides: Partial<PreviewCommandResponse> = {},
): PreviewCommandResponse {
  return {
    commandBlockId: "builtin.preview-state",
    revision: 7,
    runner: "windowsPowerShell",
    parameterSummaries: [
      {
        parameterKey: "nested",
        label: "嵌套值",
        displayValues: ["alpha", "beta"],
        totalCount: 2,
        truncated: false,
      },
    ],
    previewText: "Write-Output 'trusted'",
    fullSizeBytes: 27,
    truncated: false,
    riskLevel: "normal",
    actionLabel: "执行可信内容",
    safety: {
      state: "warning",
      summary: "当前存在提醒",
      warnings: [{ code: "NOTICE", message: "仅用于测试" }],
    },
    executionSpecHash: "b".repeat(64),
    ...overrides,
  };
}

describe("可信 Preview 快照", function describePreviewState() {
  it("把递归 ParameterValue 为请求与确认分别深复制并冻结", function isolateParameterGraphs() {
    /** 原始表单图包含数组、对象、null 和所有标量分支。 */
    const nestedArray: ParameterValue[] = [
      "alpha",
      null,
      true,
      3,
      { child: ["beta", { leaf: null }] },
    ];
    /** 原始可变参数记录。 */
    const source: Record<string, ParameterValue> = {
      nested: nestedArray,
      object: { enabled: false },
    };

    const attempt = createPreviewAttempt(createDefinition(), 11, 23, source);

    expect(attempt.request.parameterValues).not.toBe(source);
    expect(attempt.confirmedParameterValues).not.toBe(source);
    expect(attempt.request.parameterValues).not.toBe(
      attempt.confirmedParameterValues,
    );
    expect(attempt.request.parameterValues.nested).not.toBe(nestedArray);
    expect(attempt.confirmedParameterValues.nested).not.toBe(nestedArray);
    expect(attempt.request.parameterValues.nested).not.toBe(
      attempt.confirmedParameterValues.nested,
    );

    nestedArray.push("late source mutation");
    (source.object as Record<string, ParameterValue>).enabled = true;
    expect(attempt.request.parameterValues).toEqual({
      nested: ["alpha", null, true, 3, { child: ["beta", { leaf: null }] }],
      object: { enabled: false },
    });
    expect(attempt.confirmedParameterValues).toEqual(
      attempt.request.parameterValues,
    );
    expect(Object.isFrozen(attempt.request)).toBe(true);
    expect(Object.isFrozen(attempt.request.parameterValues)).toBe(true);
    expect(Object.isFrozen(attempt.request.parameterValues.nested)).toBe(true);
    expect(Object.isFrozen(attempt.confirmedParameterValues)).toBe(true);
  });

  it("把 Rust 响应另行深复制冻结且只保存完整 Rust Hash", function isolateResponseGraph() {
    const response = createResponse();
    const attempt = createPreviewAttempt(
      createDefinition(),
      11,
      23,
      { nested: [{ leaf: null }] },
    );

    const acceptance = acceptPreviewResponse(attempt, response);
    expect(acceptance.kind).toBe("confirmed");
    if (acceptance.kind !== "confirmed") {
      throw new Error("测试预期形成 ConfirmedPreview");
    }

    response.previewText = "mutated response";
    response.parameterSummaries[0].displayValues.push("late");
    response.safety.warnings[0].message = "late warning";
    expect(acceptance.confirmedPreview.response.previewText).toBe(
      "Write-Output 'trusted'",
    );
    expect(
      acceptance.confirmedPreview.response.parameterSummaries[0]
        .displayValues,
    ).toEqual(["alpha", "beta"]);
    expect(
      acceptance.confirmedPreview.response.safety.warnings[0].message,
    ).toBe("仅用于测试");
    expect(acceptance.confirmedPreview.executionSpecHash).toBe("b".repeat(64));
    expect(Object.isFrozen(acceptance.confirmedPreview)).toBe(true);
    expect(Object.isFrozen(acceptance.confirmedPreview.parameterValues)).toBe(
      true,
    );
    expect(Object.isFrozen(acceptance.confirmedPreview.response)).toBe(true);
    expect(
      Object.isFrozen(
        acceptance.confirmedPreview.response.parameterSummaries[0]
          .displayValues,
      ),
    ).toBe(true);
    expect(
      Object.isFrozen(
        acceptance.confirmedPreview.response.safety.warnings[0],
      ),
    ).toBe(true);
  });

  it("只从 ConfirmedPreview 再次深复制冻结通用 Run 请求", function isolateRunRequest() {
    const attempt = createPreviewAttempt(createDefinition(), 11, 23, {
      nested: ["alpha", { child: [null, true, 3] }],
    });
    const acceptance = acceptPreviewResponse(attempt, createResponse());
    if (acceptance.kind !== "confirmed") {
      throw new Error("测试预期形成 ConfirmedPreview");
    }

    const request = createVerifyRunRequest(acceptance.confirmedPreview);

    expect(request).toEqual({
      commandBlockId: "builtin.preview-state",
      expectedRevision: 7,
      parameterValues: {
        nested: ["alpha", { child: [null, true, 3] }],
      },
      executionSpecHash: "b".repeat(64),
    });
    expect(request.parameterValues).not.toBe(
      acceptance.confirmedPreview.parameterValues,
    );
    expect(request.parameterValues.nested).not.toBe(
      acceptance.confirmedPreview.parameterValues.nested,
    );
    expect(Object.isFrozen(request)).toBe(true);
    expect(Object.isFrozen(request.parameterValues)).toBe(true);
    expect(Object.isFrozen(request.parameterValues.nested)).toBe(true);
  });

  it("拒绝错误身份并让 blocked 只保留展示证据", function enforceIdentityAndBlockedSafety() {
    const attempt = createPreviewAttempt(createDefinition(), 11, 23, {});

    expect(
      acceptPreviewResponse(
        attempt,
        createResponse({ commandBlockId: "builtin.other" }),
      ),
    ).toEqual({ kind: "identityMismatch" });
    expect(
      acceptPreviewResponse(attempt, createResponse({ revision: 8 })),
    ).toEqual({ kind: "identityMismatch" });

    const blocked = acceptPreviewResponse(
      attempt,
      createResponse({
        riskLevel: "destructive",
        safety: {
          state: "blocked",
          summary: "Rust 已拦截",
          warnings: [{ code: "BLOCKED", message: "不能执行" }],
        },
      }),
    );
    expect(blocked.kind).toBe("blocked");
    if (blocked.kind !== "blocked") {
      throw new Error("测试预期 blocked 只保留展示响应");
    }
    expect(blocked.response.safety.summary).toBe("Rust 已拦截");
    expect(Object.isFrozen(blocked.response)).toBe(true);
    expect("confirmedPreview" in blocked).toBe(false);
  });
});
