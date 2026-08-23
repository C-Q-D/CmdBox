/**
 * Command Workspace 的统一六类 Parameter Form。
 *
 * 本模块用 React Hook Form 管理用户交互，以 Zod 提供即时 UX 校验，并把中间表单值
 * 映射为 Rust Gateway 接受的结构化 wire 记录。它不判断路径是否存在、不规范化路径，
 * 也不实现任何 Shell 校验；这些可信规则始终留在 Rust Core。
 */
import { Controller, useForm, useWatch } from "react-hook-form";
import { useEffect, useId, useRef, useState } from "react";
import { z } from "zod";
import type {
  CommandBlockDetails,
  ParameterDefinition,
  ParameterValue,
} from "../../generated/contracts";
import type { FolderPicker } from "./folder-picker";

/** RHF 保存的未提交表单中间态；Number 在这里保留空串和原始输入。 */
type ParameterFormState = Record<string, boolean | string | string[]>;

/** 统一 Parameter Form 的小型外部 Interface。 */
export interface ParameterFormProps {
  /** 当前后端 Definition；调用方按 id/revision/generation 重建组件。 */
  definition: CommandBlockDetails;
  /** Execution 活跃时锁定全部字段、选择器和移除动作。 */
  disabled: boolean;
  /** 当前 Definition 请求的独立 generation，用于拒绝跨定义字段错误。 */
  definitionGeneration: number;
  /** 当前 Definition 初始绑定的配置 generation。 */
  configurationGeneration: number;
  /** 当前 Rust Preview 返回且仍匹配双 generation 的字段错误。 */
  externalFieldError: ExternalFieldError | null;
  /** 原生目录选择 Adapter；纯浏览器环境为 `null`。 */
  folderPicker: FolderPicker | null;
  /** 在任一真实字段写入前同步使旧 Preview 失效，并返回新的 generation。 */
  onConfigurationChange(): number;
  /** 每次 wire 记录或 UX 有效性变化时返回新的防御性快照。 */
  onStateChange(state: ParameterFormSnapshot): void;
}

/** Rust 字段错误绑定到当前 Definition 与配置快照的最小可访问模型。 */
export interface ExternalFieldError {
  /** 错误所属的 Definition 请求 generation。 */
  definitionGeneration: number;
  /** 错误所属的用户配置 generation。 */
  configurationGeneration: number;
  /** 对应当前 Parameter Definition 的稳定 key。 */
  parameterKey: string;
  /** Gateway 已收敛且可安全显示的错误说明。 */
  message: string;
}

/** 下一个 Preview 原子可稳定消费的表单快照。 */
export interface ParameterFormSnapshot {
  /** 按冻结六类语义生成的结构化 wire 值。 */
  values: Record<string, ParameterValue>;
  /** 只表示当前 Definition 的即时 UX 约束是否通过。 */
  isValid: boolean;
  /** 生成此快照的真实配置 generation。 */
  configurationGeneration: number;
}

/** 从 Definition 的明确非 null 默认值创建 RHF 中间态。 */
function createDefaultFormState(
  parameters: ParameterDefinition[],
): ParameterFormState {
  const defaults: ParameterFormState = {};
  for (const parameter of parameters) {
    switch (parameter.type) {
      case "text":
      case "select":
      case "folder":
        defaults[parameter.key] = parameter.defaultValue ?? "";
        break;
      case "number":
        defaults[parameter.key] =
          parameter.defaultValue === null ? "" : String(parameter.defaultValue);
        break;
      case "boolean":
        defaults[parameter.key] = parameter.defaultValue;
        break;
      case "folders":
        defaults[parameter.key] = parameter.defaultValue
          ? [...parameter.defaultValue]
          : [];
        break;
    }
  }
  return defaults;
}

/** 把 RHF 中间态按冻结语义映射为 Gateway wire 记录。 */
function createWireValues(
  parameters: ParameterDefinition[],
  state: ParameterFormState,
  touchedKeys: ReadonlySet<string>,
): Record<string, ParameterValue> {
  const values: Record<string, ParameterValue> = {};
  for (const parameter of parameters) {
    const value = state[parameter.key];
    switch (parameter.type) {
      case "text": {
        const text = typeof value === "string" ? value : "";
        if (
          text !== "" ||
          parameter.defaultValue !== null ||
          touchedKeys.has(parameter.key)
        ) {
          values[parameter.key] = text;
        }
        break;
      }
      case "number": {
        const raw = typeof value === "string" ? value : "";
        if (raw !== "") {
          const number = Number(raw);
          if (Number.isFinite(number)) {
            values[parameter.key] = number;
          }
        }
        break;
      }
      case "boolean":
        values[parameter.key] = value === true;
        break;
      case "select":
      case "folder":
        if (typeof value === "string" && value !== "") {
          values[parameter.key] = value;
        }
        break;
      case "folders": {
        const folders = Array.isArray(value) ? value : [];
        if (
          folders.length > 0 ||
          parameter.defaultValue !== null ||
          touchedKeys.has(parameter.key)
        ) {
          values[parameter.key] = [...folders];
        }
        break;
      }
    }
  }
  return values;
}

