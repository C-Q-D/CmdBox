/**
 * Command Workspace 的可信 Preview 快照模块。
 *
 * 本模块把可变 Parameter Form 数据转换为与 Gateway 请求相互隔离的只读快照，并在
 * Rust Preview 返回时校验 Command Block 身份。它不计算 Hash、不解释 Safety，也不
 * 重算 Hash；后续执行只能消费这里产生的 ConfirmedPreview 深复制请求。
 */
import type {
  CommandBlockDetails,
  ParameterValue,
  PreviewCommandRequest,
  PreviewCommandResponse,
  VerifyRunRequest,
} from "../../generated/contracts";

/** 递归只读类型，覆盖 Preview 中的参数数组、摘要数组和 Safety warnings。 */
export type DeepReadonly<T> = T extends (...arguments_: never[]) => unknown
  ? T
  : T extends Array<infer Item>
    ? ReadonlyArray<DeepReadonly<Item>>
    : T extends object
      ? { readonly [Key in keyof T]: DeepReadonly<T[Key]> }
      : T;

/** 一次 Preview 请求开始时冻结的身份与两份互不共享参数。 */
export interface PreviewAttempt {
  /** 当前 Command Block 的稳定 ID。 */
  readonly commandBlockId: string;
  /** 请求基于的 Definition revision。 */
  readonly revision: number;
  /** 请求基于的 Definition generation。 */
  readonly definitionGeneration: number;
  /** 请求基于的配置 generation。 */
  readonly configurationGeneration: number;
  /** 只交给 Gateway 的独立深复制请求。 */
  readonly request: PreviewCommandRequest;
  /** 只供后续 ConfirmedPreview 使用的另一份深复制只读参数。 */
  readonly confirmedParameterValues: Readonly<Record<string, ParameterValue>>;
}

/** 后续 Run 唯一允许消费的当前 Rust Preview 授权快照。 */
export interface ConfirmedPreview {
  /** Preview 对应的 Command Block ID。 */
  readonly commandBlockId: string;
  /** Preview 对应的 Definition revision。 */
  readonly revision: number;
  /** Preview 对应的 Definition generation。 */
  readonly definitionGeneration: number;
  /** Preview 对应的配置 generation。 */
  readonly configurationGeneration: number;
  /** 与 Gateway 请求互不共享的只读结构化参数。 */
  readonly parameterValues: Readonly<Record<string, ParameterValue>>;
  /** Rust 返回并经前端深复制冻结的完整 Preview 响应。 */
  readonly response: DeepReadonly<PreviewCommandResponse>;
  /** 覆盖完整 Canonical Execution Spec 的 Rust Hash。 */
  readonly executionSpecHash: string;
}

/** Preview 响应经身份验证后的三种稳定结果。 */
export type PreviewAcceptance =
  | {
      /** 当前响应可形成后续 Run 的唯一授权。 */
      readonly kind: "confirmed";
      /** 已完成深复制和冻结的可信 Preview。 */
      readonly confirmedPreview: ConfirmedPreview;
    }
  | {
      /** Rust Safety 明确阻止当前内容，因此只保留展示证据。 */
      readonly kind: "blocked";
      /** 已完成深复制和冻结的被阻止 Preview。 */
      readonly response: DeepReadonly<PreviewCommandResponse>;
    }
  | {
      /** 当前响应的 ID 或 revision 与请求身份不一致。 */
      readonly kind: "identityMismatch";
    };

/** 递归复制一个 Gateway 参数值，避免数组或对象跨状态共享。 */
function cloneParameterValue(value: ParameterValue): ParameterValue {
  if (Array.isArray(value)) {
    return value.map(cloneParameterValue);
  }
  if (typeof value === "object" && value !== null) {
    /** 当前对象值的独立深复制结果。 */
    const clone: Record<string, ParameterValue> = {};
    for (const [key, nestedValue] of Object.entries(value)) {
      clone[key] = cloneParameterValue(nestedValue);
    }
    return clone;
  }
  return value;
}

/** 深复制一个 Parameter record，并保留 key 的原始插入顺序。 */
function cloneParameterValues(
  values: Readonly<Record<string, ParameterValue>>,
): Record<string, ParameterValue> {
  /** 当前记录的独立深复制结果。 */
  const clone: Record<string, ParameterValue> = {};
  for (const [key, value] of Object.entries(values)) {
    clone[key] = cloneParameterValue(value);
  }
  return clone;
}

