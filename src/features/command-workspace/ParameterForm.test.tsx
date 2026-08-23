/**
 * 统一 Parameter Form 的可访问控件、wire 值与目录选择行为测试。
 *
 * 测试只通过公开 React 属性和用户可见控件验证六类参数，不依赖 React Hook Form
 * 的内部状态；Rust 负责的路径、Shell 与最终参数校验不在这里复制。
 */
import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { useRef } from "react";
import { afterEach, describe, expect, it, vi } from "vitest";
import type {
  CommandBlockDetails,
  ParameterDefinition,
  ParameterValue,
} from "../../generated/contracts";
import type { FolderPicker } from "./folder-picker";
import {
  ParameterForm as ProductionParameterForm,
  type ParameterFormProps,
  type ParameterFormSnapshot,
} from "./ParameterForm";

/** 既有表单行为测试使用的默认双 generation 外壳。 */
type TestParameterFormProps = Omit<
  ParameterFormProps,
  | "definitionGeneration"
  | "configurationGeneration"
  | "externalFieldError"
  | "onConfigurationChange"
> &
  Partial<
    Pick<
      ParameterFormProps,
      | "definitionGeneration"
      | "configurationGeneration"
      | "externalFieldError"
      | "onConfigurationChange"
    >
  >;

/** 为每个测试表单提供独立的单调配置 generation。 */
function ParameterForm({
  definitionGeneration = 1,
  configurationGeneration = 1,
  externalFieldError = null,
  onConfigurationChange,
  ...props
}: TestParameterFormProps) {
  /** 当前测试表单的最新配置 generation。 */
  const generation = useRef(configurationGeneration);
  /** 默认在每次字段写入前递增 generation。 */
  function advanceGeneration(): number {
    generation.current += 1;
    return generation.current;
  }
  return (
    <ProductionParameterForm
      {...props}
      definitionGeneration={definitionGeneration}
      configurationGeneration={configurationGeneration}
      externalFieldError={externalFieldError}
      onConfigurationChange={onConfigurationChange ?? advanceGeneration}
    />
  );
}

/** 每个测试后卸载表单，确保异步 Picker 的 mounted 检查彼此隔离。 */
afterEach(function cleanupParameterForm() {
  cleanup();
});

/** 创建覆盖六类控件、默认值和约束的测试 Definition。 */
function createDefinition(overrides: Partial<CommandBlockDetails> = {}): CommandBlockDetails {
  return {
    id: "builtin.form-test",
    name: "类型化参数测试",
    description: "验证六类统一参数控件。",
    origin: "builtin",
    runner: "windowsPowerShell",
    riskLevel: "normal",
    revision: 1,
    parameters: [
      {
        type: "text",
        key: "text",
        label: "文本",
        description: "按 Unicode 字符计数",
        required: true,
        remember: false,
        defaultValue: null,
        minLength: 1,
        maxLength: 3,
        placeholder: "输入文本",
      },
      {
        type: "number",
        key: "count",
        label: "数字",
        description: "允许零值",
        required: false,
        remember: false,
        defaultValue: 2,
        min: 0,
        max: 10,
        step: 1,
      },
      {
        type: "boolean",
        key: "enabled",
        label: "启用条件输出",
        description: "明确提交布尔值",
        required: false,
        remember: false,
        defaultValue: false,
      },
      {
        type: "select",
        key: "mode",
        label: "模式",
        description: "只接受固定选项",
        required: false,
        remember: false,
        options: ["brief", "detailed"],
        defaultValue: null,
      },
      {
        type: "folder",
        key: "folder",
        label: "单个目录",
        description: "由 Rust 校验目录",
        required: false,
        remember: false,
        mustExist: true,
        defaultValue: null,
      },
      {
        type: "folders",
        key: "folders",
        label: "多个目录",
        description: "按选择顺序保留目录",
        required: false,
        remember: false,
        mustExist: true,
        minItems: 1,
        maxItems: 3,
        defaultValue: null,
      },
    ],
    ...overrides,
  };
}

/** 创建一个永不访问真实宿主的 Picker 替身。 */
function createPicker(): FolderPicker {
  return {
    /** 返回固定单目录。 */
    pickFolder: vi.fn(async () => "C:\\picked"),
    /** 返回固定多目录。 */
    pickFolders: vi.fn(async () => ["C:\\one", "C:\\two"]),
  };
}

/** 取得最近一次包含 generation 的完整公开快照。 */
function latestVersionedState(
  onStateChange: ReturnType<typeof vi.fn>,
): ParameterFormSnapshot {
  const calls = onStateChange.mock.calls;
  return calls[calls.length - 1]?.[0] as ParameterFormSnapshot;
}

