/** CmdBox 前端单元测试配置。 */
import { defineConfig } from "vitest/config";

/** 创建与 Vite 应用隔离、可在主机和容器复用的 jsdom 测试配置。 */
export default defineConfig({
  test: {
    environment: "jsdom",
    include: ["src/**/*.test.{ts,tsx}"],
  },
});
