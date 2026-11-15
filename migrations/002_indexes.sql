CREATE INDEX IF NOT EXISTS idx_messages_sender ON messages(sender_id);
CREATE INDEX IF NOT EXISTS idx_images_owner ON images(owner_id);
CREATE INDEX IF NOT EXISTS idx_friendships_hi ON friendships(user_hi);
CREATE INDEX IF NOT EXISTS idx_requests_to ON friend_requests(to_user, status);
CREATE INDEX IF NOT EXISTS idx_invites_used_by ON invites(used_by);
