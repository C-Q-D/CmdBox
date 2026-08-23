/**
 * Tauri 官方 Dialog Plugin 的可注入目录选择接缝。
 *
 * 本模块只请求目录 Open，不读取目录、不解析路径、不规范化文本，也不去重或重排多选结果。
 */
import { isTauri } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";

/** 目录选择允许传给官方 `open` 的完整且最小 options。 */
export interface FolderSelectionOptions {
  /** 只选择目录，不选择文件。 */
  directory: true;
  /** `false` 选择一个目录，`true` 选择多个目录。 */
  multiple: boolean;
  /** 不向插件申请递归目录读取。 */
  recursive: false;
}

/** FolderPicker 对桌面宿主和官方 Dialog `open` 的最小依赖。 */
export interface FolderPickerPlatform {
  /** 当前宿主是否支持 Tauri Dialog IPC。 */
  isAvailable(): boolean;
  /** 使用固定目录 options 打开官方选择对话框。 */
  open(options: FolderSelectionOptions): Promise<string | string[] | null>;
}

/** 参数表单可注入的单目录和多目录选择能力。 */
export interface FolderPicker {
  /** 选择一个目录；用户取消时返回 `null`。 */
  pickFolder(): Promise<string | null>;
  /** 选择多个目录；用户取消时返回 `null`。 */
  pickFolders(): Promise<string[] | null>;
}

/** 生产环境只通过官方宿主检测和 Dialog `open` 工作。 */
const tauriFolderPickerPlatform: FolderPickerPlatform = {
  /** 使用官方 Tauri 宿主检测。 */
  isAvailable(): boolean {
    return isTauri();
  },
  /** 不增加任何插件默认值或文件访问操作，直接调用官方 `open`。 */
  open(options: FolderSelectionOptions): Promise<string | string[] | null> {
    return open(options);
  },
};

/**
 * 创建目录选择器。
 *
 * @param platform 生产默认使用官方 Dialog；测试可注入无副作用替身。
 * @returns 桌面宿主中的窄 FolderPicker，纯浏览器环境返回 `null`。
 */
export function createFolderPicker(
  platform: FolderPickerPlatform = tauriFolderPickerPlatform,
): FolderPicker | null {
  if (!platform.isAvailable()) {
    return null;
  }
  return {
    /** 用官方单选形状请求一个目录，并原样返回路径文本。 */
    async pickFolder(): Promise<string | null> {
      const selected = await platform.open({
        directory: true,
        multiple: false,
        recursive: false,
      });
      if (selected === null || typeof selected === "string") {
        return selected;
      }
      throw new Error("目录单选返回了无效结果");
    },
    /** 用官方多选形状请求目录数组，并原样保留顺序和重复项。 */
    async pickFolders(): Promise<string[] | null> {
      const selected = await platform.open({
        directory: true,
        multiple: true,
        recursive: false,
      });
      if (
        selected === null ||
        (Array.isArray(selected) &&
          selected.every(function isPath(path): path is string {
            return typeof path === "string";
          }))
      ) {
        return selected;
      }
      throw new Error("目录多选返回了无效结果");
    },
  };
}
