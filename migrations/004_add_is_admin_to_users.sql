-- Add is_admin column to users. Defaults to 0 (false).
ALTER TABLE users ADD COLUMN is_admin INTEGER NOT NULL DEFAULT 0;

-- Promote all pre-existing users to admin so the original seeded account
-- retains admin access after the migration.
UPDATE users SET is_admin = 1;