/** 递归冻结一个仅由 JSON 值构成的对象图，并返回同一个值。 */
function freezeDeep<T>(value: T): T {
  if (typeof value !== "object" || value === null || Object.isFrozen(value)) {
    return value;
  }
  for (const nestedValue of Object.values(value)) {
    freezeDeep(nestedValue);
  }
  return Object.freeze(value);
}

/** 深复制 Rust Preview 响应，不把 Gateway 返回对象直接放入 React 状态。 */
function clonePreviewResponse(
  response: PreviewCommandResponse,
): PreviewCommandResponse {
  return {
    ...response,
    parameterSummaries: response.parameterSummaries.map((summary) => ({
      ...summary,
      displayValues: [...summary.displayValues],
    })),
    safety: {
      ...response.safety,
      warnings: response.safety.warnings.map((warning) => ({ ...warning })),
    },
  };
}

/**
 * 从当前 Definition 与表单快照创建一次 Preview 尝试。
 *
 * 请求参数与确认参数分别深复制并冻结，Gateway 即使保留或改变它收到的对象，也不能
 * 改写后续 Run 所依赖的 ConfirmedPreview。
 */
export function createPreviewAttempt(
  definition: CommandBlockDetails,
  definitionGeneration: number,
  configurationGeneration: number,
  parameterValues: Readonly<Record<string, ParameterValue>>,
): PreviewAttempt {
  /** 只发送给 Gateway 的独立参数图。 */
  const requestValues = freezeDeep(cloneParameterValues(parameterValues));
  /** 只保存给 ConfirmedPreview 的独立参数图。 */
  const confirmedParameterValues = freezeDeep(
    cloneParameterValues(parameterValues),
  );
  /** 精确符合 Rust 生成契约的 Preview 请求。 */
  const request = freezeDeep({
    commandBlockId: definition.id,
    expectedRevision: definition.revision,
    parameterValues: requestValues,
  }) as PreviewCommandRequest;
  return {
    commandBlockId: definition.id,
    revision: definition.revision,
    definitionGeneration,
    configurationGeneration,
    request,
    confirmedParameterValues,
  };
}

/**
 * 校验并接纳一次 Rust Preview 响应。
 *
 * blocked 响应只能用于展示；其余 Safety 状态形成不可变 ConfirmedPreview。函数从不
 * 计算或改写 Rust Hash，也不根据展示文本推导执行内容。
 */
export function acceptPreviewResponse(
  attempt: PreviewAttempt,
  response: PreviewCommandResponse,
): PreviewAcceptance {
  if (
    response.commandBlockId !== attempt.commandBlockId ||
    response.revision !== attempt.revision
  ) {
    return { kind: "identityMismatch" };
  }
  /** 与 Gateway 返回对象隔离的只读展示响应。 */
  const responseSnapshot = freezeDeep(
    clonePreviewResponse(response),
  ) as DeepReadonly<PreviewCommandResponse>;
  if (responseSnapshot.safety.state === "blocked") {
    return { kind: "blocked", response: responseSnapshot };
  }
  return {
    kind: "confirmed",
    confirmedPreview: Object.freeze({
      commandBlockId: attempt.commandBlockId,
      revision: attempt.revision,
      definitionGeneration: attempt.definitionGeneration,
      configurationGeneration: attempt.configurationGeneration,
      parameterValues: attempt.confirmedParameterValues,
      response: responseSnapshot,
      executionSpecHash: responseSnapshot.executionSpecHash,
    }),
  };
}

/**
 * 从一次仍获授权的 ConfirmedPreview 创建唯一允许发送的通用 Run 请求。
 *
 * 参数在消费时再次深复制并冻结，确保 Gateway 不能通过保留请求引用改写确认快照；
 * Command Block 身份、revision 与 Hash 则严格原样沿用 Rust 已确认的字段。
 */
export function createVerifyRunRequest(
  confirmedPreview: ConfirmedPreview,
): VerifyRunRequest {
  /** 只交给本次 Gateway Run 调用的独立参数图。 */
  const parameterValues = freezeDeep(
    cloneParameterValues(confirmedPreview.parameterValues),
  );
  return freezeDeep({
    commandBlockId: confirmedPreview.commandBlockId,
    expectedRevision: confirmedPreview.revision,
    parameterValues,
    executionSpecHash: confirmedPreview.executionSpecHash,
  });
}
