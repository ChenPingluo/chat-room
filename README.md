# 私语

邀请制的私密聊天室。注册之后，界面里只有你自己。想聊天，就搜索对方注册时确定的用户名，或者 7 位 UID。两边必须完全一致。对方同意好友申请之后，才能私聊或拉进群。

适合一台还要跑其他服务的小机器：1 核、2GB 内存。服务是单个 Rust 二进制，数据在 SQLite 里，图片在本地目录里。

## 功能

- 管理员生成一次性邀请链接，默认 7 天有效，可以作废。
- 纯文本和图片。图片会重新编码，去掉位置等元数据。
- 好友申请、私聊、群聊。群最多 100 人。
- 每个会话可以选择 1 小时、1 天、7 天或 30 天后删除。上限是 30 天，默认 30 天。
- 可以删除自己的消息，也可以退出其他设备上的登录。

## 安全边界

服务器是你自己的，管理员在技术上可以读到内容。磁盘上的消息和图片仍然加密，用来防止备份或硬盘离开机器之后被直接读出来。已经拿到 root 的人同时拥有密钥和数据，加密拦不住。

对外只开放 HTTPS。应用监听 `127.0.0.1`。部署步骤在 [deploy/hardening.md](deploy/hardening.md)。

当前能用的 sqlx 还捆绑着一个有本地漏洞的 SQLite：有人如果能把一个页大小对不上的 WAL 文件放进数据目录，进程打开它时可能读出内存里的残留。数据目录权限是 `0700`，只有服务自己能写。依赖升级之后就可以拿掉 `server/.cargo/audit.toml` 里的忽略项。MySQL 的 `rsa` 警告来自 sqlx 的过程宏，最终的二进制不包含它。

图片解码限制在 1600 万像素、长边 5000 像素以内，保存前缩小到长边 2048。这是为了让 300MB 的内存上限不会被一张大图打穿。

## 本地开发

需要 Rust stable 和 Node.js 22。

```bash
cd web && npm ci && npm run build
cd ../server
export CHAT_MASTER_KEY="$(cargo run -- keygen)"
export CHAT_DATA_DIR=/tmp/chat-data
export CHAT_PUBLIC_BASE_URL=http://127.0.0.1:8080
export CHAT_COOKIE_SECURE=0
cargo run -- serve
```

另开一个终端创建管理员。密码放在环境变量里，不要写进命令参数：

```bash
export CHAT_PASSWORD='一段足够长的密码'
cargo run -- admin create ada
cargo run -- invite create
```

浏览器打开打印出来的链接。

检查：

```bash
cd server && cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test
```

## 发布构建

不要在那台 1 核的服务器上编译。在本机或 CI 上打静态二进制：

```bash
cd web && npm ci && npm run build
cd ../server
rustup target add x86_64-unknown-linux-musl
cargo build --release --target x86_64-unknown-linux-musl
```

产物是 `server/target/x86_64-unknown-linux-musl/release/chat-server`。Debian 上编译 musl 需要安装 `musl-tools`。

## 运行时占用

Tokio 使用单线程运行时。SQLite 最多 4 个连接，缓存约 8MB。密码哈希同一时间最多算 2 个，图片同一时间处理 1 张。systemd 单元把进程限制在 300MB 内存和 60% CPU。

几十人、不到 100 人的规模，空闲时大约几十 MB。
