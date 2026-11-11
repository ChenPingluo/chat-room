use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tokio::sync::Semaphore;

use crate::crypto::{self, Key};
use crate::db;
use crate::error::{AppErr, StartupError};
use crate::files;
use crate::hub::Hub;
use crate::models::{ConversationOut, MessageOut, PublicUser, SessionUser, User};
use crate::push::{self, Vapid};
use crate::rate::RateLimiter;
use crate::repo::FriendRequestRow;
use crate::repo::{self, Posted};
use crate::util::{self, now};

pub struct App {
    db: Mutex<rusqlite::Connection>,
    key: Key,
    pub data_dir: PathBuf,
    pub public_url: String,
    pub insecure_cookies: bool,
    hub: Hub,
    image_sem: Arc<Semaphore>,
    argon_sem: Arc<Semaphore>,
    pub rate: RateLimiter,
    pub started: Instant,
    http: reqwest::Client,
    vapid: Vapid,
}

impl App {
    pub fn open(data_dir: &Path, passphrase: &str, public_url: &str) -> Result<Self, StartupError> {
        if passphrase.is_empty()
            || passphrase.len() > 1024
            || passphrase.chars().any(|c| c == '\n' || c == '\0')
        {
            return Err(StartupError::Message("主密钥为空或格式不正确"));
        }
        std::fs::create_dir_all(data_dir)?;
        let images = files::images_dir(data_dir);
        std::fs::create_dir_all(&images)?;
        restrict_dir(data_dir);
        restrict_dir(&images);
        let salt = load_salt(data_dir)?;
        let key = crypto::Key::derive(passphrase, &salt)
            .map_err(|_| StartupError::Message("主密钥派生失败"))?;
        let conn = db::open_db(&data_dir.join("chat.db"))?;
        verify_key(&conn, &key)?;
        let vapid = load_vapid(&conn, &key)?;
        let http = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(10))
            .build()
            .map_err(|_| StartupError::Message("HTTP 客户端初始化失败"))?;
        Ok(App {
            db: Mutex::new(conn),
            key,
            data_dir: data_dir.to_path_buf(),
            public_url: public_url.trim_end_matches('/').to_string(),
            insecure_cookies: false,
            hub: Hub::default(),
            image_sem: Arc::new(Semaphore::new(1)),
            argon_sem: Arc::new(Semaphore::new(1)),
            rate: RateLimiter::default(),
            started: Instant::now(),
            http,
            vapid,
        })
    }

    pub fn with_db<T>(
        &self,
        f: impl FnOnce(&mut rusqlite::Connection) -> Result<T, AppErr>,
    ) -> Result<T, AppErr> {
        let mut conn = self.db.lock().unwrap_or_else(|err| err.into_inner());
        f(&mut conn)
    }

    pub(crate) async fn hash_pw(&self, password: &str) -> Result<String, AppErr> {
        let _permit = self
            .argon_sem
            .acquire()
            .await
            .map_err(|_| AppErr::Internal)?;
        let password = password.to_string();
        tokio::task::spawn_blocking(move || crypto::hash_password(&password))
            .await
            .map_err(|_| AppErr::Internal)?
    }

    async fn verify_pw(&self, password: &str, encoded: String) -> bool {
        let Ok(_permit) = self.argon_sem.acquire().await else {
            return false;
        };
        let password = password.to_string();
        tokio::task::spawn_blocking(move || crypto::verify_password(&password, &encoded))
            .await
            .unwrap_or(false)
    }

    pub async fn register(
        self: &Arc<Self>,
        token: &str,
        username: &str,
        password: &str,
        display: &str,
        ip: &str,
        ua: &str,
    ) -> Result<(User, String), AppErr> {
        let password = util::clean_password(password).map_err(AppErr::BadRequest)?;
        let username = util::clean_username(username).map_err(AppErr::BadRequest)?;
        let display = util::clean_display(display).map_err(AppErr::BadRequest)?;
        if token.len() < 20 || token.len() > 128 {
            return Err(AppErr::BadRequest("邀请无效"));
        }
        if !self
            .rate
            .allow(&format!("register:ip:{ip}"), 30, Duration::from_secs(900))
        {
            return Err(AppErr::TooMany);
        }
        self.with_db(|conn| repo::invite_usable(conn, token))?;
        let hash = self.hash_pw(password).await?;
        let user = self
            .with_db(|conn| repo::register(conn, &self.key, token, &username, &hash, &display))?;
        let session =
            self.with_db(|conn| repo::create_session(conn, user.id, "app", ip, ua, now()))?;
        Ok((user, session))
    }

    pub async fn login(
        self: &Arc<Self>,
        username: &str,
        password: &str,
        totp_code: Option<&str>,
        admin: bool,
        ip: &str,
        ua: &str,
    ) -> Result<(User, String), AppErr> {
        let password =
            util::clean_password(password).map_err(|_| AppErr::Unauthorized("用户名或密码错误"))?;
        let record = self.with_db(|conn| repo::login_record(conn, username))?;
        let matches = match &record {
            Some(record) => self.verify_pw(password, record.password_hash.clone()).await,
            None => {
                let _ = self.hash_pw(password).await?;
                false
            }
        };
        if !matches {
            return Err(AppErr::Unauthorized("用户名或密码错误"));
        }
        let record = record.ok_or(AppErr::Unauthorized("用户名或密码错误"))?;
        if record.user.disabled {
            return Err(AppErr::Unauthorized("账号已停用"));
        }
        if admin && !record.user.is_admin {
            return Err(AppErr::Forbidden("需要管理员"));
        }
        if admin && !record.user.totp_enabled {
            return Err(AppErr::Forbidden("管理员必须开启两步验证"));
        }
        if record.user.totp_enabled {
            let packed = record.totp_secret.clone().ok_or(AppErr::Internal)?;
            let secret = repo::open_totp(&self.key, &record.user, &packed)?;
            match totp_code {
                None => return Err(AppErr::TotpRequired),
                Some(code) if !crate::totp::verify(&secret, &record.user.username, code)? => {
                    return Err(AppErr::Unauthorized("验证码错误"));
                }
                Some(_) => {}
            }
        }
        let kind = if admin { "admin" } else { "app" };
        let token =
            self.with_db(|conn| repo::create_session(conn, record.user.id, kind, ip, ua, now()))?;
        Ok((record.user, token))
    }

    pub fn logout(&self, session_id: i64, user_id: i64) -> Result<(), AppErr> {
        self.with_db(|conn| repo::delete_session(conn, user_id, session_id))
    }

    pub async fn change_password(
        &self,
        session: &SessionUser,
        old: &str,
        new: &str,
    ) -> Result<(), AppErr> {
        let new = util::clean_password(new).map_err(AppErr::BadRequest)?;
        let old = util::clean_password(old).map_err(|_| AppErr::Unauthorized("原密码错误"))?;
        let record = self
            .with_db(|conn| repo::login_record(conn, &session.user.username))?
            .ok_or(AppErr::Unauthorized("原密码错误"))?;
        if !self.verify_pw(old, record.password_hash).await {
            return Err(AppErr::Unauthorized("原密码错误"));
        }
        let hash = self.hash_pw(new).await?;
        self.with_db(|conn| {
            repo::set_password(conn, session.user.id, &hash)?;
            repo::delete_other_sessions(conn, session.user.id, session.session_id)
        })
    }

    pub fn totp_begin(&self, user: &User) -> Result<(String, String), AppErr> {
        let secret = crate::totp::new_secret();
        let view = crate::totp::provisioning(&secret, &user.username)?;
        self.with_db(|conn| repo::set_totp_pending(conn, &self.key, user, &secret))?;
        Ok(view)
    }

    pub fn totp_enable(&self, user: &User, code: &str) -> Result<(), AppErr> {
        let pending = self
            .with_db(|conn| repo::totp_pending(conn, &self.key, user))?
            .ok_or(AppErr::BadRequest("请先开始设置两步验证"))?;
        if !crate::totp::verify(&pending, &user.username, code)? {
            return Err(AppErr::Unauthorized("验证码错误"));
        }
        self.with_db(|conn| repo::enable_totp(conn, user.id))
    }

    pub async fn totp_disable(
        &self,
        user: &User,
        password: &str,
        code: &str,
    ) -> Result<(), AppErr> {
        if user.is_admin {
            return Err(AppErr::Forbidden("管理员不能关闭两步验证"));
        }
        let password =
            util::clean_password(password).map_err(|_| AppErr::Unauthorized("密码错误"))?;
        let record = self
            .with_db(|conn| repo::login_record(conn, &user.username))?
            .ok_or(AppErr::Unauthorized("密码错误"))?;
        if !self.verify_pw(password, record.password_hash).await {
            return Err(AppErr::Unauthorized("密码错误"));
        }
        let packed = record
            .totp_secret
            .ok_or(AppErr::BadRequest("尚未开启两步验证"))?;
        let secret = repo::open_totp(&self.key, user, &packed)?;
        if !crate::totp::verify(&secret, &user.username, code)? {
            return Err(AppErr::Unauthorized("验证码错误"));
        }
        self.with_db(|conn| repo::disable_totp(conn, user.id))
    }

    pub fn sessions(&self, session: &SessionUser) -> Result<Vec<repo::SessionRow>, AppErr> {
        self.with_db(|conn| repo::list_sessions(conn, session.user.id, "app", session.session_id))
    }

    pub fn revoke_session(&self, user_id: i64, session_id: i64) -> Result<(), AppErr> {
        self.with_db(|conn| repo::delete_session(conn, user_id, session_id))
    }

    pub fn search(&self, user: i64, query: &str) -> Result<Vec<PublicUser>, AppErr> {
        let users = self.with_db(|conn| repo::search_users(conn, user, query))?;
        Ok(users
            .into_iter()
            .map(|user| self.public_user(user))
            .collect())
    }

    pub fn friend_request(&self, from: i64, to: i64) -> Result<bool, AppErr> {
        let accepted = self.with_db(|conn| repo::request_friend(conn, from, to))?;
        self.hub.send_users(
            &[from, to],
            &serde_json::json!({"type":"friend"}).to_string(),
        );
        Ok(accepted)
    }

    pub fn friend_accept(&self, actor: i64, request: i64) -> Result<(), AppErr> {
        let (from, to) = self.with_db(|conn| repo::accept_request(conn, actor, request))?;
        self.hub.send_users(
            &[from, to],
            &serde_json::json!({"type":"friend"}).to_string(),
        );
        Ok(())
    }

    pub fn friend_reject(&self, actor: i64, request: i64) -> Result<(), AppErr> {
        let from = self.with_db(|conn| repo::reject_request(conn, actor, request))?;
        self.hub.send_users(
            &[from, actor],
            &serde_json::json!({"type":"friend"}).to_string(),
        );
        Ok(())
    }

    pub fn friend_delete(&self, actor: i64, other: i64) -> Result<(), AppErr> {
        self.with_db(|conn| repo::delete_friend(conn, actor, other))?;
        self.hub.send_users(
            &[actor, other],
            &serde_json::json!({"type":"friend"}).to_string(),
        );
        Ok(())
    }

    pub fn friends(&self, user: i64) -> Result<Vec<PublicUser>, AppErr> {
        let users = self.with_db(|conn| repo::list_friends(conn, user))?;
        Ok(users
            .into_iter()
            .map(|user| self.public_user(user))
            .collect())
    }

    pub fn requests(&self, user: i64) -> Result<Vec<FriendRequestRow>, AppErr> {
        let mut rows = self.with_db(|conn| repo::list_requests(conn, user))?;
        for row in &mut rows {
            row.user.online = self.hub.is_online(row.user.id);
        }
        Ok(rows)
    }

    fn public_user(&self, user: User) -> PublicUser {
        PublicUser {
            id: user.id,
            uid: user.uid,
            username: user.username,
            display_name: user.display_name,
            online: self.hub.is_online(user.id),
        }
    }

    pub fn conversations(&self, user: i64) -> Result<Vec<ConversationOut>, AppErr> {
        let mut rows = self.with_db(|conn| repo::list_conversations(conn, &self.key, user))?;
        self.fill_online(&mut rows);
        Ok(rows)
    }

    pub fn conversation(&self, user: i64, id: i64) -> Result<ConversationOut, AppErr> {
        let mut row = self.with_db(|conn| repo::conversation(conn, &self.key, user, id))?;
        self.fill_online(std::slice::from_mut(&mut row));
        Ok(row)
    }

    fn fill_online(&self, rows: &mut [ConversationOut]) {
        for row in rows {
            if let Some(members) = row.members.as_mut() {
                for member in members {
                    member.online = self.hub.is_online(member.id);
                }
            }
        }
    }

    pub fn create_dm(&self, user: i64, other: i64) -> Result<i64, AppErr> {
        let id = self.with_db(|conn| repo::create_dm(conn, user, other))?;
        self.hub.send_users(
            &[user, other],
            &serde_json::json!({"type":"conversation","conversation_id": id}).to_string(),
        );
        Ok(id)
    }

    pub fn create_group(&self, user: i64, name: &str, members: &[i64]) -> Result<i64, AppErr> {
        let id = self.with_db(|conn| repo::create_group(conn, &self.key, user, name, members))?;
        let mut notify = members.to_vec();
        notify.push(user);
        self.hub.send_users(
            &notify,
            &serde_json::json!({"type":"conversation","conversation_id": id}).to_string(),
        );
        Ok(id)
    }

    pub fn messages(
        &self,
        user: i64,
        conversation: i64,
        before: Option<i64>,
        limit: i64,
    ) -> Result<Vec<MessageOut>, AppErr> {
        self.with_db(|conn| repo::list_messages(conn, &self.key, user, conversation, before, limit))
    }

    pub fn post_text(
        self: &Arc<Self>,
        user: i64,
        conversation: i64,
        text: &str,
    ) -> Result<MessageOut, AppErr> {
        let posted =
            self.with_db(|conn| repo::post_text(conn, &self.key, user, conversation, text))?;
        self.emit_message(&posted);
        Ok(posted.message)
    }

    pub async fn post_image(
        self: &Arc<Self>,
        user: i64,
        conversation: i64,
        bytes: Vec<u8>,
    ) -> Result<MessageOut, AppErr> {
        if bytes.len() > util::MAX_UPLOAD {
            return Err(AppErr::BadRequest("图片不能超过 25MB"));
        }
        let _permit = self
            .image_sem
            .acquire()
            .await
            .map_err(|_| AppErr::Internal)?;
        let processed = tokio::task::spawn_blocking(move || crate::images::process(&bytes))
            .await
            .map_err(|_| AppErr::Internal)??;
        drop(_permit);
        let full_name = hex::encode(util::random_bytes::<32>());
        let thumb_name = hex::encode(util::random_bytes::<32>());
        let dir = files::images_dir(&self.data_dir);
        files::write_encrypted(&self.key, &dir, &full_name, &processed.full)?;
        if let Err(err) = files::write_encrypted(&self.key, &dir, &thumb_name, &processed.thumb) {
            files::remove_names(&dir, &[full_name]);
            return Err(err);
        }
        let image = repo::NewImage {
            full_name: full_name.clone(),
            thumb_name: thumb_name.clone(),
            width: processed.width as i64,
            height: processed.height as i64,
            bytes: processed.full.len() as i64,
        };
        match self.with_db(|conn| repo::post_image(conn, &self.key, user, conversation, image)) {
            Ok(posted) => {
                self.emit_message(&posted);
                Ok(posted.message)
            }
            Err(err) => {
                files::remove_names(&dir, &[full_name, thumb_name]);
                Err(err)
            }
        }
    }

    pub fn recall(self: &Arc<Self>, user: i64, message: i64) -> Result<MessageOut, AppErr> {
        let (message, files, members) =
            self.with_db(|conn| repo::recall(conn, &self.key, user, message))?;
        files::remove_names(&files::images_dir(&self.data_dir), &files);
        let payload = serde_json::json!({"type":"recall","conversation_id": message.conversation_id, "message_id": message.id}).to_string();
        self.hub.send_users(&members, &payload);
        Ok(message)
    }

    pub fn hide(&self, user: i64, message: i64) -> Result<(), AppErr> {
        self.with_db(|conn| repo::hide_message(conn, user, message))
    }

    pub fn mark_read(&self, user: i64, conversation: i64, message: i64) -> Result<(), AppErr> {
        let members = self.with_db(|conn| repo::mark_read(conn, user, conversation, message))?;
        let payload = serde_json::json!({"type":"read","conversation_id": conversation, "user_id": user, "message_id": message}).to_string();
        self.hub.send_users(&members, &payload);
        Ok(())
    }

    pub fn set_retention(
        &self,
        user: i64,
        conversation: i64,
        retention: i64,
    ) -> Result<(), AppErr> {
        let members =
            self.with_db(|conn| repo::set_retention(conn, user, conversation, retention))?;
        self.touch_conversation(&members, conversation);
        Ok(())
    }

    pub fn rename_group(&self, user: i64, conversation: i64, name: &str) -> Result<(), AppErr> {
        let members =
            self.with_db(|conn| repo::rename_group(conn, &self.key, user, conversation, name))?;
        self.touch_conversation(&members, conversation);
        Ok(())
    }

    pub fn add_member(&self, actor: i64, conversation: i64, target: i64) -> Result<(), AppErr> {
        let members =
            self.with_db(|conn| repo::add_member(conn, &self.key, actor, conversation, target))?;
        self.touch_conversation(&members, conversation);
        Ok(())
    }

    pub fn kick(&self, actor: i64, conversation: i64, target: i64) -> Result<(), AppErr> {
        let members =
            self.with_db(|conn| repo::kick_member(conn, &self.key, actor, conversation, target))?;
        self.touch_conversation(&members, conversation);
        self.hub.send_users(
            &[target],
            &serde_json::json!({"type":"conversation","conversation_id": conversation}).to_string(),
        );
        Ok(())
    }

    pub fn transfer(&self, actor: i64, conversation: i64, target: i64) -> Result<(), AppErr> {
        let members =
            self.with_db(|conn| repo::transfer(conn, &self.key, actor, conversation, target))?;
        self.touch_conversation(&members, conversation);
        Ok(())
    }

    pub fn leave(&self, user: i64, conversation: i64) -> Result<(), AppErr> {
        let (members, files) =
            self.with_db(|conn| repo::leave(conn, &self.key, user, conversation))?;
        files::remove_names(&files::images_dir(&self.data_dir), &files);
        self.touch_conversation(&members, conversation);
        Ok(())
    }

    fn touch_conversation(&self, members: &[i64], conversation: i64) {
        if !members.is_empty() {
            self.hub.send_users(
                members,
                &serde_json::json!({"type":"conversation","conversation_id": conversation})
                    .to_string(),
            );
        }
    }

    fn emit_message(self: &Arc<Self>, posted: &Posted) {
        let payload = serde_json::json!({"type":"message","conversation_id": posted.message.conversation_id, "message": &posted.message}).to_string();
        self.hub.send_users(&posted.members, &payload);
        let sender = posted.message.sender_id.unwrap_or(0);
        let recipients: Vec<i64> = posted
            .members
            .iter()
            .copied()
            .filter(|id| *id != sender)
            .collect();
        self.spawn_push(recipients);
    }

    fn spawn_push(self: &Arc<Self>, users: Vec<i64>) {
        let app = Arc::clone(self);
        tokio::spawn(async move {
            let subs = match app.with_db(|conn| repo::push_subs_for(conn, &users)) {
                Ok(subs) => subs,
                Err(err) => {
                    tracing::warn!(error = err.message(), "读取推送订阅失败");
                    return;
                }
            };
            let sem = Arc::new(Semaphore::new(4));
            let mut tasks = Vec::new();
            for sub in subs {
                if app.hub.is_online(sub.user_id) {
                    continue;
                }
                let Ok(permit) = sem.clone().acquire_owned().await else {
                    continue;
                };
                let app = Arc::clone(&app);
                tasks.push(tokio::spawn(async move {
                    let _permit = permit;
                    let subject = app.vapid_subject();
                    match push::send(&app.http, &app.vapid, &subject, &sub).await {
                        Err(push::PushGone::Remove) => {
                            let _ =
                                app.with_db(|conn| repo::delete_push_endpoint(conn, &sub.endpoint));
                        }
                        Err(push::PushGone::Temporary) => {}
                        Ok(()) => {}
                    }
                }));
            }
            for task in tasks {
                let _ = task.await;
            }
        });
    }

    fn vapid_subject(&self) -> String {
        if self.public_url.starts_with("https://") || self.public_url.starts_with("mailto:") {
            self.public_url.clone()
        } else {
            "mailto:chat-room@localhost".into()
        }
    }

    pub fn vapid_public(&self) -> &str {
        &self.vapid.public_b64
    }

    pub fn save_push(
        &self,
        user: i64,
        endpoint: &str,
        p256dh: &str,
        auth: &str,
    ) -> Result<(), AppErr> {
        self.with_db(|conn| repo::save_push(conn, user, endpoint, p256dh, auth))
    }

    pub fn delete_push(&self, user: i64, endpoint: &str) -> Result<(), AppErr> {
        self.with_db(|conn| repo::delete_push(conn, user, endpoint))
    }

    pub fn read_image(&self, user: i64, image: i64, thumb: bool) -> Result<Vec<u8>, AppErr> {
        let name = self.with_db(|conn| repo::image_name_for_member(conn, user, image, thumb))?;
        files::read_encrypted(&self.key, &files::images_dir(&self.data_dir), &name)
    }

    pub fn hub(&self) -> &Hub {
        &self.hub
    }

    pub fn maintenance(&self) {
        let files_to_remove = self.with_db(|conn| {
            let expired = repo::sweep(conn, now())?;
            let referenced = repo::referenced_images(conn)?;
            Ok::<_, AppErr>((expired, referenced))
        });
        let dir = files::images_dir(&self.data_dir);
        match files_to_remove {
            Ok((expired, referenced)) => {
                files::remove_names(&dir, &expired);
                files::remove_orphans(&dir, &referenced);
            }
            Err(err) => tracing::error!(error = err.message(), "清理过期消息失败"),
        }
    }

    pub fn checkpoint(&self) {
        if let Err(err) = self.with_db(|conn| repo::checkpoint(conn)) {
            tracing::warn!(error = err.message(), "检查点失败");
        }
    }

    pub fn rss_bytes() -> u64 {
        let Ok(text) = std::fs::read_to_string("/proc/self/status") else {
            return 0;
        };
        for line in text.lines() {
            if let Some(rest) = line.strip_prefix("VmRSS:") {
                let kb: u64 = rest
                    .split_whitespace()
                    .next()
                    .unwrap_or("0")
                    .parse()
                    .unwrap_or(0);
                return kb * 1024;
            }
        }
        0
    }

    pub fn disk_bytes(&self) -> u64 {
        dir_size(&self.data_dir)
    }

    pub fn counts(&self) -> Result<(i64, i64, i64, i64), AppErr> {
        self.with_db(|conn| repo::counts(conn))
    }

    pub fn invite_link(&self, token: &str) -> String {
        format!("{}/#/register/{token}", self.public_url)
    }
}

