#!/bin/sh
# 在 Debian 13 上安装或升级。需要 root。
# 从发布包解压后执行：sudo ./deploy/install.sh
# 可选环境变量 PUBLIC_URL=https://chat.example.com
set -eu

ROOT=$(CDPATH= cd -- "$(dirname "$0")/.." && pwd)
BIN="$ROOT/chat-room"
if [ ! -f "$BIN" ]; then
  echo "没有找到 $BIN。请先解压发布包。" >&2
  exit 1
fi

if [ "$(id -u)" -ne 0 ]; then
  echo "请用 root 执行。" >&2
  exit 1
fi

install -m 0755 "$BIN" /usr/local/bin/chat-room
install -d -m 0700 /etc/chat-room
UNIT=/etc/systemd/system/chat-room.service
if [ -f "$UNIT" ]; then
  echo "已存在 $UNIT，升级时不覆盖。请自行对照 $ROOT/deploy/chat-room.service。"
else
  if [ -n "${PUBLIC_URL:-}" ]; then
    sed "s#https://chat.example.com#${PUBLIC_URL}#g" "$ROOT/deploy/chat-room.service" > "$UNIT"
  else
    install -m 0644 "$ROOT/deploy/chat-room.service" "$UNIT"
  fi
fi
install -m 0644 "$ROOT/deploy/Caddyfile" /etc/chat-room/Caddyfile.example

if ! id chatroom >/dev/null 2>&1; then
  useradd --system --home-dir /var/lib/chat-room --create-home --shell /usr/sbin/nologin chatroom
fi
install -d -m 0700 -o chatroom -g chatroom /var/lib/chat-room

systemctl daemon-reload
echo "二进制已安装到 /usr/local/bin/chat-room"
if [ ! -f /etc/chat-room/master.key ]; then
  echo "下一步：生成主密钥并限制权限"
  echo "  umask 077"
  echo "  chat-room gen-key > /etc/chat-room/master.key"
  echo "  chown root:root /etc/chat-room/master.key"
  echo "  chmod 600 /etc/chat-room/master.key"
fi
echo "编辑 /etc/systemd/system/chat-room.service 里的 --public-url，然后："
echo "  chat-room create-admin --master-key /etc/chat-room/master.key --data-dir /var/lib/chat-room --username 你的名字"
echo "  systemctl enable --now chat-room"
echo "管理后台只在服务器本机的 127.0.0.1:8081。用 ssh -L 8081:127.0.0.1:8081 打开。"
