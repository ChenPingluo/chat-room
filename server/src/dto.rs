use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct MeDto {
    pub id: i64,
    pub uid: String,
    pub username: String,
    pub is_admin: bool,
    pub created_at: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct PublicUser {
    pub id: i64,
    pub uid: String,
    pub username: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub relation: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SessionDto {
    pub id: i64,
    pub created_at: i64,
    pub last_seen: i64,
    pub expires_at: i64,
    pub user_agent: String,
    pub current: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct InviteDto {
    pub id: i64,
    pub created_at: i64,
    pub expires_at: i64,
    pub revoked: bool,
    pub used_at: Option<i64>,
    pub used_by: Option<PublicUser>,
}

#[derive(Debug, Clone, Serialize)]
pub struct InviteCreated {
    pub invite: InviteDto,
    pub url: String,
    pub token: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct FriendRequestDto {
    pub id: i64,
    pub from_user: PublicUser,
    pub to_user: PublicUser,
    pub status: String,
    pub created_at: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct FriendRequestLists {
    pub incoming: Vec<FriendRequestDto>,
    pub outgoing: Vec<FriendRequestDto>,
}

#[derive(Debug, Clone, Serialize)]
pub struct MemberDto {
    pub id: i64,
    pub uid: String,
    pub username: String,
    pub role: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct AttachmentDto {
    pub id: i64,
    pub width: i64,
    pub height: i64,
    pub size: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct MessageDto {
    pub id: i64,
    pub conv_id: i64,
    pub sender_id: i64,
    pub kind: String,
    pub body: Option<String>,
    pub attachment: Option<AttachmentDto>,
    pub created_at: i64,
    pub expires_at: i64,
    pub deleted: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub client_msg_id: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ConversationDto {
    pub id: i64,
    pub kind: String,
    pub name: Option<String>,
    pub retention_secs: i64,
    pub my_role: String,
    pub owner_id: Option<i64>,
    pub members: Vec<MemberDto>,
    pub last_message: Option<MessageDto>,
    pub unread: i64,
    pub created_at: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerEvent {
    Ready {
        user_id: i64,
    },
    Message {
        message: MessageDto,
    },
    MessageDeleted {
        conv_id: i64,
        message_id: i64,
    },
    FriendRequest {
        request: FriendRequestDto,
    },
    Friendship {
        user: PublicUser,
        status: String,
    },
    Conversation {
        conversation: ConversationDto,
    },
    Error {
        client_msg_id: Option<String>,
        error: String,
    },
    Pong,
}
