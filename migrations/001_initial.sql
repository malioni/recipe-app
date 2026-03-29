-- Enable WAL mode for concurrent reads with serialized writes.
PRAGMA journal_mode = WAL;

-- Enforce foreign key constraints (SQLite disables these by default).
PRAGMA foreign_keys = ON;

-- Reclaim space from deleted rows on demand via PRAGMA incremental_vacuum.
PRAGMA auto_vacuum = INCREMENTAL;

-- ---------------------------------------------------------------------------
-- Users
-- ---------------------------------------------------------------------------
-- user_id is present on all domain tables from the start so that adding
-- real authentication later requires no schema migration, only replacing
-- the hardcoded SINGLE_USER_ID constant with the session user's ID.
CREATE TABLE IF NOT EXISTS users (
    id            INTEGER PRIMARY KEY AUTOINCREMENT,
    username      TEXT NOT NULL UNIQUE,
    password_hash TEXT NOT NULL,
    created_at    TEXT NOT NULL DEFAULT (datetime('now'))
);

-- ---------------------------------------------------------------------------
-- Recipes
-- ---------------------------------------------------------------------------
-- ingredients and instructions are stored as JSON arrays. They are always
-- loaded together with the recipe and never queried individually, so
-- normalizing them into separate tables would add complexity with no benefit
-- at this stage.
CREATE TABLE IF NOT EXISTS recipes (
    id           INTEGER PRIMARY KEY AUTOINCREMENT,
    user_id      INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    name         TEXT NOT NULL,
    source_url   TEXT,
    ingredients  TEXT NOT NULL DEFAULT '[]',
    instructions TEXT NOT NULL DEFAULT '[]'
);

-- ---------------------------------------------------------------------------
-- Meal plan
-- ---------------------------------------------------------------------------
-- UNIQUE(user_id, date, slot) enforces the "one recipe per slot per day"
-- rule at the database level. ON DELETE CASCADE removes meal plan entries
-- automatically when the referenced recipe or user is deleted.
CREATE TABLE IF NOT EXISTS meal_plan (
    id        INTEGER PRIMARY KEY AUTOINCREMENT,
    user_id   INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    date      TEXT NOT NULL,
    slot      TEXT NOT NULL CHECK(slot IN ('breakfast', 'lunch', 'dinner')),
    recipe_id INTEGER NOT NULL REFERENCES recipes(id) ON DELETE CASCADE,
    UNIQUE(user_id, date, slot)
);

-- ---------------------------------------------------------------------------
-- Cooked log
-- ---------------------------------------------------------------------------
-- UNIQUE(user_id, date, recipe_id) prevents duplicate cooked entries for
-- the same recipe on the same day.
CREATE TABLE IF NOT EXISTS cooked_log (
    id        INTEGER PRIMARY KEY AUTOINCREMENT,
    user_id   INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    date      TEXT NOT NULL,
    recipe_id INTEGER NOT NULL REFERENCES recipes(id) ON DELETE CASCADE,
    UNIQUE(user_id, date, recipe_id)
);