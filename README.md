# 聊天室

单文件部署的私密聊天服务。Rust 后端把 PWA 前端嵌进同一个二进制，Debian 13 上只需要这一个文件、SQLite 和本地磁盘。面向 100 人以内、信任自己服务器的小圈子。

## 能做什么

- 只有管理员能生成邀请链接。每个链接只能用一次，默认 7 天过期，可以随时作废。
- 注册时分配不重复的 7 位 UID。用用户名或 UID 搜索，必须完全匹配，不区分大小写。
- 先发好友申请，对方同意后才能私聊，或被拉进群。
- 群主可以改名、踢人、设置保留时间、转让群主。普通成员可以拉自己的好友进群，也可以自己退群。群上限 100 人。
- 每个会话可以设置 1 小时、1 天、7 天或 30 天后自动删除。默认 30 天，不能更长。到期后消息和图片一起删除。
- 可以撤回自己的消息（所有人看到“消息已撤回”，服务器上的正文和图片立即删除），也可以只从自己这边删除。
- 上传的图片会重新编码成 JPEG、缩到最长边 2048 像素，并去掉 EXIF/GPS。原图不会写入磁盘。
- 可选的浏览器推送只写“你有新消息”，不包含正文和发送者。

## 安全模型

这套程序假设你信任正在运行它的服务器。全程通过 Caddy 提供 HTTPS。消息正文、群名、两步验证密钥和图片在磁盘上用 XChaCha20-Poly1305 加密。主密钥来自你自己保管的口令，程序用 Argon2id（19 MiB、2 次迭代）加每台机器独立的盐派生出密钥。

用户名、UID、好友关系和会话成员关系是明文，否则无法搜索和做权限判断。服务器进程运行时内存里有密钥。如果别人同时拿到数据库和 `/etc/chat-room/master.key`，就能解密。备份时这两个东西要一起收好，也要分开保管。

密码用 Argon2id 存储。登录会话放在 `HttpOnly`、`SameSite=Strict` 的 Cookie 里，只存哈希。登录和管理员操作有频率限制。管理员登录必须使用两步验证。

## 登录

打开域名就会进入登录页。地址是 `https://你的域名/#/login`，写成 `https://你的域名/login` 也一样。注册页上有「已有账号？去登录」。邀请链接用过、过期或作废时，页面会直接说明，并给出登录按钮。

手机上的退出在「设置」页顶部。桌面端左侧栏最下面也有退出。退出之后再打开域名，会回到登录页。没有账号时，只能用管理员发给你的邀请链接注册。

## 管理后台

管理页面和聊天服务在同一个进程里，只监听 `127.0.0.1:8081`，不经过 Caddy。在你自己的电脑上执行：

```sh
ssh -L 8081:127.0.0.1:8081 user@your-server
```

然后打开 `http://127.0.0.1:8081`。可以生成和作废邀请、停用或删除用户、重置密码、指定管理员、查看存储和内存占用、压缩数据库。后台不显示消息正文和群名。

## 在 Debian 13 上安装

目标机器是 x86_64，不需要在上面编译。发布页的压缩包是静态链接的 musl 二进制。

