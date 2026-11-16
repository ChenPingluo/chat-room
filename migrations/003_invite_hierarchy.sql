ALTER TABLE users ADD COLUMN invited_by INTEGER;

UPDATE users
SET invited_by = (
    SELECT i.created_by
    FROM invites i
    WHERE i.used_by = users.id
      AND i.created_by IS NOT NULL
      AND i.created_by > 0
    ORDER BY i.used_at ASC, i.id ASC
    LIMIT 1
)
WHERE invited_by IS NULL;

CREATE INDEX IF NOT EXISTS idx_users_invited_by ON users(invited_by);
CREATE INDEX IF NOT EXISTS idx_invites_created_by ON invites(created_by);