fn dir_size(path: &Path) -> u64 {
    let mut total = 0u64;
    let Ok(entries) = std::fs::read_dir(path) else {
        return 0;
    };
    for entry in entries.flatten() {
        let Ok(meta) = entry.metadata() else { continue };
        if meta.is_dir() {
            total = total.saturating_add(dir_size(&entry.path()));
        } else {
            total = total.saturating_add(meta.len());
        }
    }
    total
}

fn restrict_dir(path: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700));
    }
}

fn load_salt(data_dir: &Path) -> Result<[u8; 16], StartupError> {
    let path = data_dir.join("kdf.salt");
    let db = data_dir.join("chat.db");
    if path.exists() {
        let bytes = std::fs::read(&path)?;
        let salt: [u8; 16] = bytes
            .try_into()
            .map_err(|_| StartupError::Message("盐文件损坏"))?;
        return Ok(salt);
    }
    if db.exists() && std::fs::metadata(&db)?.len() > 0 {
        return Err(StartupError::BadKey);
    }
    let salt = util::random_bytes::<16>();
    std::fs::write(&path, salt)?;
    restrict_file(&path);
    Ok(salt)
}

fn restrict_file(path: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
    }
}

fn verify_key(conn: &rusqlite::Connection, key: &Key) -> Result<(), StartupError> {
    match repo::setting(conn, "kdf_check").map_err(|_| StartupError::Message("无法读取密钥校验"))?
    {
        Some(packed) => {
            if crypto::open_startup(key, b"kdf", &packed) {
                Ok(())
            } else {
                Err(StartupError::BadKey)
            }
        }
        None => {
            let users =
                repo::user_count(conn).map_err(|_| StartupError::Message("无法读取用户"))?;
            if users > 0 {
                return Err(StartupError::BadKey);
            }
            let packed = crypto::seal(key, b"kdf", b"chat-room-ok")
                .map_err(|_| StartupError::Message("无法写入密钥校验"))?;
            repo::set_setting(conn, "kdf_check", &packed)
                .map_err(|_| StartupError::Message("无法写入密钥校验"))?;
            Ok(())
        }
    }
}

