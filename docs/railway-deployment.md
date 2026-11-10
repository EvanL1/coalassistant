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

后台管理员账号（可选，三个都设了才启用后台；缺任一后台关闭，普通登录不受影响）：

- `ADMIN_USERNAME`：后台账号，不要与 `AUTH_USERNAME` 相同。
- `ADMIN_PASSWORD`：后台密码。
- `ADMIN_SESSION_TOKEN`：至少 32 字节的随机会话令牌，不要与 `AUTH_SESSION_TOKEN` 相同。

用后台账号在同一个登录页登录即进入后台：煤阶交互 k、CSR 模型状态、煤库数据维护、API 密钥。

数据更新密钥**不走环境变量**——用后台账号登录后生成、命名、销毁，存在数据库里。
详见 `docs/coal-data-api.md`。

Railway 自动注入 `PORT`，服务监听 `0.0.0.0:$PORT`。`.railway/railway.ts`
将 `/api/health` 配置为部署健康检查；服务启动时自动执行
`blend_kit_server/migrations/` 中的数据库迁移。

**迁移与回滚。** 从引入迁移 `0014_coal_price_partial.sql` 的这一版起，迁移器容忍库里有
当前二进制不认识的已应用迁移（`set_ignore_missing`）：以后带新迁移的版本发布后，一键回滚到
**这一版或更新的**镜像仍能启动。这要求每个迁移都满足：

- 只做加法：加表、加列（带默认值或可空）、放宽约束；不删表删列、不改列类型、不收紧约束；
- 上一版代码在迁移后的库上照常读写（例如旧代码写出的行仍合法，新增的取值旧代码读到也不出错）；
- 已发布的迁移文件不再修改（sqlx 校验已应用迁移的校验和）。

真要删改结构时分两次发布：先发不再依赖旧结构的代码，稳定后再发删除旧结构的迁移；
回滚只能回到删除迁移之前的那一版。

**回滚到这一版之前的镜像（不能一键回滚）。** 更早的镜像没有上面的容忍设置，看到库里已应用的
`0014` 会报 `VersionMissing`、启动即退出，Railway 重试几次后服务停住。另外这一版会写
`source = 'partial'` 的估价快照（输入不全时的临时快照）；更早的版本把它原样返回，旧前端只认
`live` / `backfill`，含这种行的煤点开估价走势会显示不出来。按下面的顺序操作
（SQL 在 Supabase SQL Editor 里执行，或用上文的 `supabase db query --linked '...'`）：

1. 把临时快照改记为实时快照（这一版对外本来就把它显示成 `live`，数值不变）：

   ```sql
   UPDATE coal_price_daily SET source = 'live' WHERE source = 'partial';
   ```

   旧版本补算只填空，不会再改这些过去日子的行；当天的行它每小时照常重算。若宁可让旧版本按当天输入
   重新补算，可改为 `DELETE FROM coal_price_daily WHERE source = 'partial';`，
   但那天的输入若仍不全，旧版本补不出来，走势图上会缺那几天。
2. 删掉 `0014` 的迁移记录，旧镜像才能启动：

   ```sql
   DELETE FROM _sqlx_migrations WHERE version = 14;
   ```

   `0014` 放宽的取值约束留着即可，旧代码只写 `live` / `backfill`，不受影响；
   以后再发布这一版或更新的版本时，`0014` 会重新执行（可重复执行）。
3. 立刻在 Railway 里回滚到目标镜像。第 2 步之后、回滚之前若新版本恰好重启过，它会重新执行
   `0014` 并写回记录，旧镜像仍起不来——这时再执行一次第 2 步，然后重新部署旧镜像。
4. 旧镜像起来后再执行一次第 1 步，处理新版本停下前最后一小时可能写入的临时快照。核对：

   ```sql
   SELECT version, description FROM _sqlx_migrations ORDER BY version DESC LIMIT 3;
   SELECT count(*) FROM coal_price_daily WHERE source = 'partial';  -- 应为 0
   ```

