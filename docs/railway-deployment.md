# Railway 部署

生产部署使用根目录 `Dockerfile`：Node 阶段构建 React，Rust 阶段编译
`blend_kit_server`，最终镜像只包含原生服务和静态资源。

服务必须配置以下变量：

- `AUTH_USERNAME`：Web 登录账号。
- `AUTH_PASSWORD`：Web 登录密码。
- `AUTH_SESSION_TOKEN`：至少 32 字节的随机会话令牌。

Railway 自动注入 `PORT`，服务监听 `0.0.0.0:$PORT`。`railway.json`
将 `/api/health` 配置为部署健康检查。

本地联调：

```bash
cd doudou_blend
npm ci
npm run build

cd ../blend_kit_server
STATIC_DIR=../doudou_blend/dist cargo run
```

开发环境默认账号仅供本机使用。Railway 环境缺少任一认证变量时，服务会拒绝启动，
避免把开发凭据带到公网。
