#!/usr/bin/env bash
# 一键 VPS 部署脚本 (Debian 12 / Ubuntu 22.04+).
#
# 用法 (在 VPS 上, root 或 sudo):
#   AUTH_PASS=你设的密码 DOMAIN=doudou.example.com bash setup.sh
#
# 流程:
#   1. 装 Node 22 + Caddy + git + build 工具
#   2. 建 doudou-blend 用户 + 目录
#   3. clone 仓库, 装依赖, build
#   4. 写 /etc/doudou-blend.env (AUTH_PASS)
#   5. 装 systemd unit + Caddyfile
#   6. 启服务

set -euo pipefail

: "${AUTH_PASS:?必须设 AUTH_PASS 环境变量 (登录密码)}"
: "${DOMAIN:?必须设 DOMAIN 环境变量 (你买的域名, 如 doudou.example.com)}"

REPO_URL="${REPO_URL:-https://github.com/evanl1/coalassistant.git}"
BRANCH="${BRANCH:-main}"

# ==========================================================
# 1. 系统依赖
# ==========================================================
echo "==> 装 Node 22 + Caddy + 基础工具"
apt-get update
apt-get install -y curl git build-essential debian-keyring debian-archive-keyring apt-transport-https

# Node 22 (nodesource)
curl -fsSL https://deb.nodesource.com/setup_22.x | bash -
apt-get install -y nodejs

# Caddy (官方仓库)
curl -1sLf 'https://dl.cloudsmith.io/public/caddy/stable/gpg.key' \
	| gpg --dearmor -o /usr/share/keyrings/caddy-stable-archive-keyring.gpg
curl -1sLf 'https://dl.cloudsmith.io/public/caddy/stable/debian.deb.txt' \
	> /etc/apt/sources.list.d/caddy-stable.list
apt-get update
apt-get install -y caddy

# Rust (build blend_kit_wasm 需要)
if ! command -v cargo > /dev/null; then
	curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
	# shellcheck disable=SC1091
	source "$HOME/.cargo/env"
fi

# wasm-pack
if ! command -v wasm-pack > /dev/null; then
	curl https://rustwasm.github.io/wasm-pack/installer/init.sh -sSf | sh
fi

# ==========================================================
# 2. 用户 + 目录
# ==========================================================
echo "==> 建 doudou-blend 用户 + 目录"
id -u doudou-blend > /dev/null 2>&1 || useradd --system --create-home --shell /bin/bash doudou-blend

mkdir -p /opt/doudou-blend /var/lib/doudou-blend
chown -R doudou-blend:doudou-blend /opt/doudou-blend /var/lib/doudou-blend

# ==========================================================
# 3. 拉代码 + build
# ==========================================================
echo "==> Clone + build"
if [[ -d /opt/doudou-blend/.git ]]; then
	sudo -u doudou-blend git -C /opt/doudou-blend fetch origin
	sudo -u doudou-blend git -C /opt/doudou-blend reset --hard "origin/$BRANCH"
else
	sudo -u doudou-blend git clone -b "$BRANCH" "$REPO_URL" /opt/doudou-blend
fi

cd /opt/doudou-blend

# 编 WASM (blend_kit_wasm)
sudo -u doudou-blend bash -c "source ~/.cargo/env && cd blend_kit_wasm && wasm-pack build --target web --out-dir pkg --release"

# Node deps + build (前端 dist + 服务端 dist-server)
sudo -u doudou-blend bash -c "cd doudou_blend && npm ci && npm run build && npm run build:server"

# 把 build 产物 + node_modules 链到根目录给 systemd 用
sudo -u doudou-blend ln -sfn /opt/doudou-blend/doudou_blend/dist /opt/doudou-blend/dist
sudo -u doudou-blend ln -sfn /opt/doudou-blend/doudou_blend/dist-server /opt/doudou-blend/dist-server
sudo -u doudou-blend ln -sfn /opt/doudou-blend/doudou_blend/node_modules /opt/doudou-blend/node_modules

# ==========================================================
# 4. 配置文件 (AUTH_PASS 单独存)
# ==========================================================
echo "==> 写 /etc/doudou-blend.env"
cat > /etc/doudou-blend.env <<EOF
AUTH_PASS=$AUTH_PASS
EOF
chmod 600 /etc/doudou-blend.env
chown root:doudou-blend /etc/doudou-blend.env

# ==========================================================
# 5. systemd + Caddy
# ==========================================================
echo "==> 装 systemd unit"
cp deploy/systemd/doudou-blend.service /etc/systemd/system/
systemctl daemon-reload
systemctl enable doudou-blend

echo "==> 装 Caddyfile (域名: $DOMAIN)"
sed "s/doudou.example.com/$DOMAIN/g" deploy/Caddyfile > /etc/caddy/Caddyfile

# ==========================================================
# 6. 启动
# ==========================================================
echo "==> 启服务"
systemctl restart doudou-blend
systemctl reload caddy

sleep 2
systemctl status doudou-blend --no-pager | head -15

echo
echo "===================================="
echo "部署完成!"
echo "  服务日志:  journalctl -u doudou-blend -f"
echo "  Caddy 日志: journalctl -u caddy -f"
echo "  浏览器打开: https://$DOMAIN"
echo "  登录账号:  doudou"
echo "  登录密码:  (你刚设的 AUTH_PASS)"
echo "===================================="
