import { defineRailway, github, preserve, project, service } from "railway/iac";

// 本仓库只管自己的服务; 其他仓库各自导出自己的 partial.
export const partial = "coalassistant";

// 从 railway.json 迁移而来 (Config as Code 于 2026-12-01 停用).
//
// 这是声明式配置: 文件里没写的东西, apply 时会被当成要删除。所以
// - 代码来源必须写明 github 仓库, 否则会断开自动部署;
// - 环境变量必须逐个声明, 机密一律用 preserve() 保留线上现值, 不写进仓库。
// `railway config migrate` 生成的版本这两样都没写, 还漏了重启策略, 不要用它覆盖本文件。
// 改完先跑 `railway config plan`, 确认 0 to destroy 再 apply。
export default defineRailway(() => {
  const web = service("web", {
    source: github("EvanL1/coalassistant"),
    build: {
      builder: "DOCKERFILE",
      dockerfilePath: "Dockerfile",
    },
    deploy: {
      healthcheckPath: "/api/health",
      healthcheckTimeout: 300,
      // 重启策略取 Railway 默认的 ON_FAILURE (失败时重启)。不写 restartPolicyType:
      // 默认值在线上存为空, 显式写上会让 plan 每次都报一条 null → ON_FAILURE 的假差异.
      restartPolicyMaxRetries: 3,
    },
    env: {
      AUTH_USERNAME: preserve(),
      AUTH_PASSWORD: preserve(),
      AUTH_SESSION_TOKEN: preserve(),
      DATABASE_URL: preserve(),
      SUPABASE_SINGAPORE_DATABASE_URL: preserve(),
      SUPABASE_TOKYO_DATABASE_URL: preserve(),
    },
  });
  return project("coalassistant", {
    resources: [web],
  });
});