/** 取得既有表单断言关注的值与 validity，不重复 generation 断言。 */
function latestState(
  onStateChange: ReturnType<typeof vi.fn>,
): Omit<ParameterFormSnapshot, "configurationGeneration"> {
  const { values, isValid } = latestVersionedState(onStateChange);
  return { values, isValid };
}

/** 取得最近一次表单回调的结构化 wire 记录。 */
function latestValues(onStateChange: ReturnType<typeof vi.fn>): Record<string, ParameterValue> {
  return latestState(onStateChange).values;
}

/** 创建一个可由测试精确解析的 Promise。 */
function createDeferred<T>() {
  /** 外部持有的解析器。 */
  let resolvePromise!: (value: T) => void;
  /** 当前挂起的 Promise。 */
  const promise = new Promise<T>((resolve) => {
    resolvePromise = resolve;
  });
  return { promise, resolve: resolvePromise };
}

describe("ParameterForm", function describeParameterForm() {
  it("按 Definition 顺序渲染六类可访问控件并复制非 null 默认值", async function renderSixControls() {
    const onStateChange = vi.fn();
    render(
      <ParameterForm
        definition={createDefinition()}
        disabled={false}
        folderPicker={createPicker()}
        onStateChange={onStateChange}
      />,
    );

    const form = screen.getByRole("form", { name: "类型化参数" });
    const fields = Array.from(form.querySelectorAll(".parameter-field"));
    expect(fields.map((field) => field.getAttribute("data-parameter-key"))).toEqual([
      "text",
      "count",
      "enabled",
      "mode",
      "folder",
      "folders",
    ]);
    expect(screen.getByRole("textbox", { name: /文本/ })).toHaveProperty("placeholder", "输入文本");
    expect(screen.getByRole("spinbutton", { name: /数字/ })).toHaveProperty("value", "2");
    expect(screen.getByRole("checkbox", { name: /启用条件输出/ })).toHaveProperty("checked", false);
    expect(screen.getByRole("combobox", { name: /模式/ })).toHaveProperty("value", "");
    expect(screen.getByRole("button", { name: "选择单个目录" })).toBeDefined();
    expect(screen.getByRole("button", { name: "添加多个目录" })).toBeDefined();
    expect(screen.getByText("按 Unicode 字符计数")).toBeDefined();
    expect(screen.getByText(/必填 · 1–3 个 Unicode 字符/)).toBeDefined();

    await waitFor(() => {
      expect(latestValues(onStateChange)).toEqual({ count: 2, enabled: false });
    });
  });

  it("把 Text、Number、Boolean 与 Select 中间态映射为精确 wire 语义", async function mapScalarWireValues() {
    const onStateChange = vi.fn();
    render(
      <ParameterForm
        definition={createDefinition()}
        disabled={false}
        folderPicker={createPicker()}
        onStateChange={onStateChange}
      />,
    );
    await waitFor(() => expect(latestValues(onStateChange)).toEqual({ count: 2, enabled: false }));

    const text = screen.getByRole("textbox", { name: /文本/ });
    fireEvent.change(text, { target: { value: "abc" } });
    await waitFor(() => expect(latestValues(onStateChange).text).toBe("abc"));
    fireEvent.change(text, { target: { value: "" } });
    await waitFor(() => expect(latestValues(onStateChange).text).toBe(""));

    const number = screen.getByRole("spinbutton", { name: /数字/ });
    fireEvent.change(number, { target: { value: "" } });
    await waitFor(() => expect(latestValues(onStateChange)).not.toHaveProperty("count"));
    fireEvent.change(number, { target: { value: "0" } });
    await waitFor(() => expect(latestValues(onStateChange).count).toBe(0));
    fireEvent.change(number, { target: { value: "1e309" } });
    await waitFor(() => {
      expect(latestValues(onStateChange)).not.toHaveProperty("count");
    });
    fireEvent.change(number, { target: { value: "4" } });
    await waitFor(() => expect(latestValues(onStateChange).count).toBe(4));

    fireEvent.click(screen.getByRole("checkbox", { name: /启用条件输出/ }));
    await waitFor(() => expect(latestValues(onStateChange).enabled).toBe(true));
    const select = screen.getByRole("combobox", { name: /模式/ });
    fireEvent.change(select, { target: { value: "detailed" } });
    await waitFor(() => expect(latestValues(onStateChange).mode).toBe("detailed"));
    fireEvent.change(select, { target: { value: "" } });
    await waitFor(() => expect(latestValues(onStateChange)).not.toHaveProperty("mode"));
  });

  it("按 Unicode scalar 而不是 UTF-16 code unit 校验 Text 长度", async function validateUnicodeScalars() {
    const definition = createDefinition();
    definition.parameters = [
      {
        type: "text",
        key: "emoji",
        label: "Emoji",
        description: null,
        required: true,
        remember: false,
        defaultValue: null,
        minLength: 1,
        maxLength: 1,
        placeholder: null,
      },
    ];
    render(
      <ParameterForm
        definition={definition}
        disabled={false}
        folderPicker={null}
        onStateChange={vi.fn()}
      />,
    );

    const input = screen.getByRole("textbox", { name: /Emoji/ });
    fireEvent.change(input, { target: { value: "😀" } });
    await waitFor(() => expect(screen.queryByRole("alert")).toBeNull());
    fireEvent.change(input, { target: { value: "😀😀" } });
    await waitFor(() => expect(screen.getByRole("alert")).toHaveProperty("textContent", "最多输入 1 个 Unicode 字符"));
  });

  it("选择单目录与多目录时原样提交，并按精确 index 删除重复路径", async function pickAndRemoveFolders() {
    const picker: FolderPicker = {
      /** 单选返回原始路径文本。 */
      pickFolder: vi.fn(async () => "C:\\picked folder"),
      /** 多选保留顺序和重复项。 */
      pickFolders: vi.fn(async () => ["C:\\same", "C:\\same", "C:\\other"]),
    };
    const onStateChange = vi.fn();
    render(
      <ParameterForm
        definition={createDefinition()}
        disabled={false}
        folderPicker={picker}
        onStateChange={onStateChange}
      />,
    );

    fireEvent.click(screen.getByRole("button", { name: "选择单个目录" }));
    await waitFor(() => expect(latestValues(onStateChange).folder).toBe("C:\\picked folder"));
    fireEvent.click(screen.getByRole("button", { name: "添加多个目录" }));
    await waitFor(() => expect(latestValues(onStateChange).folders).toEqual(["C:\\same", "C:\\same", "C:\\other"]));
    expect(screen.getAllByText("C:\\same")).toHaveLength(2);

    fireEvent.click(screen.getByRole("button", { name: "移除多个目录第 2 项" }));
    await waitFor(() => expect(latestValues(onStateChange).folders).toEqual(["C:\\same", "C:\\other"]));
  });

  it("允许通过可访问入口清空 Folder 默认值并按 optional 语义从 wire 省略", async function clearOptionalFolder() {
    const definition = createDefinition();
    definition.parameters = definition.parameters
      .filter((parameter) => parameter.type === "folder")
      .map((parameter) => ({
        ...parameter,
        required: false,
        defaultValue: "C:\\default-folder",
      }));
    const onStateChange = vi.fn();
    render(
      <ParameterForm
        definition={definition}
        disabled={false}
        folderPicker={null}
        onStateChange={onStateChange}
      />,
    );

    await waitFor(() => {
      expect(latestState(onStateChange)).toEqual({
        values: { folder: "C:\\default-folder" },
        isValid: true,
      });
    });
    fireEvent.click(screen.getByRole("button", { name: "清空单个目录" }));
    await waitFor(() => {
      expect(latestState(onStateChange)).toEqual({ values: {}, isValid: true });
    });
  });

  it("清空 required Folder 后省略 wire 值并暴露无效状态", async function clearRequiredFolder() {
    const definition = createDefinition();
    definition.parameters = definition.parameters
      .filter((parameter) => parameter.type === "folder")
      .map((parameter) => ({
        ...parameter,
        required: true,
        defaultValue: "C:\\required-folder",
      }));
    const onStateChange = vi.fn();
    render(
      <ParameterForm
        definition={definition}
        disabled={false}
        folderPicker={null}
        onStateChange={onStateChange}
      />,
    );

    await waitFor(() => expect(latestState(onStateChange).isValid).toBe(true));
    fireEvent.click(screen.getByRole("button", { name: "清空单个目录" }));
    await waitFor(() => {
      expect(latestState(onStateChange)).toEqual({ values: {}, isValid: false });
      expect(screen.getByRole("alert")).toHaveProperty(
        "textContent",
        "请选择一个目录",
      );
    });
  });

  it("把 required、Number 范围步长、Select 与 Folders 数量汇总为 isValid", async function exposeUxValidity() {
    const definition = createDefinition();
    definition.parameters = definition.parameters.map((parameter) => {
      if (parameter.type === "number") {
        return { ...parameter, required: true, defaultValue: null };
      }
      if (parameter.type === "select" || parameter.type === "folder") {
        return { ...parameter, required: true, defaultValue: null };
      }
      if (parameter.type === "folders") {
        return { ...parameter, required: true, minItems: 2, maxItems: 2 };
      }
      return parameter;
    });
    const folderSelections = [["Z:\\definitely-missing-a"], ["Z:\\definitely-missing-b"], ["Z:\\overflow"]];
    const picker: FolderPicker = {
      /** 返回不存在路径，证明前端不做路径存在性判断。 */
      pickFolder: vi.fn(async () => "Z:\\definitely-missing-folder"),
      /** 逐次返回目录项以验证 min/max。 */
      pickFolders: vi.fn(async () => folderSelections.shift() ?? null),
    };
    const onStateChange = vi.fn();
    render(
      <ParameterForm
        definition={definition}
        disabled={false}
        folderPicker={picker}
        onStateChange={onStateChange}
      />,
    );

    await waitFor(() => expect(latestState(onStateChange).isValid).toBe(false));
    fireEvent.change(screen.getByRole("textbox", { name: /文本/ }), { target: { value: "ok" } });
    const number = screen.getByRole("spinbutton", { name: /数字/ });
    fireEvent.change(number, { target: { value: "1e309" } });
    await waitFor(() => {
      // HTML number 控件会把非有限字面量清为空串；结果仍必须无效且不能进入 wire。
      expect(latestState(onStateChange).isValid).toBe(false);
      expect(latestValues(onStateChange)).not.toHaveProperty("count");
    });
    fireEvent.change(number, { target: { value: "-1" } });
    await waitFor(() => expect(screen.getByText("数字不得小于 0")).toBeDefined());
    fireEvent.change(number, { target: { value: "11" } });
    await waitFor(() => expect(screen.getByText("数字不得大于 10")).toBeDefined());
    fireEvent.change(number, { target: { value: "1.5" } });
    await waitFor(() => expect(screen.getByText("数字必须符合步长 1")).toBeDefined());
    fireEvent.change(number, { target: { value: "2" } });
    fireEvent.change(screen.getByRole("combobox", { name: /模式/ }), { target: { value: "brief" } });
    fireEvent.click(screen.getByRole("button", { name: "选择单个目录" }));
    await waitFor(() => expect(latestValues(onStateChange).folder).toBe("Z:\\definitely-missing-folder"));
    fireEvent.click(screen.getByRole("button", { name: "添加多个目录" }));
    await waitFor(() => {
      expect(latestValues(onStateChange).folders).toEqual(["Z:\\definitely-missing-a"]);
      expect(latestState(onStateChange).isValid).toBe(false);
    });
    fireEvent.click(screen.getByRole("button", { name: "添加多个目录" }));
    await waitFor(() => {
      expect(latestValues(onStateChange).folder).toBe("Z:\\definitely-missing-folder");
      expect(latestValues(onStateChange).folders).toEqual(["Z:\\definitely-missing-a", "Z:\\definitely-missing-b"]);
      expect(latestState(onStateChange).isValid).toBe(true);
    });

    fireEvent.click(screen.getByRole("button", { name: "添加多个目录" }));
    await waitFor(() => {
      expect(latestValues(onStateChange).folders).toEqual(["Z:\\definitely-missing-a", "Z:\\definitely-missing-b"]);
      expect(latestState(onStateChange).isValid).toBe(false);
      expect(screen.getByText("最多选择 2 个目录；当前选择未更改")).toBeDefined();
    });
  });

  it("为 Text、Number、Folder 与 Folders 错误提供稳定关系和 aria-invalid", async function exposeAccessibleErrors() {
    const definition = createDefinition();
    definition.parameters = definition.parameters.flatMap<ParameterDefinition>(
      (parameter) => {
        switch (parameter.type) {
          case "text":
            return [{ ...parameter, description: null, required: true, defaultValue: "x" }];
          case "number":
            return [{ ...parameter, description: null, required: true, defaultValue: 1 }];
          case "folder":
            return [{ ...parameter, description: null, required: true, defaultValue: "C:\\one" }];
          case "folders":
            return [{ ...parameter, description: null, required: true, defaultValue: ["C:\\one"] }];
          default:
            return [];
        }
      },
    );
    const onStateChange = vi.fn();
    render(
      <ParameterForm
        definition={definition}
        disabled={false}
        folderPicker={null}
        onStateChange={onStateChange}
      />,
    );

    await waitFor(() => expect(latestState(onStateChange).isValid).toBe(true));
    fireEvent.change(screen.getByRole("textbox", { name: "文本" }), {
      target: { value: "" },
    });
    fireEvent.change(screen.getByRole("spinbutton", { name: "数字" }), {
      target: { value: "" },
    });
    fireEvent.click(screen.getByRole("button", { name: "清空单个目录" }));
    fireEvent.click(
      screen.getByRole("button", { name: "移除多个目录第 1 项" }),
    );
    const controls = [
      screen.getByRole("textbox", { name: "文本" }),
      screen.getByRole("spinbutton", { name: "数字" }),
      screen.getByRole("textbox", { name: "单个目录" }),
      screen.getByRole("button", { name: "添加多个目录" }),
    ];
    await waitFor(() => {
      expect(controls.map((control) => control.getAttribute("aria-invalid"))).toEqual([
        "true",
        "true",
        "true",
        "true",
      ]);
    });
    for (const control of controls) {
      const describedBy = control.getAttribute("aria-describedby")?.split(" ") ?? [];
      expect(describedBy.some((id) => id.endsWith("-description"))).toBe(false);
      const errorId = describedBy.find((id) => id.endsWith("-error"));
      expect(errorId).toBeDefined();
      expect(document.getElementById(errorId ?? "")?.getAttribute("role")).toBe("alert");
    }
  });

  it("把可选 Text 与 Folders 的初始空值视为未提交，显式清空后才应用约束", async function validateOnlySubmittedOptionalValues() {
    const definition = createDefinition();
    definition.parameters = definition.parameters
      .filter((parameter) => parameter.type === "text" || parameter.type === "folders")
      .map((parameter) => ({ ...parameter, required: false }));
    const onStateChange = vi.fn();
    render(
      <ParameterForm
        definition={definition}
        disabled={false}
        folderPicker={null}
        onStateChange={onStateChange}
      />,
    );

    await waitFor(() => {
      expect(latestState(onStateChange)).toEqual({ values: {}, isValid: true });
    });
    const text = screen.getByRole("textbox", { name: /文本/ });
    fireEvent.change(text, { target: { value: "a" } });
    fireEvent.change(text, { target: { value: "" } });
    await waitFor(() => {
      expect(latestValues(onStateChange).text).toBe("");
      expect(latestState(onStateChange).isValid).toBe(false);
    });
  });

  it("把所有非 null 默认值防御复制到 wire，并把用户清空 Folders 表达为显式空数组", async function preserveExplicitDefaults() {
    const definition = createDefinition();
    definition.parameters = definition.parameters.map((parameter) => {
      switch (parameter.type) {
        case "text":
          return { ...parameter, required: false, defaultValue: "" };
        case "number":
          return { ...parameter, defaultValue: 0 };
        case "boolean":
          return { ...parameter, defaultValue: true };
        case "select":
          return { ...parameter, defaultValue: "brief" };
        case "folder":
          return { ...parameter, defaultValue: "C:\\default" };
        case "folders":
          return { ...parameter, defaultValue: ["C:\\same", "C:\\same"] };
      }
    });
    const foldersDefault = definition.parameters.find((parameter) => parameter.type === "folders")?.defaultValue;
    const onStateChange = vi.fn();
    render(
      <ParameterForm
        definition={definition}
        disabled={false}
        folderPicker={null}
        onStateChange={onStateChange}
      />,
    );

    await waitFor(() => {
      expect(latestValues(onStateChange)).toEqual({
        text: "",
        count: 0,
        enabled: true,
        mode: "brief",
        folder: "C:\\default",
        folders: ["C:\\same", "C:\\same"],
      });
      expect(latestValues(onStateChange).folders).not.toBe(foldersDefault);
    });
    fireEvent.click(screen.getByRole("button", { name: "移除多个目录第 2 项" }));
    fireEvent.click(screen.getByRole("button", { name: "移除多个目录第 1 项" }));
    await waitFor(() => expect(latestValues(onStateChange).folders).toEqual([]));
  });

  it("全表单只允许一个 Picker pending，并在响应时追加到等待期间的最新数组", async function appendToLatestFolders() {
    const definition = createDefinition();
    definition.parameters = definition.parameters.map((parameter) =>
      parameter.type === "folders"
        ? { ...parameter, defaultValue: ["C:\\keep", "C:\\remove"] }
        : parameter,
    );
    const pendingFolders = createDeferred<string[] | null>();
    const picker: FolderPicker = {
      /** 当前测试不应进入单目录选择。 */
      pickFolder: vi.fn(async () => "C:\\unexpected"),
      /** 挂起多目录结果，允许等待期间修改表单。 */
      pickFolders: vi.fn(() => pendingFolders.promise),
    };
    const onStateChange = vi.fn();
    render(
      <ParameterForm
        definition={definition}
        disabled={false}
        folderPicker={picker}
        onStateChange={onStateChange}
      />,
    );

    fireEvent.click(screen.getByRole("button", { name: "添加多个目录" }));
    expect((screen.getByRole("button", { name: "添加多个目录" }) as HTMLButtonElement).disabled).toBe(true);
    expect((screen.getByRole("button", { name: "选择单个目录" }) as HTMLButtonElement).disabled).toBe(true);
    fireEvent.click(screen.getByRole("button", { name: "移除多个目录第 2 项" }));
    expect(picker.pickFolders).toHaveBeenCalledOnce();

    await act(async () => pendingFolders.resolve(["C:\\new"]));
    await waitFor(() => expect(latestValues(onStateChange).folders).toEqual(["C:\\keep", "C:\\new"]));
  });

  it("Folders Picker 超限时保持初始 omitted 值且不把空数组标记为显式提交", async function preserveOmittedFoldersOnOverflow() {
    const definition = createDefinition();
    definition.parameters = definition.parameters
      .filter(
        (parameter) =>
          parameter.type === "boolean" || parameter.type === "folders",
      )
      .map((parameter) =>
        parameter.type === "folders"
          ? {
              ...parameter,
              required: false,
              minItems: 0,
              maxItems: 1,
              defaultValue: null,
            }
          : parameter,
      );
    const onStateChange = vi.fn();
    render(
      <ParameterForm
        definition={definition}
        disabled={false}
        folderPicker={{
          pickFolder: vi.fn(async () => null),
          pickFolders: vi.fn(async () => ["C:\\one", "C:\\two"]),
        }}
        onStateChange={onStateChange}
      />,
    );

    await waitFor(() => {
      expect(latestState(onStateChange)).toEqual({
        values: { enabled: false },
        isValid: true,
      });
    });
    fireEvent.click(screen.getByRole("button", { name: "添加多个目录" }));
    await waitFor(() => {
      expect(latestValues(onStateChange)).toEqual({ enabled: false });
      expect(screen.getByRole("alert")).toHaveProperty(
        "textContent",
        "最多选择 1 个目录；当前选择未更改",
      );
    });
    fireEvent.click(screen.getByRole("checkbox", { name: /启用条件输出/ }));
    await waitFor(() => {
      expect(latestValues(onStateChange)).toEqual({ enabled: true });
    });
  });

  it("Picker 取消不改变值，浏览器无 Picker 时入口保持禁用", async function ignorePickerCancellation() {
    const picker: FolderPicker = {
      /** 模拟单选取消。 */
      pickFolder: vi.fn(async () => null),
      /** 模拟多选取消。 */
      pickFolders: vi.fn(async () => null),
    };
    const onStateChange = vi.fn();
    const rendered = render(
      <ParameterForm
        definition={createDefinition()}
        disabled={false}
        folderPicker={picker}
        onStateChange={onStateChange}
      />,
    );
    await waitFor(() => expect(latestValues(onStateChange)).toEqual({ count: 2, enabled: false }));

    fireEvent.click(screen.getByRole("button", { name: "选择单个目录" }));
    await waitFor(() => expect(picker.pickFolder).toHaveBeenCalledOnce());
    fireEvent.click(screen.getByRole("button", { name: "添加多个目录" }));
    await waitFor(() => expect(picker.pickFolders).toHaveBeenCalledOnce());
    expect(latestValues(onStateChange)).toEqual({ count: 2, enabled: false });

    rendered.rerender(
      <ParameterForm
        definition={createDefinition()}
        disabled={false}
        folderPicker={null}
        onStateChange={onStateChange}
      />,
    );
    expect((screen.getByRole("button", { name: "选择单个目录" }) as HTMLButtonElement).disabled).toBe(true);
    expect((screen.getByRole("button", { name: "添加多个目录" }) as HTMLButtonElement).disabled).toBe(true);
  });

  it("Picker 取消保持 Folder 既有值、错误与 validity", async function preserveErrorOnPickerCancellation() {
    const definition = createDefinition();
    definition.parameters = definition.parameters
      .filter((parameter) => parameter.type === "folder")
      .map((parameter) => ({
        ...parameter,
        required: true,
        defaultValue: "C:\\existing",
      }));
    const onStateChange = vi.fn();
    const pickFolder = vi.fn(async () => null);
    render(
      <ParameterForm
        definition={definition}
        disabled={false}
        folderPicker={{
          pickFolder,
          pickFolders: vi.fn(async () => null),
        }}
        onStateChange={onStateChange}
      />,
    );

    await waitFor(() => expect(latestState(onStateChange).isValid).toBe(true));
    fireEvent.click(screen.getByRole("button", { name: "清空单个目录" }));
    const alert = await screen.findByRole("alert");
    const errorId = alert.id;
    expect(alert).toHaveProperty("textContent", "请选择一个目录");
    fireEvent.click(screen.getByRole("button", { name: "选择单个目录" }));
    await waitFor(() => expect(pickFolder).toHaveBeenCalledOnce());

    expect(screen.getByRole("textbox", { name: "单个目录" })).toHaveProperty(
      "value",
      "",
    );
    expect(latestState(onStateChange)).toEqual({ values: {}, isValid: false });
    expect(screen.getByRole("alert")).toHaveProperty("id", errorId);
    expect(screen.getByRole("alert")).toHaveProperty(
      "textContent",
      "请选择一个目录",
    );
  });

  it("仅 Definition identity 变化或卸载时丢弃 Picker 迟到结果", async function discardIdentityAndUnmountedPickerResults() {
    const pendingFolder = createDeferred<string | null>();
    const picker: FolderPicker = {
      /** 挂起单目录响应以制造 identity 与 lock 变化。 */
      pickFolder: vi.fn(() => pendingFolder.promise),
      /** 当前测试不使用多目录。 */
      pickFolders: vi.fn(async () => []),
    };
    const onStateChange = vi.fn();
    const definition = createDefinition();
    const rendered = render(
      <ParameterForm
        definition={definition}
        disabled={false}
        folderPicker={picker}
        onStateChange={onStateChange}
      />,
    );
    fireEvent.click(screen.getByRole("button", { name: "选择单个目录" }));

    const nextDefinition = createDefinition({ id: "builtin.form-next", revision: 2 });
    rendered.rerender(
      <ParameterForm
        definition={nextDefinition}
        disabled={false}
        folderPicker={picker}
        onStateChange={onStateChange}
      />,
    );
    await act(async () => pendingFolder.resolve("C:\\late"));
    expect(latestValues(onStateChange)).not.toHaveProperty("folder");
    expect((screen.getByRole("textbox", { name: /文本/ }) as HTMLInputElement).disabled).toBe(false);

    rendered.unmount();

    const pendingAfterUnmount = createDeferred<string | null>();
    const unmountedStateChange = vi.fn();
    const unmounted = render(
      <ParameterForm
        definition={definition}
        disabled={false}
        folderPicker={{
          pickFolder: vi.fn(() => pendingAfterUnmount.promise),
          pickFolders: vi.fn(async () => []),
        }}
        onStateChange={unmountedStateChange}
      />,
    );
    fireEvent.click(screen.getByRole("button", { name: "选择单个目录" }));
    const callsBeforeUnmount = unmountedStateChange.mock.calls.length;
    unmounted.unmount();
    await act(async () => pendingAfterUnmount.resolve("C:\\after-unmount"));
    expect(unmountedStateChange).toHaveBeenCalledTimes(callsBeforeUnmount);
  });

  it("仅 Execution lock 变化时丢弃 Picker 迟到结果", async function discardLockedPickerResult() {
    const pendingFolder = createDeferred<string | null>();
    const picker: FolderPicker = {
      pickFolder: vi.fn(() => pendingFolder.promise),
      pickFolders: vi.fn(async () => []),
    };
    const onStateChange = vi.fn();
    const definition = createDefinition();
    const rendered = render(
      <ParameterForm
        definition={definition}
        disabled={false}
        folderPicker={picker}
        onStateChange={onStateChange}
      />,
    );
    fireEvent.click(screen.getByRole("button", { name: "选择单个目录" }));
    rendered.rerender(
      <ParameterForm
        definition={definition}
        disabled={true}
        folderPicker={picker}
        onStateChange={onStateChange}
      />,
    );

    await act(async () => pendingFolder.resolve("C:\\locked-late"));
    expect(latestValues(onStateChange)).not.toHaveProperty("folder");
    expect((screen.getByRole("textbox", { name: /文本/ }) as HTMLInputElement).disabled).toBe(true);
  });

  it("连续两次输入只发布各自 render 捕获的 generation 与对应值", async function preserveRenderGenerationPairs() {
    const onStateChange = vi.fn();
    const onConfigurationChange = vi
      .fn<() => number>()
      .mockReturnValueOnce(2)
      .mockReturnValueOnce(3);
    render(
      <ParameterForm
        definition={createDefinition()}
        disabled={false}
        configurationGeneration={1}
        folderPicker={null}
        onConfigurationChange={onConfigurationChange}
        onStateChange={onStateChange}
      />,
    );
    await waitFor(() =>
      expect(latestVersionedState(onStateChange).configurationGeneration).toBe(
        1,
      ),
    );
    onStateChange.mockClear();

    const textInput = screen.getByRole("textbox", {
      name: /文本/,
    }) as HTMLInputElement;
    /** 原生 value setter 让两个 React input 事件共处同一 act 提交，effect 只能在其后运行。 */
    const setNativeValue = Object.getOwnPropertyDescriptor(
      HTMLInputElement.prototype,
      "value",
    )?.set;
    if (!setNativeValue) {
      throw new Error("测试环境缺少 HTMLInputElement.value setter");
    }
    act(() => {
      setNativeValue.call(textInput, "A");
      textInput.dispatchEvent(new Event("input", { bubbles: true }));
      setNativeValue.call(textInput, "AB");
      textInput.dispatchEvent(new Event("input", { bubbles: true }));
    });

    await waitFor(() => {
      expect(latestVersionedState(onStateChange)).toMatchObject({
        values: { text: "AB" },
        configurationGeneration: 3,
      });
    });
    expect(onConfigurationChange).toHaveBeenCalledTimes(2);
    /** 即使 React 为离散输入分别提交 effect，也不能组合新 generation 与旧 values。 */
    const deliveredSnapshots = onStateChange.mock.calls.map(
      ([snapshot]) => snapshot as ParameterFormSnapshot,
    );
    expect(
      deliveredSnapshots.some(
        (snapshot) =>
          snapshot.configurationGeneration === 2 &&
          snapshot.values.text !== "A",
      ),
    ).toBe(false);
    expect(
      deliveredSnapshots.some(
        (snapshot) =>
          snapshot.configurationGeneration === 3 &&
          snapshot.values.text !== "AB",
      ),
    ).toBe(false);
  });

  it("Picker 返回与当前值相同仍发布新的配置 generation", async function versionSamePickerValue() {
    const definition = createDefinition({
      parameters: [
        {
          type: "folder",
          key: "folder",
          label: "单个目录",
          description: "验证同值选择仍撤销旧 Preview",
          required: false,
          remember: false,
          mustExist: true,
          defaultValue: "C:\\same",
        },
      ],
    });
    const onStateChange = vi.fn();
    const onConfigurationChange = vi.fn<() => number>(() => 2);
    render(
      <ParameterForm
        definition={definition}
        disabled={false}
        configurationGeneration={1}
        folderPicker={{
          pickFolder: vi.fn(async () => "C:\\same"),
          pickFolders: vi.fn(async () => []),
        }}
        onConfigurationChange={onConfigurationChange}
        onStateChange={onStateChange}
      />,
    );
    await waitFor(() =>
      expect(latestVersionedState(onStateChange)).toMatchObject({
        values: { folder: "C:\\same" },
        configurationGeneration: 1,
      }),
    );
    onStateChange.mockClear();

    fireEvent.click(screen.getByRole("button", { name: "选择单个目录" }));

    await waitFor(() =>
      expect(latestVersionedState(onStateChange)).toEqual({
        values: { folder: "C:\\same" },
        isValid: true,
        configurationGeneration: 2,
      }),
    );
    expect(onConfigurationChange).toHaveBeenCalledOnce();
  });

  it("只呈现匹配双 generation 的 Rust 字段错误且不改写 RHF validity", async function renderVersionedExternalError() {
    const definition = createDefinition({
      parameters: [
        {
          type: "text",
          key: "text",
          label: "文本",
          description: "当前 Definition 的文本参数",
          required: true,
          remember: false,
          defaultValue: "valid",
          minLength: 1,
          maxLength: 16,
          placeholder: null,
        },
      ],
    });
    const onStateChange = vi.fn();
    const rendered = render(
      <ParameterForm
        definition={definition}
        disabled={false}
        definitionGeneration={5}
        configurationGeneration={7}
        externalFieldError={{
          definitionGeneration: 5,
          configurationGeneration: 7,
          parameterKey: "text",
          message: "请求参数未通过校验",
        }}
        folderPicker={null}
        onStateChange={onStateChange}
      />,
    );

    const input = screen.getByRole("textbox", { name: /文本/ });
    const externalError = await screen.findByText("请求参数未通过校验");
    expect(input.getAttribute("aria-invalid")).toBe("true");
    expect(input.getAttribute("aria-describedby")?.split(" ")).toContain(
      externalError.id,
    );
    await waitFor(() => expect(latestState(onStateChange).isValid).toBe(true));

    rendered.rerender(
      <ParameterForm
        definition={definition}
        disabled={false}
        definitionGeneration={5}
        configurationGeneration={7}
        externalFieldError={{
          definitionGeneration: 4,
          configurationGeneration: 7,
          parameterKey: "text",
          message: "旧 Definition 错误",
        }}
        folderPicker={null}
        onStateChange={onStateChange}
      />,
    );
    expect(screen.queryByText("旧 Definition 错误")).toBeNull();
    expect(input.getAttribute("aria-invalid")).toBe("false");
  });
});
