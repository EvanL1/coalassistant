/**
 * Node.js 服务入口.
 *
 * 监听 PORT (默认 8787), 处理:
 *   - /api/*  → Hono app (handlers in app.ts)
 *   - 其他    → serve dist/ 静态 (SPA fallback to index.html)
 *
 * 环境变量:
 *   PORT       监听端口 (默认 8787)
 *   DATA_DIR   SQLite 文件所在目录 (默认 ./data)
 *   AUTH_PASS  登录密码 (**必填**, 没设拒启动)
 *   STATIC_DIR 前端构建产物目录 (默认 ./dist, 没有就跳过静态)
 *
 * 部署 (VPS):
 *   AUTH_PASS=xxx DATA_DIR=/var/lib/doudou-blend node server/index.js
 *
 * 反向代理 + HTTPS 走 Caddy, 见 deploy/Caddyfile
 */

import { serve } from "@hono/node-server";
import { serveStatic } from "@hono/node-server/serve-static";
import { Hono } from "hono";
import path from "node:path";
import fs from "node:fs";

import { openDatabase } from "./db.js";
import { createApp } from "./app.js";

function getRequiredEnv(name: string): string {
  const v = process.env[name];
  if (!v || v.length === 0) {
    console.error(`致命错误: 环境变量 ${name} 必填, 未设, 拒启动.`);
    process.exit(1);
  }
  return v;
}

function main(): void {
  const port = Number(process.env.PORT ?? 8787);
  const dataDir = process.env.DATA_DIR ?? "./data";
  const authPass = getRequiredEnv("AUTH_PASS");
  const staticDir = process.env.STATIC_DIR ?? "./dist";

  fs.mkdirSync(dataDir, { recursive: true });
  const dbPath = path.join(dataDir, "data.db");
  const db = openDatabase(dbPath);

  console.log(`[doudou-blend] SQLite: ${dbPath}`);

  const apiApp = createApp({ db, authPass });

  // 顶层 app: /api/* 交给 apiApp, 其他走静态.
  const app = new Hono();
  app.route("/api", apiApp);

  if (fs.existsSync(staticDir)) {
    console.log(`[doudou-blend] 静态目录: ${staticDir}`);
    app.use("/*", serveStatic({ root: staticDir }));
    // SPA fallback: 找不到文件就返回 index.html
    app.notFound((c) => {
      const indexPath = path.join(staticDir, "index.html");
      if (fs.existsSync(indexPath)) {
        return c.html(fs.readFileSync(indexPath, "utf-8"));
      }
      return c.text("404", 404);
    });
  } else {
    console.warn(
      `[doudou-blend] 警告: ${staticDir} 不存在, 前端静态资源无法 serve. ` +
        `cd doudou_blend && npm run build 后再启服务, 或设 STATIC_DIR.`,
    );
  }

  serve({ fetch: app.fetch, port }, (info) => {
    console.log(`[doudou-blend] 启动成功: http://0.0.0.0:${info.port}`);
  });
}

main();
