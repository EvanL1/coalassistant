# Cloudflare Pages + D1 部署指南

本文档说明如何把豆哥配煤从纯静态(GitHub Pages)迁移到 Cloudflare Pages + D1,
让用户新增的煤种**跨设备共享**。

---

## 架构

```
浏览器 (React + WASM 求解器, 跟现在一样)
   │
   │  fetch /api/login → 拿 token
   │  fetch /api/coals (GET/POST/DELETE) [Bearer token]
   ▼
Cloudflare Pages Functions  (doudou_blend/functions/api/*)
   │
   ▼
Cloudflare D1  (user_coals 表)
```

**变与不变:**

| 内容 | 部署前 | 部署后 |
|---|---|---|
| 73 master 煤 | WASM 嵌入 | WASM 嵌入(**不变**) |
| 求解器 | 浏览器 WASM | 浏览器 WASM(**不变**) |
| coal_prefs / contract / history | localStorage | localStorage(**不变**, 单设备) |
| **user_coals** | localStorage | **D1**(**跨设备共享**) |
| 登录 | 前端 hardcode | 前端调 /api/login 验密码,token 后续用 |

---

## 一次性配置步骤(用户做)

### 1. 注册 Cloudflare(免费)

https://dash.cloudflare.com/sign-up — GitHub 一键登录。

### 2. 安装 Wrangler CLI

```bash
npm install -g wrangler
wrangler login   # 浏览器弹出授权
```

### 3. 创建 D1 数据库

```bash
wrangler d1 create coalassistant
```

输出会有一行:
```
database_id = "xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx"
```

把这个 ID 复制到 `doudou_blend/wrangler.toml` 里替换 `REPLACE_WITH_D1_DATABASE_ID`。

### 4. 跑 schema 建表

```bash
wrangler d1 execute coalassistant --remote --file=cloudflare/schema.sql
```

### 5. 创建 Pages 项目

两种方式:

**A. CLI 一键创建(推荐):**
```bash
cd doudou_blend
npm install
npm run build
wrangler pages deploy dist --project-name coalassistant
```

**B. Dashboard 接 GitHub 仓:**
1. 进 https://dash.cloudflare.com/?to=/:account/pages → Create → Connect to Git
2. 选 `EvanL1/coalassistant` 仓
3. Build settings:
   - Framework preset: None
   - Build command: `cd doudou_blend && npm install && npm run build`
   - Build output directory: `doudou_blend/dist`
   - Root directory: `/`
   - **Environment variables:** 加 `NODE_VERSION=22`

B 的好处:push main 自动重新部署,跟现在的 deploy-web 同感觉。

### 6. 设置登录密码

```bash
wrangler pages secret put AUTH_PASS --project-name coalassistant
```

提示输入密码时,**输入跟前端 hardcoded 同样的密码**(目前是 `123456`)。

> 安全说明: 后续可以删掉前端 hardcoded 密码,完全走 /api/login 验证。
> 这次先做最小可用版本,确保现有用户无缝过渡。

### 7. 绑定 D1 到 Pages 项目

Dashboard → Pages → coalassistant → Settings → Functions → D1 database bindings:
- Variable name: `DB`
- Database: `coalassistant`

(或者 wrangler.toml 里的 d1 配置会自动应用,看 Cloudflare 行为,如有疑问按 Dashboard 操作)

### 8. 验证

打开 `https://coalassistant.pages.dev/`(或自定义域),登录,加一个测试煤,
换台设备 / 浏览器无痕窗口登录看是否能看到。

---

## 一次性数据迁移

旧用户(浏览器 localStorage 里已经加了煤)登录时,**前端自动**把
localStorage 的 user_coals 上传到 D1,只跑一次,跑完打标记。

代码在 `storage.ts:migrateLocalCoalsToD1`。

---

## 日常运维

### 看煤库内容
```bash
wrangler d1 execute coalassistant --remote --command="SELECT * FROM user_coals"
```

### 清空煤库(慎用)
```bash
wrangler d1 execute coalassistant --remote --command="DELETE FROM user_coals"
```

### 备份
```bash
wrangler d1 export coalassistant --remote --output=backup.sql
```

---

## 成本估算

单租户 + 偶尔加煤的场景,**全在免费档**:

| 资源 | 免费配额 | 豆哥实际用量 |
|---|---|---|
| Pages 请求 | 100k/月 | <1k/月 |
| Workers 调用 | 100k/天 | <50/天 |
| D1 读 | 5M/天 | <500/天 |
| D1 写 | 100k/天 | <50/天 |
| D1 存储 | 5GB | <1MB |

不买域名,用 `coalassistant.pages.dev`,要自定义域名 ¥50-100/年(可选)。

---

## 已知限制 / 后续

- **单租户**: 所有人共享一个 user_coals 表. 多用户需引入 `created_by` + JWT.
- **共享密码 = token**: 简单但不可吊销. 升级路径: 登录返回随机 session token,
  worker 用 KV 存 token → user mapping.
- **coal_prefs / contract / history 仍在 localStorage**: 跨设备不同步.
  下一波可以同模式搬到 D1.
- **桌面端 Tauri 没后端**: Tauri 跑的 App 调 `/api/coals` 会 404,
  storage 层自动 fallback 用 localStorage cache. 要桌面端也共享需配置 API base URL.
- **离线写**: 离线时 addUserCoal 会 throw, UI 提示错误. 不实现离线队列.
