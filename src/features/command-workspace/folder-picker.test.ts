/** 原生目录选择接缝的单选、多选、取消与浏览器降级测试。 */
import { describe, expect, it } from "vitest";
import {
  createFolderPicker,
  type FolderPickerPlatform,
  type FolderSelectionOptions,
} from "./folder-picker";

/** 创建记录精确 Dialog options 的无副作用平台替身。 */
function createPlatform(
  results: Array<string | string[] | null>,
  available = true,
) {
  /** 官方 open 形状收到的完整 options。 */
  const calls: FolderSelectionOptions[] = [];
  /** 测试可控制返回结果的平台。 */
  const platform: FolderPickerPlatform = {
    /** 返回测试指定的宿主可用性。 */
    isAvailable(): boolean {
      return available;
    },
    /** 原样记录 options 并依次返回预设结果。 */
    async open(
      options: FolderSelectionOptions,
    ): Promise<string | string[] | null> {
      calls.push(options);
      return results.shift() ?? null;
    },
  };
  return { platform, calls };
}

describe("FolderPicker", function describeFolderPicker() {
  it("单选只请求一个目录并原样返回路径", async function pickOneFolder() {
    const rawPath = String.raw`F:\原始 目录\child\..\target`;
    const fixture = createPlatform([rawPath]);
    const picker = createFolderPicker(fixture.platform);

    expect(picker).not.toBeNull();
    if (!picker) {
      throw new Error("可用桌面平台应创建目录选择器");
    }

    await expect(picker.pickFolder()).resolves.toBe(rawPath);
    expect(fixture.calls).toEqual([
      { directory: true, multiple: false, recursive: false },
    ]);
  });

  it("多选保留顺序、重复项和原始文本", async function pickManyFolders() {
    const rawPaths = [
      String.raw`F:\原始 目录\one`,
      String.raw`F:\原始 目录\one`,
      String.raw`F:\原始 目录\child\..\two`,
    ];
    const fixture = createPlatform([rawPaths]);
    const picker = createFolderPicker(fixture.platform);
    if (!picker) {
      throw new Error("可用桌面平台应创建目录选择器");
    }

    await expect(picker.pickFolders()).resolves.toEqual(rawPaths);
    expect(fixture.calls).toEqual([
      { directory: true, multiple: true, recursive: false },
    ]);
  });

  it("多选拒绝包含非字符串项的畸形数组", async function rejectMalformedManySelection() {
    const malformed = [String.raw`F:\valid`, 7] as unknown as string[];
    const fixture = createPlatform([malformed]);
    const picker = createFolderPicker(fixture.platform);
    if (!picker) {
      throw new Error("可用桌面平台应创建目录选择器");
    }

    await expect(picker.pickFolders()).rejects.toThrow(
      "目录多选返回了无效结果",
    );
  });

  it("单选或多选取消都返回 null", async function cancelSelection() {
    const fixture = createPlatform([null, null]);
    const picker = createFolderPicker(fixture.platform);
    if (!picker) {
      throw new Error("可用桌面平台应创建目录选择器");
    }

    await expect(picker.pickFolder()).resolves.toBeNull();
    await expect(picker.pickFolders()).resolves.toBeNull();
  });

  it("纯浏览器环境返回 null 且不调用 Dialog", function rejectWebHost() {
    const fixture = createPlatform([String.raw`F:\must-not-be-used`], false);

    expect(createFolderPicker()).toBeNull();
    expect(createFolderPicker(fixture.platform)).toBeNull();
    expect(fixture.calls).toHaveLength(0);
  });
});