/** 使用 Unicode scalar 数量而不是 UTF-16 code unit 数量计算 Text 长度。 */
function scalarLength(value: string): number {
  return Array.from(value).length;
}

/** 用 Zod 验证一个字段的即时 UX 约束，并返回 RHF 接受的结果。 */
function validateParameterUx(
  parameter: ParameterDefinition,
  value: boolean | string | string[],
  explicitlySubmitted: boolean,
): true | string {
  let schema: z.ZodType;
  switch (parameter.type) {
    case "text":
      schema = z.string().superRefine((text, context) => {
        const length = scalarLength(text);
        if (parameter.required && length === 0) {
          context.addIssue({ code: "custom", message: "请输入必填文本" });
        }
        if (!explicitlySubmitted && length === 0) {
          return;
        }
        if (parameter.minLength !== null && length < parameter.minLength) {
          context.addIssue({ code: "custom", message: `至少输入 ${parameter.minLength} 个 Unicode 字符` });
        }
        if (parameter.maxLength !== null && length > parameter.maxLength) {
          context.addIssue({ code: "custom", message: `最多输入 ${parameter.maxLength} 个 Unicode 字符` });
        }
      });
      break;
    case "number":
      schema = z.string().superRefine((raw, context) => {
        if (raw === "") {
          if (parameter.required) {
            context.addIssue({ code: "custom", message: "请输入必填数字" });
          }
          return;
        }
        const number = Number(raw);
        if (!Number.isFinite(number)) {
          context.addIssue({ code: "custom", message: "请输入有限数字" });
          return;
        }
        if (parameter.min !== null && number < parameter.min) {
          context.addIssue({ code: "custom", message: `数字不得小于 ${parameter.min}` });
        }
        if (parameter.max !== null && number > parameter.max) {
          context.addIssue({ code: "custom", message: `数字不得大于 ${parameter.max}` });
        }
        if (parameter.step !== null) {
          const origin = parameter.min ?? 0;
          const quotient = (number - origin) / parameter.step;
          const tolerance = Number.EPSILON * Math.max(1, Math.abs(quotient)) * 8;
          if (Math.abs(quotient - Math.round(quotient)) > tolerance) {
            context.addIssue({ code: "custom", message: `数字必须符合步长 ${parameter.step}` });
          }
        }
      });
      break;
    case "boolean":
      schema = z.boolean();
      break;
    case "select":
      schema = z.string().superRefine((selected, context) => {
        if (parameter.required && selected === "") {
          context.addIssue({ code: "custom", message: "请选择一个选项" });
        } else if (selected !== "" && !parameter.options.includes(selected)) {
          context.addIssue({ code: "custom", message: "请选择 Definition 提供的选项" });
        }
      });
      break;
    case "folder":
      schema = z.string().superRefine((folder, context) => {
        if (parameter.required && folder === "") {
          context.addIssue({ code: "custom", message: "请选择一个目录" });
        }
      });
      break;
    case "folders":
      schema = z.array(z.string()).superRefine((folders, context) => {
        if (parameter.required && folders.length === 0) {
          context.addIssue({ code: "custom", message: "请至少选择一个目录" });
        }
        if (!explicitlySubmitted && folders.length === 0) {
          return;
        }
        if (parameter.minItems !== null && folders.length < parameter.minItems) {
          context.addIssue({ code: "custom", message: `至少选择 ${parameter.minItems} 个目录` });
        }
        if (parameter.maxItems !== null && folders.length > parameter.maxItems) {
          context.addIssue({ code: "custom", message: `最多选择 ${parameter.maxItems} 个目录` });
        }
      });
      break;
  }
  const result = schema.safeParse(value);
  return result.success ? true : result.error.issues[0]?.message ?? "参数格式无效";
}

