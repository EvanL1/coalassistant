# Railway + Supabase 部署

生产部署使用根目录 `Dockerfile`：Node 阶段构建 React，Rust 阶段编译
`blend_kit_server`，最终镜像只包含原生服务和静态资源。Railway 托管 Web
与 Rust 计算 API，Supabase 托管生产 PostgreSQL；两者均部署在新加坡。

服务必须配置以下变量：

- `AUTH_USERNAME`：Web 登录账号。
- `AUTH_PASSWORD`：Web 登录密码。
- `AUTH_SESSION_TOKEN`：至少 32 字节的随机会话令牌。
- `DATABASE_URL`：Supabase Session pooler 连接串，使用 5432 端口并启用
  `sslmode=require`。不要把连接串提交到 Git。

可选变量：

- `DATA_API_KEYS`：煤库数据更新接口的**具名**机器密钥，格式
  `名字:密钥,名字:密钥`，每把密钥至少 32 字符。**不配置则该组接口整体返回
  503**（默认不开放写入口）。

  ```bash
  # 给每个持有者单独生成一把
  openssl rand -hex 32   # -> evan 的
  openssl rand -hex 32   # -> friend 的
  ```

  ```
  DATA_API_KEYS=evan:8f3a...,friend:c91d...
  ```

  **一人一把，不要共用。** 名字不是秘密，它有两个作用：

  1. **撤销**：删掉 `friend:...` 那一段并重启，只有他失效，你的自动化不受影响。
     共用一把的话，收回任何人的权限都得让所有人换钥匙。
  2. **留痕**：落库的 `updated_by` 由服务端从密钥推导，**不是调用方自己填的**，
     所以出了坏数据查得到是谁写的。

  格式错误、名字为空、密钥过短的条目会被**跳过并告警**，不影响同一行里其他
  条目；全部无效时接口保持关闭。名字重复只保留第一条（避免"撤销了却还能用"）。

  这套密钥与 `AUTH_*` 用户登录完全独立：它只能写煤库覆盖层，碰不到用户数据与
  配煤历史；反过来，登录会话也调不动数据更新接口。

  **发给对方时**：密钥是 bearer 凭证，拿到就能用。别贴在聊天记录、issue、
  日志里；用密码管理器分享或当面给。怀疑泄露就换掉那一条，其他人无感。

Railway 自动注入 `PORT`，服务监听 `0.0.0.0:$PORT`。`railway.json`
将 `/api/health` 配置为部署健康检查；服务启动时自动执行
`blend_kit_server/migrations/` 中的数据库迁移。

生产 Supabase 项目：

- 项目名：`CoalAssistant`
- Project ref：`xbsdqlxmzidmvbafxgzn`
- 区域：`ap-southeast-1`（新加坡）

仓库内的 `blend_kit_server/migrations/` 是数据库结构的唯一迁移源。
Supabase CLI 只用于项目管理和检查，不维护第二份迁移文件。首次在本地关联：

```bash
supabase link --project-ref xbsdqlxmzidmvbafxgzn
supabase db query --linked \
  'select current_database() as database, current_user as role;'
```

生产发布源绑定 GitHub 仓库 `EvanL1/coalassistant` 的 `main` 分支。正常发布流程是：

1. 本地完成测试并提交。
2. 推送 GitHub `main`。
3. Railway 自动读取根目录 `Dockerfile` 构建并健康检查。

**CI 不是部署门禁，这是有意的。** `ci.yml` 与部署并行跑，测试红了不会阻止发布。
之所以不加 CD 门禁：Docker 构建里的 `tsc` 和 `cargo build` 已经挡住类型/编译错误，
`railway.json` 的 `/api/health`（同时探 Supabase）挡住起不来的版本，两者重合度很高；
门禁多出来的只有测试和 clippy，代价却是部署周期翻倍加一个要轮换的 token。
CI 仍在每次 push 时跑，坏逻辑几分钟内会亮红叉，配合 Railway 一键回滚足够。
等到有付费焦化厂在线上跑，再考虑分支保护 + PR（比 CD 门禁更对症）。

不要再发布 GitHub Pages；Web 版依赖同源 Rust API，静态 Pages 无法独立运行。
不要在 Railway PostgreSQL 与 Supabase 之间双写。旧 Railway PostgreSQL
以及原东京、美国西部 Supabase 项目暂时保留为回滚资源，确认新加坡项目稳定
后再单独决定是否移除。

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