1. 安装 Caddy，并准备一个指向这台机器的域名。
2. 从 [Releases](https://github.com/ChenPingluo/chat-room/releases) 下载 `chat-room-vX.Y.Z-x86_64-linux-musl.tar.gz` 和同名 `.sha256`。
3. 校验并解压：

```sh
sha256sum -c chat-room-vX.Y.Z-x86_64-linux-musl.tar.gz.sha256
tar -xzf chat-room-vX.Y.Z-x86_64-linux-musl.tar.gz
cd chat-room-vX.Y.Z
sudo PUBLIC_URL=https://chat.example.com ./deploy/install.sh
```

4. 生成主密钥。文件末尾允许有一个换行，程序会去掉它。不要把密钥提交到仓库。

```sh
umask 077
sudo mkdir -p /etc/chat-room
chat-room gen-key | sudo tee /etc/chat-room/master.key >/dev/null
sudo chown root:root /etc/chat-room/master.key
sudo chmod 600 /etc/chat-room/master.key
```

5. 确认 `/etc/systemd/system/chat-room.service` 里的 `--public-url` 是你的 `https://` 地址。
6. 创建管理员。命令会先打印两步验证密钥，你把它加进验证器后输入当前验证码，账号才会写入。

```sh
sudo chat-room create-admin --master-key /etc/chat-room/master.key --data-dir /var/lib/chat-room --username admin
```

密码也可以不放在命令行里，程序会提示输入。不要用 `--password`，否则它会出现在进程列表里。

7. 启动并检查：

```sh
sudo systemctl enable --now chat-room
sudo systemctl status chat-room
chat-room check --data-dir /var/lib/chat-room
```

8. 把 `deploy/Caddyfile` 里的域名改成你的，放到 Caddy 配置里并重载。只反代 `127.0.0.1:8080`。不要公开 8081。

升级时重新执行 `install.sh`，然后 `sudo systemctl restart chat-room`。已存在的 systemd 单元不会被覆盖。主密钥和 `/var/lib/chat-room` 不要删。

### 从 v0.2.0 升级到 v0.2.1

这次没有改数据库、接口、Cookie 和 systemd 单元。已有账号、登录状态和消息都会留着。不要删除 `/var/lib/chat-room` 和 `/etc/chat-room/master.key`。服务器上不需要编译，下载后替换二进制再重启即可。

```sh
curl -LO https://github.com/ChenPingluo/chat-room/releases/download/v0.2.1/chat-room-v0.2.1-x86_64-linux-musl.tar.gz
curl -LO https://github.com/ChenPingluo/chat-room/releases/download/v0.2.1/chat-room-v0.2.1-x86_64-linux-musl.tar.gz.sha256
sha256sum -c chat-room-v0.2.1-x86_64-linux-musl.tar.gz.sha256
tar -xzf chat-room-v0.2.1-x86_64-linux-musl.tar.gz
cd chat-room-v0.2.1
sudo ./deploy/install.sh
sudo systemctl restart chat-room
```

同一个压缩包也放在仓库的 `release/` 目录里。已经克隆了仓库的话，拉取对应标签后可以直接用这个文件，不必再下载一次。

如果 systemd 因为 `MemoryDenyWriteExecute` 或系统调用过滤无法启动，服务日志里会有 `Permission denied` 或 `Operation not permitted`。注释掉对应那一行后再启动。内存上限 `MemoryMax=300M` 建议保留。

## 服务器加固

- SSH 只允许密钥登录，关掉密码登录。
- 防火墙只开放 22、80、443。8080 和 8081 只监听本机。
- 开启 `unattended-upgrades`，自动安装安全更新。
- `/etc/chat-room` 保持 `0700`，`master.key` 保持 `0600` 且属主是 root。
- Caddy 的访问日志不要记录请求体。仓库里的示例配置没有记录请求体。
- 定期把 `/var/lib/chat-room` 和 `master.key` 分开备份。只备份其中一个无法恢复聊天记录。

## 本地开发和编译

需要 Rust stable（1.87 或更新）和 Node.js 22。

```sh
cd web && npm ci && npm run build && cd ..
cargo test
cargo build --release
```

本地不用 HTTPS 时：

```sh
chat-room gen-key > /tmp/chat-room.key
chat-room serve --data-dir /tmp/chat-data --master-key /tmp/chat-room.key \
  --public-url http://127.0.0.1:8080 --insecure-cookies
```

发布用的静态二进制在推送 `v*` 标签后由 GitHub Actions 编译，目标是 `x86_64-unknown-linux-musl`。本机交叉编译需要 `musl-tools`，仓库的 `.cargo/config.toml` 已经指定 `musl-gcc`。

## 资源占用

服务使用单线程 Tokio，SQLite 使用 WAL，缓存约 2MB，并关闭 mmap。密码哈希和图片处理都是一次只做一件。空闲内存目标低于 100MB，systemd 在 300MB 时强制停止进程。图片一次只处理一张，单张上传不超过 25MB，解码像素不超过 2500 万。

## 命令

```text
chat-room serve --data-dir DIR --public-url https://example.com
chat-room create-admin --data-dir DIR --username NAME
chat-room gen-key
chat-room check --data-dir DIR
```

主密钥默认从 `--master-key`、环境变量 `CHAT_MASTER_KEY_FILE`，或 systemd 的 `CREDENTIALS_DIRECTORY/master.key` 读取。
