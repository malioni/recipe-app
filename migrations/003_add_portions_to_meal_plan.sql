-- Allow users to specify how many servings they are preparing for each planned
-- meal. The shopping list multiplies ingredient quantities by this value, so a
-- user cooking for 4 people gets four times the ingredients on their list.
-- SQLite supports ADD COLUMN with a DEFAULT so existing rows automatically
-- receive portions = 1 (i.e. no scaling) without a table rebuild.
ALTER TABLE meal_plan ADD COLUMN portions INTEGER NOT NULL DEFAULT 1;
