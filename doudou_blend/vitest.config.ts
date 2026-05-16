import { cloudflareTest } from "@cloudflare/vitest-pool-workers";
import { defineConfig } from "vitest/config";

// Worker 集成测试: 真实 workerd runtime + miniflare 内存 D1.
// 每个测试通过 SELF.fetch() 打 worker 完整入口, 校验 status/body/auth.
export default defineConfig({
  plugins: [
    cloudflareTest({
      wrangler: { configPath: "./wrangler.toml" },
      miniflare: {
        // 测试用密码, 跟 secret AUTH_PASS 解耦, 避免依赖环境变量.
        bindings: {
          AUTH_PASS: "test-pass-123",
        },
      },
    }),
  ],
  test: {
    include: ["worker/**/*.test.ts"],
  },
});
