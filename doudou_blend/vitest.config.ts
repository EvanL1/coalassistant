import { defineConfig } from "vitest/config";

// Node 服务集成测试: in-memory SQLite + Hono app.request().
// 每个测试 new 一个 app 实例, 通过 app.request() 打 /api/*, 校验
// status/body/auth.
export default defineConfig({
  test: {
    include: ["server/**/*.test.ts"],
  },
});
