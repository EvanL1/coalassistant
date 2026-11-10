import { defineConfig } from "vitest/config";

export default defineConfig({
  test: {
    coverage: {
      provider: "v8",
      include: ["src/**/*.{ts,tsx}"],
      exclude: ["src/**/*.test.{ts,tsx}", "src/**/*.d.ts"],
      // 全局门槛比实测 (语句 80.9 / 分支 75.9 / 函数 79.1 / 行 82.8) 低约 3 个点, 只防明显回退;
      // 纯逻辑的 domain 层维持 80%.
      thresholds: {
        statements: 78,
        branches: 73,
        functions: 76,
        lines: 80,
        "src/domain/**": {
          statements: 80,
          branches: 80,
          functions: 80,
          lines: 80,
        },
      },
    },
  },
});