**数据库超时。** 请求期间连接池取连接最多等 5 秒，数据库不可用时请求很快失败（`/api/health`
回 503），不会挂满 30 秒。启动时首次连库更宽容：失败后按 1、2、4、8、15、15 秒间隔重试
（每次尝试仍最多 5 秒，合计最多约 80 秒），日志里每次失败记一条“连接 PostgreSQL 失败,
稍后重试”，全部失败才退出。迁移失败不重试，直接退出。

**登录防护。** 始终生效、且不依赖来源 IP 的两项：全站同时验密码不超过 4 个（排不上 10 秒回 429），
每次失败固定等 1 秒，猜密码的总速率因此有上限。只有一个共用 Web 账号，所以不按账号锁。

按来源 IP 锁定**默认关闭**，由环境变量 `LOGIN_IP_LOCKOUT=on`（也认 `true` / `1`）打开。打开后
同一来源 15 分钟内错 5 次即锁定，回 429 和 `Retry-After`；锁定从 1 分钟起每次翻倍，最长 1 小时，
24 小时没再失败就忘掉，成功登录清零。IPv6 按 /64 归为一个来源。来源取 `X-Forwarded-For`
最左一项，取不到再看 `X-Real-IP`，再退回 TCP 对端。最左一项只有在 Railway 边缘代理**覆盖**
客户端自带的 `X-Forwarded-For` 时才可信；如果代理是在客户端的值后面**追加**，攻击者带上
`X-Forwarded-For: <受害者出口 IP>` 错 5 次，就能把那个出口后面的所有人锁住，而且可以一直重复。
所以先确认代理行为再打开：

1. 不设 `LOGIN_IP_LOCKOUT` 部署。每次登录失败，服务都会记一条 `登录失败` 日志，带
   `x_forwarded_for`、`x_real_ip`（原样，多条用 ` | ` 分隔，过长截断）、`peer`（TCP 对端）和
   `source`（锁定时用的来源键）；不记账号和密码。
2. 从一台知道公网出口 IP 的机器，故意带一个伪造的头登录失败一次：

   ```bash
   curl -s -o /dev/null -w '%{http_code}\n' https://doudou-blend.up.railway.app/api/auth/login \
     -H 'Content-Type: application/json' -H 'X-Forwarded-For: 203.0.113.77' \
     -d '{"username":"probe","password":"wrong"}'
   ```

   应回 401。然后在 Railway 日志里找这条 `登录失败`：
   - `x_forwarded_for` 只有你的真实出口 IP、看不到 `203.0.113.77`，且 `source` 等于你的出口 IP
     （IPv6 为它所在的 /64）：代理覆盖了这个头，可以打开锁定；
   - `x_forwarded_for` 是 `203.0.113.77, <你的出口 IP>`，或 `203.0.113.77` 作为单独一条出现，
     `source` 是 `203.0.113.77`：代理保留了客户端的值，**不要打开**——全站并发名额与失败等待照常生效；
   - `peer` 是 Railway 代理的内网地址，不是客户端，属正常。
3. 确认可以打开后，在 `.railway/railway.ts` 里声明 `LOGIN_IP_LOCKOUT: "on"`（那份配置是声明式的，
   只在面板里加的变量下次 apply 会被删掉），`railway config plan` 确认 0 to destroy 后 apply。
   启动日志会打印“登录按来源 IP 锁定: 开”。Railway 调整网络或换区域后，按第 2 步复查一次。

**计算上限。** 求解与估价（`/api/solve`、`/mcp`、`/api/price`）同时跑的不超过 CPU 核数
（至少 2 个），排队加计算超过 60 秒回明确的超时原因；MCP 一个请求最多 16 条消息，其中只有要求解或
估价的 `tools/call`（查合同除外）占名额，`initialize`、`tools/list` 等忙时也当场回。

**响应头。** 所有响应带 `X-Content-Type-Options: nosniff`、`X-Frame-Options: SAMEORIGIN`、
`Content-Security-Policy: frame-ancestors 'self'`、`Referrer-Policy: same-origin`；
Railway 上另带 HSTS。`AUTH_SESSION_TOKEN` 短于 32 字节时启动日志会告警（不拒绝启动）。

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
CI 仍在每次 push 时跑，坏逻辑几分钟内会亮红叉，配合 Railway 一键回滚足够
（回滚的前提见上文“迁移与回滚”）。
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
