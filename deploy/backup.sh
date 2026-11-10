#!/bin/sh
# 复制服务自己生成的数据库快照和加密图片。
# 不要在服务运行时用 root 直接打开正在使用的 chat.db，否则可能留下 root 拥有的 WAL 文件。
set -eu
src=/var/lib/chat
dest=${1:-/var/backups/chat}
stamp=$(date -u +%Y%m%dT%H%M%SZ)
out="$dest/$stamp"
mkdir -p "$out/db" "$out/files"
if [ -d "$src/backups" ]; then
  cp -a "$src/backups/." "$out/db/"
fi
if [ -d "$src/files" ]; then
  cp -a "$src/files/." "$out/files/"
fi
echo "已复制到 $out"
echo "主密钥不在这份备份里。请把 /etc/chat/master.key 另行离线保存。"
