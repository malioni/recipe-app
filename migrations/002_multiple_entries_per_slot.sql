-- Recreate meal_plan without the UNIQUE(user_id, date, slot) constraint.
-- SQLite cannot ALTER TABLE to drop a constraint, so the table must be
-- rebuilt. Existing data is preserved.
ALTER TABLE meal_plan RENAME TO meal_plan_old;

CREATE TABLE meal_plan (
    id        INTEGER PRIMARY KEY AUTOINCREMENT,
    user_id   INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    date      TEXT NOT NULL,
    slot      TEXT NOT NULL CHECK(slot IN ('breakfast', 'lunch', 'dinner')),
    recipe_id INTEGER NOT NULL REFERENCES recipes(id) ON DELETE CASCADE
);

INSERT INTO meal_plan (id, user_id, date, slot, recipe_id)
    SELECT id, user_id, date, slot, recipe_id FROM meal_plan_old;

DROP TABLE meal_plan_old;
