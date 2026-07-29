# Railway 部署

生产部署使用根目录 `Dockerfile`：Node 阶段构建 React，Rust 阶段编译
`blend_kit_server`，最终镜像只包含原生服务和静态资源。

服务必须配置以下变量：

- `AUTH_USERNAME`：Web 登录账号。
- `AUTH_PASSWORD`：Web 登录密码。
- `AUTH_SESSION_TOKEN`：至少 32 字节的随机会话令牌。
- `DATABASE_URL`：引用同项目 PostgreSQL 的私网连接串，生产环境建议配置为
  `${{Postgres.DATABASE_URL}}`。

Railway 自动注入 `PORT`，服务监听 `0.0.0.0:$PORT`。`railway.json`
将 `/api/health` 配置为部署健康检查；服务启动时自动执行
`blend_kit_server/migrations/` 中的数据库迁移。

生产发布源绑定 GitHub 仓库 `EvanL1/coalassistant` 的 `main` 分支。正常发布流程是：

1. 本地完成测试并提交。
2. 推送 GitHub `main`。
3. Railway 自动读取根目录 `Dockerfile` 构建并健康检查。

不要再发布 GitHub Pages；Web 版依赖同源 Rust API，静态 Pages 无法独立运行。

本地联调：

```bash
cd doudou_blend
npm ci
npm run build

cd ../blend_kit_server
DATABASE_URL=postgresql://... STATIC_DIR=../doudou_blend/dist cargo run
```

开发环境默认账号仅供本机使用。Railway 环境缺少任一认证变量时，服务会拒绝启动，
缺少 `DATABASE_URL` 时也会拒绝启动，避免生产数据意外退回浏览器本地存储。
