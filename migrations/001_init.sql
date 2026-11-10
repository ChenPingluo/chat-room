CREATE TABLE users (
    id INTEGER PRIMARY KEY,
    uid TEXT NOT NULL UNIQUE,
    username TEXT NOT NULL UNIQUE COLLATE NOCASE,
    username_lower TEXT NOT NULL UNIQUE,
    password_hash TEXT NOT NULL,
    display_name TEXT NOT NULL,
    totp_secret BLOB,
    totp_pending BLOB,
    totp_enabled INTEGER NOT NULL DEFAULT 0,
    is_admin INTEGER NOT NULL DEFAULT 0,
    disabled INTEGER NOT NULL DEFAULT 0,
    created_at INTEGER NOT NULL
);

CREATE TABLE sessions (
    id INTEGER PRIMARY KEY,
    user_id INTEGER NOT NULL,
    token_hash BLOB NOT NULL UNIQUE,
    kind TEXT NOT NULL CHECK (kind IN ('app', 'admin')),
    created_at INTEGER NOT NULL,
    last_seen INTEGER NOT NULL,
    expires_at INTEGER NOT NULL,
    user_agent TEXT NOT NULL DEFAULT '',
    ip TEXT NOT NULL DEFAULT ''
);
CREATE INDEX idx_sessions_user ON sessions(user_id, kind);

CREATE TABLE invites (
    id INTEGER PRIMARY KEY,
    token_hash BLOB NOT NULL UNIQUE,
    created_by INTEGER,
    created_at INTEGER NOT NULL,
    expires_at INTEGER NOT NULL,
    used_by INTEGER,
    used_at INTEGER,
    revoked INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE friend_requests (
    id INTEGER PRIMARY KEY,
    from_user INTEGER NOT NULL,
    to_user INTEGER NOT NULL,
    status TEXT NOT NULL CHECK (status IN ('pending', 'accepted', 'rejected')),
    created_at INTEGER NOT NULL,
    UNIQUE (from_user, to_user)
);

CREATE TABLE friendships (
    user_lo INTEGER NOT NULL,
    user_hi INTEGER NOT NULL,
    created_at INTEGER NOT NULL,
    PRIMARY KEY (user_lo, user_hi),
    CHECK (user_lo < user_hi)
);

CREATE TABLE conversations (
    id INTEGER PRIMARY KEY,
    kind TEXT NOT NULL CHECK (kind IN ('dm', 'group')),
    dm_pair TEXT UNIQUE,
    name_enc BLOB,
    retention_secs INTEGER NOT NULL DEFAULT 2592000
        CHECK (retention_secs IN (3600, 86400, 604800, 2592000)),
    created_by INTEGER,
    created_at INTEGER NOT NULL
);

CREATE TABLE members (
    conversation_id INTEGER NOT NULL,
    user_id INTEGER NOT NULL,
    role TEXT NOT NULL CHECK (role IN ('owner', 'member')),
    joined_at INTEGER NOT NULL,
    last_read_id INTEGER,
    PRIMARY KEY (conversation_id, user_id)
);
CREATE INDEX idx_members_user ON members(user_id);

CREATE TABLE messages (
    id INTEGER PRIMARY KEY,
    conversation_id INTEGER NOT NULL,
    sender_id INTEGER,
    kind TEXT NOT NULL CHECK (kind IN ('text', 'image', 'system')),
    body_enc BLOB,
    image_id INTEGER,
    created_at INTEGER NOT NULL,
    expires_at INTEGER NOT NULL,
    recalled INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX idx_messages_conv ON messages(conversation_id, id);
CREATE INDEX idx_messages_exp ON messages(expires_at);

CREATE TABLE message_hidden (
    message_id INTEGER NOT NULL,
    user_id INTEGER NOT NULL,
    PRIMARY KEY (message_id, user_id)
);

CREATE TABLE images (
    id INTEGER PRIMARY KEY,
    owner_id INTEGER NOT NULL,
    conversation_id INTEGER NOT NULL,
    file_name TEXT NOT NULL UNIQUE,
    thumb_name TEXT NOT NULL UNIQUE,
    width INTEGER NOT NULL,
    height INTEGER NOT NULL,
    bytes INTEGER NOT NULL,
    created_at INTEGER NOT NULL
);

CREATE TABLE push_subs (
    id INTEGER PRIMARY KEY,
    user_id INTEGER NOT NULL,
    endpoint TEXT NOT NULL UNIQUE,
    p256dh TEXT NOT NULL,
    auth TEXT NOT NULL,
    created_at INTEGER NOT NULL
);
CREATE INDEX idx_push_user ON push_subs(user_id);

CREATE TABLE settings (
    key TEXT PRIMARY KEY,
    value BLOB NOT NULL
);

CREATE TABLE audit_log (
    id INTEGER PRIMARY KEY,
    actor_id INTEGER,
    action TEXT NOT NULL,
    detail TEXT NOT NULL DEFAULT '',
    created_at INTEGER NOT NULL
);
CREATE INDEX idx_audit_id ON audit_log(id);
