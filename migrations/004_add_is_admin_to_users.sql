-- Introduce admin accounts for user management (create/delete users, reset
-- passwords). Regular users can only self-manage their own password. Admins
-- additionally access /admin routes to manage all accounts.
ALTER TABLE users ADD COLUMN is_admin INTEGER NOT NULL DEFAULT 0;

-- Promote all pre-existing users to admin so the original seeded account
-- retains admin access after the migration. New accounts created after this
-- migration default to is_admin = 0 (non-admin).
UPDATE users SET is_admin = 1;
