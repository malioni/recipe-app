-- Enable WAL mode for concurrent reads with serialized writes.
PRAGMA journal_mode = WAL;

-- Enforce foreign key constraints (SQLite disables these by default).
PRAGMA foreign_keys = ON;

-- Reclaim space from deleted rows on demand via PRAGMA incremental_vacuum.
PRAGMA auto_vacuum = INCREMENTAL;

-- ---------------------------------------------------------------------------
-- Users
-- ---------------------------------------------------------------------------
CREATE TABLE IF NOT EXISTS users (
    id            INTEGER PRIMARY KEY AUTOINCREMENT,
    username      TEXT NOT NULL UNIQUE,
    password_hash TEXT NOT NULL,
    is_admin      INTEGER NOT NULL DEFAULT 0,
    created_at    TEXT NOT NULL DEFAULT (datetime('now'))
);

-- ---------------------------------------------------------------------------
-- Recipes
-- ---------------------------------------------------------------------------
-- ingredients and instructions are stored as JSON arrays. They are always
-- loaded together with the recipe and never queried individually, so
-- normalizing them into separate tables would add complexity with no benefit.
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
-- No UNIQUE constraint on (user_id, date, slot) — multiple recipes per slot
-- are allowed (e.g. a main dish plus sides). portions multiplies ingredient
-- quantities on the shopping list.
CREATE TABLE IF NOT EXISTS meal_plan (
    id        INTEGER PRIMARY KEY AUTOINCREMENT,
    user_id   INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    date      TEXT NOT NULL,
    slot      TEXT NOT NULL CHECK(slot IN ('breakfast', 'lunch', 'dinner')),
    recipe_id INTEGER NOT NULL REFERENCES recipes(id) ON DELETE CASCADE,
    portions  INTEGER NOT NULL DEFAULT 1
);

-- ---------------------------------------------------------------------------
-- Cooked log
-- ---------------------------------------------------------------------------
-- UNIQUE(user_id, date, recipe_id) makes mark-as-cooked idempotent.
CREATE TABLE IF NOT EXISTS cooked_log (
    id        INTEGER PRIMARY KEY AUTOINCREMENT,
    user_id   INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    date      TEXT NOT NULL,
    recipe_id INTEGER NOT NULL REFERENCES recipes(id) ON DELETE CASCADE,
    UNIQUE(user_id, date, recipe_id)
);
