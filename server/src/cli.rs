use std::net::SocketAddr;
use std::path::PathBuf;

use anyhow::Context;
use clap::{Parser, Subcommand};
use tokio::net::TcpListener;

use crate::domain::auth as accounts;
use crate::janitor;
use crate::state::AppState;
use crate::util;
use crate::Config;

pub fn run() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    runtime.block_on(async move {
        match cli.command {
            Command::Serve {
                data_dir,
                listen,
                public_base_url,
            } => serve(data_dir, listen, public_base_url).await,
            Command::Keygen => {
                let token = util::random_token()?;
                println!("{token}");
                Ok(())
            }
            Command::Admin { command } => match command {
                AdminCommand::Create { username } => {
                    let password = read_secret("密码")?;
                    let state = boot_from_env().await?;
                    let id = accounts::provision_admin(&state, &username, &password).await?;
                    println!("已创建管理员 {username}，用户 id {id}");
                    Ok(())
                }
            },
            Command::Invite { command } => match command {
                InviteCommand::Create { days } => {
                    let state = boot_from_env().await?;
                    let admin_id = only_admin_id(&state).await?;
                    let created = accounts::create_invite_for(&state, admin_id, days).await?;
                    println!("{}", created.url);
                    Ok(())
                }
                InviteCommand::List => {
                    let state = boot_from_env().await?;
                    let admin = admin_user(&state).await?;
                    let invites = accounts::list_invites(&state, &admin).await?;
                    for invite in invites {
                        let used = invite
                            .used_by
                            .map(|user| user.username)
                            .unwrap_or_else(|| "-".into());
                        println!(
                            "{}\texpires={}\trevoked={}\tused_by={}",
                            invite.id, invite.expires_at, invite.revoked, used
                        );
                    }
                    Ok(())
                }
                InviteCommand::Revoke { id } => {
                    let state = boot_from_env().await?;
                    let admin = admin_user(&state).await?;
                    accounts::revoke_invite(&state, &admin, id).await?;
                    println!("已作废邀请 {id}");
                    Ok(())
                }
            },
            Command::Backup { dest } => {
                let state = boot_from_env().await?;
                janitor::snapshot_database(&state.db, &dest).await?;
                let files = state.files_dir();
                let target = dest.join("files");
                copy_dir(&files, &target)?;
                println!("已备份到 {}", dest.display());
                Ok(())
            }
        }
    })
}

async fn serve(
    data_dir: Option<PathBuf>,
    listen: Option<SocketAddr>,
    public_base_url: Option<String>,
) -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,sqlx=warn".into()),
        )
        .init();
    let config = Config::from_env()?.with_overrides(data_dir, listen, public_base_url)?;
    let listen = config.listen;
    let state = AppState::boot(config).await?;
    let users: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM users")
        .fetch_one(&state.db)
        .await?;
    if users == 0 {
        tracing::warn!("还没有用户。先运行 chat-server admin create --username <名字>");
    }
    janitor::spawn(state.clone());
    let app = crate::router(state).into_make_service_with_connect_info::<SocketAddr>();
    let listener = TcpListener::bind(listen)
        .await
        .with_context(|| format!("无法监听 {listen}"))?;
    tracing::info!(%listen, "私语已启动");
    axum::serve(listener, app)
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await?;
    Ok(())
}

async fn boot_from_env() -> anyhow::Result<AppState> {
    Ok(AppState::boot(Config::from_env()?).await?)
}

async fn only_admin_id(state: &AppState) -> anyhow::Result<i64> {
    let id: Option<i64> =
        sqlx::query_scalar("SELECT id FROM users WHERE is_admin = 1 ORDER BY id LIMIT 1")
            .fetch_optional(&state.db)
            .await?;
    id.context("还没有管理员，请先运行 chat-server admin create")
}

async fn admin_user(state: &AppState) -> anyhow::Result<crate::auth::AuthUser> {
    let id = only_admin_id(state).await?;
    Ok(crate::auth::AuthUser {
        id,
        uid: String::new(),
        username: String::new(),
        is_admin: true,
        session_id: 0,
    })
}

fn read_secret(label: &str) -> anyhow::Result<String> {
    if let Ok(value) = std::env::var("CHAT_PASSWORD") {
        if !value.is_empty() {
            return Ok(value);
        }
    }
    eprint!("{label}：");
    let mut line = String::new();
    std::io::stdin().read_line(&mut line)?;
    let line = line.trim().to_string();
    if line.is_empty() {
        anyhow::bail!("{label}不能为空");
    }
    Ok(line)
}

fn copy_dir(from: &std::path::Path, to: &std::path::Path) -> anyhow::Result<()> {
    std::fs::create_dir_all(to)?;
    if !from.exists() {
        return Ok(());
    }
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let target = to.join(entry.file_name());
        if entry.file_type()?.is_file() {
            std::fs::copy(entry.path(), target)?;
        }
    }
    Ok(())
}

#[derive(Parser)]
#[command(name = "chat-server", about = "邀请制私密聊天室")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// 启动服务
    Serve {
        #[arg(long)]
        data_dir: Option<PathBuf>,
        #[arg(long)]
        listen: Option<SocketAddr>,
        #[arg(long)]
        public_base_url: Option<String>,
    },
    /// 生成 32 字节主密钥（base64）
    Keygen,
    /// 管理员账号
    Admin {
        #[command(subcommand)]
        command: AdminCommand,
    },
    /// 邀请链接
    Invite {
        #[command(subcommand)]
        command: InviteCommand,
    },
    /// 把数据库快照和图片复制到目标目录。建议先停止服务，或直接使用服务自己的每日快照。
    Backup { dest: PathBuf },
}

#[derive(Subcommand)]
enum AdminCommand {
    /// 创建管理员。密码从环境变量 CHAT_PASSWORD 读取，否则从标准输入读取。
    Create { username: String },
}

#[derive(Subcommand)]
enum InviteCommand {
    /// 创建一次性邀请，默认 7 天有效
    Create {
        #[arg(long, default_value_t = 7)]
        days: i64,
    },
    List,
    Revoke {
        id: i64,
    },
}