/** 返回字段下方可访问的 required 与类型约束摘要。 */
function constraintLabel(parameter: ParameterDefinition): string {
  const constraints = [parameter.required ? "必填" : "可选"];
  switch (parameter.type) {
    case "text":
      if (parameter.minLength !== null && parameter.maxLength !== null) {
        constraints.push(`${parameter.minLength}–${parameter.maxLength} 个 Unicode 字符`);
      } else if (parameter.minLength !== null) {
        constraints.push(`至少 ${parameter.minLength} 个 Unicode 字符`);
      } else if (parameter.maxLength !== null) {
        constraints.push(`最多 ${parameter.maxLength} 个 Unicode 字符`);
      }
      break;
    case "number":
      if (parameter.min !== null) constraints.push(`最小 ${parameter.min}`);
      if (parameter.max !== null) constraints.push(`最大 ${parameter.max}`);
      if (parameter.step !== null) constraints.push(`步长 ${parameter.step}`);
      break;
    case "boolean":
      constraints.push("明确提交 true / false");
      break;
    case "select":
      constraints.push(`${parameter.options.length} 个固定选项`);
      break;
    case "folder":
      constraints.push(parameter.mustExist ? "Rust 将校验目录存在" : "目录文本");
      break;
    case "folders":
      if (parameter.minItems !== null) constraints.push(`至少 ${parameter.minItems} 项`);
      if (parameter.maxItems !== null) constraints.push(`最多 ${parameter.maxItems} 项`);
      constraints.push(parameter.mustExist ? "Rust 将校验目录存在" : "目录文本列表");
      break;
  }
  return constraints.join(" · ");
}

