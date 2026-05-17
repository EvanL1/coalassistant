# 部署到 VPS

豆哥配煤从 Cloudflare Workers 迁到自建 VPS, 因为 CF 在国内访问稳定性
不可靠. 这套用 Node.js + Caddy + better-sqlite3, 单文件 SQLite 存全部
业务数据.

## 1. 前置

- VPS, 推荐 LocVPS 东京 JPTY-EXP (2 核 4G ¥360/年, 三网直连)
  或任意 1 核 1G+ Linux. **位置很重要**: 选三网直连日本/香港机器, 大陆
  访问稳定; 美西/欧洲机器豆哥那边会很慢
- 一个域名指 VPS IP (买个 `.xyz` 几十块/年). DNS 走 Cloudflare:
  - A 记录 `doudou.你的域.xyz` → VPS IP
  - **关橙云走灰云** (Proxy status: DNS only), Caddy 才能拿 Let's Encrypt 证书
- VPS 上 SSH 进去 (root 或 sudo)

## 2. 一键部署

```bash
# 在 VPS 上
git clone https://github.com/evanl1/coalassistant.git /tmp/coal
cd /tmp/coal

# 把 doudou.example.com 改成你自己的域名, AUTH_PASS 设个豆哥能记的密码
sudo AUTH_PASS='setapassword123' DOMAIN='doudou.example.com' bash deploy/setup.sh
```

完成后:
- 浏览器打开 `https://doudou.example.com`
- 账号 `doudou`, 密码就是你刚设的 `AUTH_PASS`

## 3. 更新代码

```bash
ssh vps
cd /opt/doudou-blend
sudo -u doudou-blend git pull
sudo -u doudou-blend bash -c "source ~/.cargo/env && cd blend_kit_wasm && wasm-pack build --target web --out-dir pkg --release"
sudo -u doudou-blend bash -c "cd doudou_blend && npm ci && npm run build && npm run build:server"
sudo systemctl restart doudou-blend
```

可以写成 `deploy/update.sh` 一键跑.

## 4. 调试

```bash
# 看服务日志
journalctl -u doudou-blend -f

# 看 Caddy 日志
journalctl -u caddy -f

# 服务状态
systemctl status doudou-blend

# 直接打 Node (跳过 Caddy 排查反代问题)
curl http://127.0.0.1:8787/api/login -X POST -H 'Content-Type: application/json' \
  -d '{"user":"doudou","password":"setapassword123"}'

# 看 SQLite 数据
sudo -u doudou-blend sqlite3 /var/lib/doudou-blend/data.db '.tables'
sudo -u doudou-blend sqlite3 /var/lib/doudou-blend/data.db 'SELECT * FROM customers'
```

## 5. 备份

业务数据全在 `/var/lib/doudou-blend/data.db` 一个文件里 (加 WAL 模式下还有
`-wal` / `-shm` 两个临时文件, 备份时一起拷).

简单粗暴的 cron 备份:
```bash
# /etc/cron.daily/doudou-backup
#!/usr/bin/env bash
TS=$(date +%Y%m%d-%H%M%S)
mkdir -p /var/backups/doudou-blend
sudo -u doudou-blend sqlite3 /var/lib/doudou-blend/data.db ".backup /var/backups/doudou-blend/data-$TS.db"
# 保留 30 天
find /var/backups/doudou-blend -name 'data-*.db' -mtime +30 -delete
```

`.backup` 命令是 SQLite 内置, 在数据库运行时也能安全拷贝.

## 6. 从 Cloudflare D1 迁数据 (老用户)

如果之前在 CF 上有数据, 现在迁过来:

```bash
# 在本机, 拉 D1 dump
cd doudou_blend
npx wrangler d1 export DB --output=migrate.sql

# 复制到 VPS
scp migrate.sql vps:/tmp/

# VPS 上导入
sudo systemctl stop doudou-blend
sudo -u doudou-blend sqlite3 /var/lib/doudou-blend/data.db ".read /tmp/migrate.sql"
sudo systemctl start doudou-blend
```

D1 的 schema 跟 SQLite schema 完全一致, 直接导入零损耗.

## 7. 卸载

```bash
sudo systemctl stop doudou-blend caddy
sudo systemctl disable doudou-blend
sudo rm /etc/systemd/system/doudou-blend.service /etc/caddy/Caddyfile /etc/doudou-blend.env
sudo rm -rf /opt/doudou-blend /var/lib/doudou-blend
sudo userdel doudou-blend
```
