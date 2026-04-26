-- Add portions column to meal_plan.
-- SQLite supports ADD COLUMN with a DEFAULT so existing rows automatically
-- receive portions = 1 without a table rebuild.
ALTER TABLE meal_plan ADD COLUMN portions INTEGER NOT NULL DEFAULT 1;