fn load_vapid(conn: &rusqlite::Connection, key: &Key) -> Result<Vapid, StartupError> {
    match repo::setting(conn, "vapid_private")
        .map_err(|_| StartupError::Message("无法读取推送密钥"))?
    {
        Some(packed) => Vapid::open(key, &packed).map_err(|_| StartupError::BadKey),
        None => {
            let vapid = Vapid::generate().map_err(|_| StartupError::Message("无法生成推送密钥"))?;
            let packed = vapid
                .seal(key)
                .map_err(|_| StartupError::Message("无法保存推送密钥"))?;
            repo::set_setting(conn, "vapid_private", &packed)
                .map_err(|_| StartupError::Message("无法保存推送密钥"))?;
            Ok(vapid)
        }
    }
}

pub fn create_admin(
    data_dir: &Path,
    passphrase: &str,
    username: &str,
    password: &str,
    display: &str,
    totp_secret: &[u8],
    code: &str,
) -> Result<User, StartupError> {
    let username = util::clean_username(username).map_err(StartupError::Message)?;
    let display = util::clean_display(display).map_err(StartupError::Message)?;
    let password = util::clean_password(password).map_err(StartupError::Message)?;
    if !crate::totp::verify(totp_secret, &username, code)
        .map_err(|_| StartupError::Message("验证码校验失败"))?
    {
        return Err(StartupError::Message("验证码不正确，管理员未创建"));
    }
    let app = App::open(data_dir, passphrase, "https://localhost")?;
    let admins = app
        .with_db(|conn| repo::admin_count(conn))
        .map_err(|_| StartupError::Message("无法读取管理员"))?;
    if admins > 0 {
        return Err(StartupError::Message("已经存在管理员"));
    }
    let hash =
        crypto::hash_password(password).map_err(|_| StartupError::Message("密码处理失败"))?;
    app.with_db(|conn| {
        repo::insert_user(
            conn,
            &app.key,
            &username,
            &hash,
            &display,
            Some(totp_secret),
            true,
            now(),
        )
    })
    .map_err(|err| StartupError::Message(err.message()))
}

pub fn check(data_dir: &Path, passphrase: &str) -> Result<String, StartupError> {
    let app = App::open(data_dir, passphrase, "https://localhost")?;
    let users = app
        .with_db(|conn| repo::user_count(conn))
        .map_err(|_| StartupError::Message("无法读取数据库"))?;
    Ok(format!("密钥可用，当前用户数 {users}"))
}
