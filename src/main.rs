use std::io::{self, Write};
use std::net::SocketAddr;
use std::path::PathBuf;

use chat_room::{check, create_admin, generate_passphrase, serve, ServeConfig};
use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "chat-room", version, about = "单文件部署的私密聊天室")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// 启动聊天服务和管理后台
    Serve {
        #[arg(long, default_value = "/var/lib/chat-room")]
        data_dir: PathBuf,
        #[arg(long, default_value = "127.0.0.1:8080")]
        listen: SocketAddr,
        #[arg(long, default_value = "127.0.0.1:8081")]
        admin_listen: SocketAddr,
        #[arg(long)]
        public_url: String,
        #[arg(long)]
        master_key: Option<PathBuf>,
        /// 仅用于本机 http 测试。生产环境不要使用。
        #[arg(long)]
        insecure_cookies: bool,
    },
    /// 创建第一位管理员。会先显示两步验证密钥，确认验证码后才写入。
    CreateAdmin {
        #[arg(long, default_value = "/var/lib/chat-room")]
        data_dir: PathBuf,
        #[arg(long)]
        master_key: Option<PathBuf>,
        #[arg(long)]
        username: String,
        #[arg(long)]
        display_name: Option<String>,
        #[arg(long)]
        password: Option<String>,
        #[arg(long)]
        totp_code: Option<String>,
    },
    /// 生成一个随机主密钥，写到标准输出
    GenKey,
    /// 检查主密钥能否解密当前数据
    Check {
        #[arg(long, default_value = "/var/lib/chat-room")]
        data_dir: PathBuf,
        #[arg(long)]
        master_key: Option<PathBuf>,
    },
}

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "chat_room=info".into()),
        )
        .init();
    let cli = Cli::parse();
    run(cli);
}

fn run(cli: Cli) {
    match cli.command {
        Command::GenKey => println!("{}", generate_passphrase()),
        Command::Check {
            data_dir,
            master_key,
        } => match check(&data_dir, &read_passphrase(master_key.as_deref())) {
            Ok(message) => println!("{message}"),
            Err(err) => fail(&err.to_string()),
        },
        Command::CreateAdmin {
            data_dir,
            master_key,
            username,
            display_name,
            password,
            totp_code,
        } => {
            let password = password.unwrap_or_else(|| prompt("管理员密码: "));
            let display = display_name.unwrap_or_else(|| username.clone());
            let secret = show_totp(&username);
            let code = totp_code.unwrap_or_else(|| prompt("当前验证码: "));
            match create_admin(
                &data_dir,
                &read_passphrase(master_key.as_deref()),
                &username,
                &password,
                &display,
                &secret,
                &code,
            ) {
                Ok(user) => {
                    println!("管理员已创建");
                    println!("用户名: {}", user.username);
                    println!("UID: {}", user.uid);
                }
                Err(err) => fail(&err.to_string()),
            }
        }
        Command::Serve {
            data_dir,
            listen,
            admin_listen,
            public_url,
            master_key,
            insecure_cookies,
        } => {
            let config = ServeConfig {
                data_dir,
                passphrase: read_passphrase(master_key.as_deref()),
                public_url,
                listen,
                admin_listen,
                insecure_cookies,
            };
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("运行时");
            if let Err(err) = runtime.block_on(serve(config)) {
                fail(&err.to_string());
            }
        }
    }
}

fn show_totp(username: &str) -> Vec<u8> {
    let secret = chat_room::totp_secret();
    match chat_room::totp_view(&secret, username) {
        Ok((base32, url)) => {
            println!("两步验证密钥: {base32}");
            println!("otpauth: {url}");
            println!("请把密钥加入验证器，然后输入当前 6 位验证码。");
            let _ = io::stdout().flush();
        }
        Err(err) => fail(err),
    }
    secret
}

fn read_passphrase(path: Option<&std::path::Path>) -> String {
    let path = path
        .map(|path| path.to_path_buf())
        .or_else(|| std::env::var_os("CHAT_MASTER_KEY_FILE").map(PathBuf::from))
        .or_else(|| {
            std::env::var_os("CREDENTIALS_DIRECTORY")
                .map(|dir| PathBuf::from(dir).join("master.key"))
        });
    let Some(path) = path else {
        fail("没有找到主密钥。请设置 --master-key，或由 systemd LoadCredential 提供 master.key");
    };
    match std::fs::read_to_string(&path) {
        Ok(mut text) => {
            if text.ends_with('\n') {
                text.pop();
            }
            if text.ends_with('\r') {
                text.pop();
            }
            if text.is_empty() {
                fail("主密钥文件是空的");
            }
            text
        }
        Err(err) => fail(&format!("无法读取主密钥 {}: {err}", path.display())),
    }
}

fn prompt(label: &str) -> String {
    eprint!("{label}");
    let _ = io::stderr().flush();
    let mut line = String::new();
    if io::stdin().read_line(&mut line).is_err() {
        fail("无法读取输入");
    }
    line.trim_end_matches(['\n', '\r']).to_string()
}

fn fail(message: &str) -> ! {
    eprintln!("{message}");
    std::process::exit(1);
}
