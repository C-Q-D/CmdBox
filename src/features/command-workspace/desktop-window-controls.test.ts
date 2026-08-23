/** CmdBox 无装饰主窗口与最小 Capability 的静态契约测试。 */
import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

/** 读取相对当前测试文件的 JSON 配置。 */
function readJson(relativePath: string): unknown {
  return JSON.parse(readFileSync(new URL(relativePath, import.meta.url), "utf8"));
}

describe("CmdBox desktop window contract", function describeDesktopWindowContract() {
  it("主窗口关闭原生装饰并保留既有尺寸约束", function useCustomWindowChrome() {
    const config = readJson("../../../src-tauri/tauri.conf.json") as {
      app: { windows: Array<Record<string, unknown>> };
    };
    expect(config.app.windows[0]).toMatchObject({
      title: "CmdBox",
      minWidth: 800,
      minHeight: 560,
      decorations: false,
    });
  });

  it("主窗口只开放标题栏与目录 Open 所需权限", function keepWindowPermissionsMinimal() {
    const capability = readJson("../../../src-tauri/capabilities/default.json") as {
      windows: string[];
      permissions: string[];
    };
    expect(capability.windows).toEqual(["main"]);
    expect(capability.permissions).toEqual([
      "core:window:allow-minimize",
      "core:window:allow-toggle-maximize",
      "core:window:allow-internal-toggle-maximize",
      "core:window:allow-close",
      "core:window:allow-start-dragging",
      "dialog:allow-open",
    ]);
  });
});
