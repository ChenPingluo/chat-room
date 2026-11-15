use serde::Serialize;

#[derive(Debug, Clone)]
pub struct User {
    pub id: i64,
    pub uid: String,
    pub username: String,
    pub display_name: String,
    pub is_admin: bool,
    pub disabled: bool,
    pub totp_enabled: bool,
    pub created_at: i64,
}

#[derive(Debug, Clone)]
pub struct SessionUser {
    pub session_id: i64,
    pub user: User,
}

#[derive(Serialize)]
pub struct UserOut {
    pub id: i64,
    pub uid: String,
    pub username: String,
    pub display_name: String,
    pub is_admin: bool,
    pub totp_enabled: bool,
}

impl From<&User> for UserOut {
    fn from(user: &User) -> Self {
        UserOut {
            id: user.id,
            uid: user.uid.clone(),
            username: user.username.clone(),
            display_name: user.display_name.clone(),
            is_admin: user.is_admin,
            totp_enabled: user.totp_enabled,
        }
    }
}

#[derive(Serialize)]
pub struct PublicUser {
    pub id: i64,
    pub uid: String,
    pub username: String,
    pub display_name: String,
    pub online: bool,
}

#[derive(Serialize)]
pub struct MessageOut {
    pub id: i64,
    pub conversation_id: i64,
    pub sender_id: Option<i64>,
    pub sender_username: Option<String>,
    pub sender_display_name: String,
    pub kind: String,
    pub body: Option<String>,
    pub image_id: Option<i64>,
    pub width: Option<i64>,
    pub height: Option<i64>,
    pub recalled: bool,
    pub created_at: i64,
    pub mine: bool,
}

#[derive(Serialize)]
pub struct MemberOut {
    pub id: i64,
    pub uid: String,
    pub username: String,
    pub display_name: String,
    pub role: String,
    pub online: bool,
}

#[derive(Serialize)]
pub struct ConversationOut {
    pub id: i64,
    pub kind: String,
    pub title: String,
    pub retention: i64,
    pub unread: i64,
    pub role: String,
    pub can_send: bool,
    pub last_preview: Option<String>,
    pub last_at: Option<i64>,
    pub members: Option<Vec<MemberOut>>,
}