/** 渲染由当前 Definition 驱动的唯一六类 Parameter Form。 */
export function ParameterForm({
  definition,
  disabled,
  definitionGeneration,
  configurationGeneration,
  externalFieldError,
  folderPicker,
  onConfigurationChange,
  onStateChange,
}: ParameterFormProps) {
  /** 只用于当前挂载实例的 DOM id 前缀。 */
  const idPrefix = useId();
  /** 记录用户实际触达的 key，以区分初始空值和显式清空。 */
  const touchedKeys = useRef(new Set<string>());
  /** 当前组件是否仍挂载，阻止 Dialog 迟到结果写回已卸载表单。 */
  const mounted = useRef(false);
  /** 始终指向最新 Execution lock，异步 Picker 不依赖旧 render 闭包。 */
  const disabledSnapshot = useRef(disabled);
  /** 当前 Definition 身份，异步结果必须与请求时完全一致。 */
  const definitionIdentity = `${definition.id}:${definition.revision}:${definitionGeneration}`;
  /** 当前 Definition 身份的最新快照。 */
  const definitionIdentitySnapshot = useRef(definitionIdentity);
  /** 单调 Picker token 与当前全表单唯一 pending 请求。 */
  const pickerRequest = useRef<{ token: number; key: string } | null>(null);
  /** 为每次 Picker 请求分配当前实例内唯一的单调 token。 */
  const pickerRequestSequence = useRef(0);
  /** 用于禁用其他 Picker 入口并呈现等待状态的字段 key。 */
  const [pendingPickerKey, setPendingPickerKey] = useState<string | null>(null);
  /** 当前 render 对应的配置 generation，保证 effect 不读取可变最新值。 */
  const [snapshotGeneration, setSnapshotGeneration] = useState(
    configurationGeneration,
  );
  /** RHF 是当前表单交互状态的唯一所有者。 */
  const { clearErrors, control, formState, getValues, setError, setValue } = useForm<ParameterFormState>({
    defaultValues: createDefaultFormState(definition.parameters),
    mode: "onChange",
  });
  /** 订阅公开字段值，用于生成结构化 wire 快照。 */
  const watchedValues = useWatch({ control }) as ParameterFormState;

  disabledSnapshot.current = disabled;
  definitionIdentitySnapshot.current = definitionIdentity;

  /** 建立 mounted 身份；卸载时同时使当前 Picker token 失效。 */
  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
      pickerRequest.current = null;
    };
  }, []);

  /** 在初始默认值或任一字段变化后，把防御性 wire 快照交给 Workspace。 */
  useEffect(() => {
    onStateChange({
      values: createWireValues(
        definition.parameters,
        watchedValues,
        touchedKeys.current,
      ),
      isValid: formState.isValid,
      configurationGeneration: snapshotGeneration,
    });
  }, [
    definition.parameters,
    formState.isValid,
    onStateChange,
    snapshotGeneration,
    watchedValues,
  ]);

  /** 在写入 RHF 前取得新 generation，使旧 Preview 与字段错误同步失效。 */
  function beginConfigurationWrite(): void {
    const nextGeneration = onConfigurationChange();
    setSnapshotGeneration(nextGeneration);
  }

  /** 判断一个异步 Picker 响应是否仍属于当前 Definition、字段、token 和 lock。 */
  function canAcceptPickerResult(
    token: number,
    parameter: ParameterDefinition,
    requestIdentity: string,
  ): boolean {
    return (
      mounted.current &&
      !disabledSnapshot.current &&
      definitionIdentitySnapshot.current === requestIdentity &&
      pickerRequest.current?.token === token &&
      pickerRequest.current.key === parameter.key &&
      definition.parameters.some(
        (candidate) =>
          candidate.key === parameter.key && candidate.type === parameter.type,
      )
    );
  }

  /** 请求单目录或多目录，并只把仍可信的当前响应写入 RHF。 */
  async function pickParameter(parameter: ParameterDefinition) {
    if (
      !folderPicker ||
      disabledSnapshot.current ||
      pickerRequest.current ||
      (parameter.type !== "folder" && parameter.type !== "folders")
    ) {
      return;
    }
    const token = pickerRequestSequence.current + 1;
    pickerRequestSequence.current = token;
    const requestIdentity = definitionIdentitySnapshot.current;
    pickerRequest.current = { token, key: parameter.key };
    setPendingPickerKey(parameter.key);
    try {
      const selected =
        parameter.type === "folder"
          ? await folderPicker.pickFolder()
          : await folderPicker.pickFolders();
      if (!canAcceptPickerResult(token, parameter, requestIdentity) || selected === null) {
        return;
      }
      if (parameter.type === "folder") {
        beginConfigurationWrite();
        touchedKeys.current.add(parameter.key);
        setValue(parameter.key, selected as string, {
          shouldDirty: true,
          shouldValidate: true,
        });
        return;
      }
      // Dialog 打开期间用户仍可移除旧项，因此必须在响应时读取 RHF 的最新数组。
      const current = getValues(parameter.key);
      const currentFolders = Array.isArray(current) ? current : [];
      const nextFolders = [...currentFolders, ...(selected as string[])];
      if (
        parameter.maxItems !== null &&
        nextFolders.length > parameter.maxItems
      ) {
        setError(parameter.key, {
          type: "maxItems",
          message: `最多选择 ${parameter.maxItems} 个目录；当前选择未更改`,
        });
        return;
      }
      beginConfigurationWrite();
      touchedKeys.current.add(parameter.key);
      setValue(parameter.key, nextFolders, {
        shouldDirty: true,
        shouldValidate: true,
      });
    } catch {
      if (canAcceptPickerResult(token, parameter, requestIdentity)) {
        setError(parameter.key, {
          type: "picker",
          message: "目录选择未完成，请重试",
        });
      }
    } finally {
      if (mounted.current && pickerRequest.current?.token === token) {
        pickerRequest.current = null;
        setPendingPickerKey(null);
      }
    }
  }

  /** 按当前数组的精确 index 删除一项，保留其他重复路径的位置和顺序。 */
  function removeFolder(parameterKey: string, index: number) {
    if (disabledSnapshot.current) {
      return;
    }
    const current = getValues(parameterKey);
    const currentFolders = Array.isArray(current) ? current : [];
    if (index < 0 || index >= currentFolders.length) {
      return;
    }
    beginConfigurationWrite();
    touchedKeys.current.add(parameterKey);
    clearErrors(parameterKey);
    setValue(
      parameterKey,
      currentFolders.filter((_, currentIndex) => currentIndex !== index),
      { shouldDirty: true, shouldValidate: true },
    );
  }

  return (
    <form className="parameter-form" aria-label="类型化参数" onSubmit={(event) => event.preventDefault()}>
      {definition.parameters.map((parameter) => {
        const fieldId = `${idPrefix}-${parameter.key}`;
        const descriptionId = `${fieldId}-description`;
        const constraintId = `${fieldId}-constraints`;
        const errorId = `${fieldId}-error`;
        return (
          <div className="parameter-field" data-parameter-key={parameter.key} key={parameter.key}>
            <div className="parameter-field__heading">
              <label htmlFor={fieldId}>{parameter.label}</label>
              <span className="parameter-field__type">{parameter.type}</span>
            </div>
            {parameter.description ? <p id={descriptionId} className="parameter-field__description">{parameter.description}</p> : null}
            <Controller
              control={control}
              name={parameter.key}
              rules={{
                /** 仅提供即时 UX；提交后仍由 Rust 使用当前 Definition 最终校验。 */
                validate(value) {
                  const explicitlySubmitted =
                    parameter.type === "text"
                      ? value !== "" ||
                        parameter.defaultValue !== null ||
                        touchedKeys.current.has(parameter.key)
                      : parameter.type === "folders"
                        ? (Array.isArray(value) && value.length > 0) ||
                          parameter.defaultValue !== null ||
                          touchedKeys.current.has(parameter.key)
                        : true;
                  return validateParameterUx(
                    parameter,
                    value,
                    explicitlySubmitted,
                  );
                },
              }}
              render={({ field, fieldState }) => {
                /** 只接受同时匹配当前 Definition、配置 generation 与字段 key 的 Rust 错误。 */
                const currentExternalError =
                  externalFieldError?.definitionGeneration ===
                    definitionGeneration &&
                  externalFieldError.configurationGeneration ===
                    snapshotGeneration &&
                  externalFieldError.parameterKey === parameter.key
                    ? externalFieldError.message
                    : null;
                /** 本地即时校验和 Rust 字段错误共享同一个稳定可访问错误节点。 */
                const currentError = [
                  fieldState.error?.message,
                  currentExternalError,
                ]
                  .filter((message): message is string => Boolean(message))
                  .join("；");
                /** 只关联当前实际存在的说明、约束和错误节点。 */
                const describedBy = [
                  parameter.description ? descriptionId : null,
                  constraintId,
                  currentError ? errorId : null,
                ]
                  .filter((id): id is string => id !== null)
                  .join(" ");
                /** 标记本字段由用户触达，再把值交回 RHF。 */
                function changeValue(value: boolean | string | string[]) {
                  beginConfigurationWrite();
                  touchedKeys.current.add(parameter.key);
                  field.onChange(value);
                }
                let controlElement;
                switch (parameter.type) {
                  case "text":
                    controlElement = <input id={fieldId} type="text" value={field.value as string} placeholder={parameter.placeholder ?? undefined} aria-describedby={describedBy} aria-invalid={Boolean(currentError)} disabled={disabled} onBlur={field.onBlur} onChange={(event) => changeValue(event.currentTarget.value)} />;
                    break;
                  case "number":
                    controlElement = <input id={fieldId} type="number" value={field.value as string} min={parameter.min ?? undefined} max={parameter.max ?? undefined} step={parameter.step ?? "any"} aria-describedby={describedBy} aria-invalid={Boolean(currentError)} disabled={disabled} onBlur={field.onBlur} onChange={(event) => changeValue(event.currentTarget.value)} />;
                    break;
                  case "boolean":
                    controlElement = <input id={fieldId} type="checkbox" checked={field.value === true} aria-describedby={describedBy} aria-invalid={Boolean(currentError)} disabled={disabled} onBlur={field.onBlur} onChange={(event) => changeValue(event.currentTarget.checked)} />;
                    break;
                  case "select":
                    controlElement = <select id={fieldId} value={field.value as string} aria-describedby={describedBy} aria-invalid={Boolean(currentError)} disabled={disabled} onBlur={field.onBlur} onChange={(event) => changeValue(event.currentTarget.value)}><option value="">请选择…</option>{parameter.options.map((option) => <option key={option} value={option}>{option}</option>)}</select>;
                    break;
                  case "folder":
                    controlElement = <div className="parameter-picker parameter-picker--folder"><input id={fieldId} type="text" readOnly value={field.value as string} aria-describedby={describedBy} aria-invalid={Boolean(currentError)} disabled={disabled} /><button type="button" disabled={disabled || !folderPicker || pendingPickerKey !== null} aria-label={`选择${parameter.label}`} onClick={() => void pickParameter(parameter)}>{pendingPickerKey === parameter.key ? "正在选择…" : "选择目录"}</button><button type="button" disabled={disabled || field.value === ""} aria-label={`清空${parameter.label}`} onClick={() => changeValue("")}>清空</button></div>;
                    break;
                  case "folders":
                    controlElement = <div className="parameter-picker parameter-picker--multiple"><button id={fieldId} type="button" disabled={disabled || !folderPicker || pendingPickerKey !== null} aria-describedby={describedBy} aria-invalid={Boolean(currentError)} aria-label={`添加${parameter.label}`} onClick={() => void pickParameter(parameter)}>{pendingPickerKey === parameter.key ? "正在选择…" : "添加目录"}</button><ol className="parameter-folder-list" aria-label={`${parameter.label}已选值`}>{(field.value as string[]).map((folder, index) => <li key={`${index}-${folder}`}><code>{folder}</code><button type="button" disabled={disabled} aria-label={`移除${parameter.label}第 ${index + 1} 项`} onClick={() => removeFolder(parameter.key, index)}>移除</button></li>)}</ol></div>;
                    break;
                }
                return <>{controlElement}<p id={constraintId} className="parameter-field__constraints">{constraintLabel(parameter)}</p>{currentError ? <p id={errorId} className="parameter-field__error" role="alert">{currentError}</p> : null}</>;
              }}
            />
          </div>
        );
      })}
    </form>
  );
}
