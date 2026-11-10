# 服务器加固

这些步骤面向 Debian 13、1 核、约 2GB 内存、已经有其他服务的机器。聊天室只监听 `127.0.0.1:8080`，由 Caddy 对外提供 HTTPS。

动手前开着一个已经登录的 SSH 会话，改防火墙和 SSH 时不要把唯一的入口关掉。

## 1. 系统和 SSH

```bash
sudo apt update && sudo apt upgrade -y
sudo apt install -y caddy nftables fail2ban unattended-upgrades sqlite3
sudo dpkg-reconfigure -plow unattended-upgrades
```

`/etc/ssh/sshd_config` 至少包括：

```
PermitRootLogin no
PasswordAuthentication no
KbdInteractiveAuthentication no
```

然后 `sudo systemctl reload ssh`。确认密钥登录可用之后再断开旧会话。

## 2. 防火墙

```bash
sudo cp deploy/nftables.conf /etc/nftables.conf
sudo nft -f /etc/nftables.conf
sudo systemctl enable --now nftables
```

如果 SSH 不是 22 端口，先改配置里的 `dport`。规则默认丢弃其他入站流量。

## 3. 主密钥

密钥不要写进环境变量，也不要放进数据库备份。

```bash
sudo install -d -m 0700 /etc/chat
sudo sh -c 'umask 077; /usr/local/bin/chat-server keygen > /etc/chat/master.key'
```

把这个文件另外抄到离线的地方。丢了密钥，旧消息和图片都解不开。

## 4. 安装服务

把发布产物放到 `/usr/local/bin/chat-server`，`chmod 755`。

不要预先创建名为 `chat` 的系统用户。`DynamicUser=yes` 会自己分配这个名字。

编辑 `deploy/chat.service` 里的 `CHAT_PUBLIC_BASE_URL`，再：

```bash
sudo cp deploy/chat.service /etc/systemd/system/chat.service
sudo systemctl daemon-reload
sudo systemctl enable --now chat.service
sudo journalctl -u chat -f
```

先启动一次，让 systemd 建好数据目录，再停掉服务创建管理员。密码放在环境变量里，避免出现在进程列表里。用 root 写完数据库之后，把目录交回服务用户，否则服务会打不开自己的文件。

```bash
sudo systemctl start chat
owner=$(stat -c %u:%g /var/lib/chat)
sudo systemctl stop chat
sudo install -m 0600 /etc/chat/master.key /var/lib/chat/master.key
sudo chown "$owner" /var/lib/chat/master.key
export CHAT_DATA_DIR=/var/lib/chat
export CHAT_MASTER_KEY_FILE=/etc/chat/master.key
export CHAT_PUBLIC_BASE_URL=https://chat.example.com
sudo -E env CHAT_PASSWORD='一段至少十个字符的密码' /usr/local/bin/chat-server admin create ada
sudo -E /usr/local/bin/chat-server invite create
sudo chown -R "$owner" /var/lib/chat
sudo rm -f /var/lib/chat/master.key
sudo systemctl start chat
```

之后查邀请、作废邀请也用同样的停服务、执行、交回属主、再启动的方式：

```bash
chat-server invite list
chat-server invite revoke <id>
```

`invite create` 打印的链接形如 `https://域名/invite#……`。`#` 后面的内容浏览器不会发给服务器。

如果服务起不来，先看日志。`IPAddressAllow=localhost` 只允许访问本机；如果和现有网络策略冲突，删掉单元文件里的 `IPAddressDeny` 和 `IPAddressAllow` 两行。应用本身仍然只绑定 `127.0.0.1`。

## 5. Caddy

```bash
sudo install -d -m 0755 /var/log/caddy
sudo cp deploy/Caddyfile /etc/caddy/Caddyfile
sudo systemctl reload caddy
```

把文件里的域名换成你的。Caddy 会申请并续期证书。

## 6. fail2ban

```bash
sudo cp deploy/fail2ban/jail.d/chat.local /etc/fail2ban/jail.d/chat.local
sudo cp deploy/fail2ban/filter.d/caddy-chat.conf /etc/fail2ban/filter.d/caddy-chat.conf
sudo systemctl restart fail2ban
```

应用自己也会锁登录：同一 IP 或同一用户名连续失败后，锁定时间从 1 分钟增加到最多 1 小时。

## 7. 备份

服务大约每天把数据库快照写到 `/var/lib/chat/backups/chat.db`。图片在 `/var/lib/chat/files`，内容已加密。

```bash
sudo install -D -m 0755 deploy/backup.sh /usr/local/lib/chat/backup.sh
sudo cp deploy/chat-backup.service deploy/chat-backup.timer /etc/systemd/system/
sudo systemctl enable --now chat-backup.timer
```

恢复时停掉服务，用快照换掉 `chat.db`，把 `files` 拷回去，并使用原来的主密钥。

## 8. 这套措施防什么

- 没有邀请就不能注册。链接一次性、默认 7 天、可作废，数据库只存哈希。
- 密码用 Argon2id。会话令牌是随机的，数据库只存哈希，Cookie 是 HttpOnly、SameSite=Strict；HTTPS 下使用 `__Host-` 前缀。
- 浏览器跨站请求会被 Origin 和 Sec-Fetch-Site 拒绝。
- 图片会重新编码成 WebP，EXIF 和 GPS 不会留下来。解码有像素上限，避免一张图把内存打满。
- 消息正文和图片文件用 XChaCha20-Poly1305 加密后落盘。密钥通过 systemd `LoadCredential` 注入。

加密不能防已经拿到这台机器 root 的人：密钥和数据在同一台服务器上。它防的是备份泄露和磁盘被拿走。HTTPS、防火墙和最小权限防的是外部直接打进进程。
