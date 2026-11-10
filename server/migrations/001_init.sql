CREATE TABLE users (
    id INTEGER PRIMARY KEY,
    uid TEXT NOT NULL UNIQUE,
    username TEXT NOT NULL,
    username_lower TEXT NOT NULL UNIQUE,
    password_hash TEXT NOT NULL,
    is_admin INTEGER NOT NULL DEFAULT 0,
    disabled INTEGER NOT NULL DEFAULT 0,
    created_at INTEGER NOT NULL
);

CREATE TABLE invites (
    id INTEGER PRIMARY KEY,
    token_hash TEXT NOT NULL UNIQUE,
    created_by INTEGER NOT NULL REFERENCES users(id),
    expires_at INTEGER NOT NULL,
    used_by INTEGER REFERENCES users(id),
    used_at INTEGER,
    revoked INTEGER NOT NULL DEFAULT 0,
    created_at INTEGER NOT NULL
);

CREATE TABLE sessions (
    id INTEGER PRIMARY KEY,
    token_hash TEXT NOT NULL UNIQUE,
    user_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    created_at INTEGER NOT NULL,
    last_seen INTEGER NOT NULL,
    expires_at INTEGER NOT NULL,
    user_agent TEXT NOT NULL DEFAULT ''
);

CREATE INDEX idx_sessions_user ON sessions(user_id);

CREATE TABLE login_attempts (
    key TEXT PRIMARY KEY,
    failures INTEGER NOT NULL,
    locked_until INTEGER NOT NULL DEFAULT 0,
    updated_at INTEGER NOT NULL
);

CREATE TABLE friend_requests (
    id INTEGER PRIMARY KEY,
    from_user INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    to_user INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    status TEXT NOT NULL CHECK (status IN ('pending', 'accepted', 'rejected')),
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    UNIQUE (from_user, to_user)
);

CREATE INDEX idx_friend_requests_to ON friend_requests(to_user, status);

CREATE TABLE friendships (
    user_a INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    user_b INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    created_at INTEGER NOT NULL,
    PRIMARY KEY (user_a, user_b),
    CHECK (user_a < user_b)
);

CREATE INDEX idx_friendships_b ON friendships(user_b);

CREATE TABLE conversations (
    id INTEGER PRIMARY KEY,
    kind TEXT NOT NULL CHECK (kind IN ('direct', 'group')),
    name TEXT,
    owner_id INTEGER REFERENCES users(id),
    retention_secs INTEGER NOT NULL CHECK (retention_secs IN (3600, 86400, 604800, 2592000)),
    direct_key TEXT UNIQUE,
    created_at INTEGER NOT NULL
);

CREATE TABLE conversation_members (
    conv_id INTEGER NOT NULL REFERENCES conversations(id) ON DELETE CASCADE,
    user_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    role TEXT NOT NULL CHECK (role IN ('owner', 'member')),
    joined_at INTEGER NOT NULL,
    last_read_id INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (conv_id, user_id)
);

CREATE INDEX idx_members_user ON conversation_members(user_id);

CREATE TABLE attachments (
    id INTEGER PRIMARY KEY,
    uploader_id INTEGER NOT NULL REFERENCES users(id),
    file_name TEXT NOT NULL UNIQUE,
    thumb_file_name TEXT NOT NULL,
    mime TEXT NOT NULL,
    size INTEGER NOT NULL,
    width INTEGER NOT NULL,
    height INTEGER NOT NULL,
    message_id INTEGER,
    created_at INTEGER NOT NULL
);

CREATE TABLE messages (
    id INTEGER PRIMARY KEY,
    conv_id INTEGER NOT NULL REFERENCES conversations(id) ON DELETE CASCADE,
    sender_id INTEGER NOT NULL REFERENCES users(id),
    kind TEXT NOT NULL CHECK (kind IN ('text', 'image')),
    body_ct BLOB,
    nonce BLOB,
    attachment_id INTEGER REFERENCES attachments(id),
    created_at INTEGER NOT NULL,
    expires_at INTEGER NOT NULL,
    deleted_at INTEGER
);

CREATE INDEX idx_messages_conv ON messages(conv_id, id);
CREATE INDEX idx_messages_expires ON messages(expires_at);

CREATE TABLE meta (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL
);
