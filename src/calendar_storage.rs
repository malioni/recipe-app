/// Calendar storage module responsible for persisting meal plan and cooked log entries.
///
/// This module owns the SQLite tables `meal_plan` and `cooked_log`.
/// No other module should know about table names or query details.
/// If the backend changes (e.g. SQLite → Postgres), only this file changes.
use sqlx::SqlitePool;
use chrono::NaiveDate;
use crate::model::{CookedEntry, MealEntry, MealSlot};

// ---------------------------------------------------------------------------
// Meal plan
// ---------------------------------------------------------------------------

/// Loads all planned meals for a user whose date falls within `[start, end]` (inclusive).
///
/// # Errors
///
/// Returns `Err` if the query fails.
pub async fn load_meal_entries_in_range(
    pool: &SqlitePool,
    user_id: i64,
    start: NaiveDate,
    end: NaiveDate,
) -> Result<Vec<MealEntry>, String> {
    let start_str = start.to_string();
    let end_str = end.to_string();

    let rows = sqlx::query!(
        "SELECT date, slot, recipe_id FROM meal_plan
         WHERE user_id = ? AND date >= ? AND date <= ?
         ORDER BY date, slot",
        user_id, start_str, end_str
    )
    .fetch_all(pool)
    .await
    .map_err(|e| format!("Failed to query meal entries: {e}"))?;

    rows.into_iter()
        .map(|row| {
            let date = row.date.parse::<NaiveDate>()
                .map_err(|e| format!("Failed to parse date '{}': {e}", row.date))?;
            let slot = parse_slot(&row.slot)?;
            Ok(MealEntry { date, slot, recipe_id: row.recipe_id })
        })
        .collect()
}

/// Inserts or replaces a meal entry for the given user.
///
/// The UNIQUE(user_id, date, slot) constraint combined with INSERT OR REPLACE
/// ensures there is never more than one recipe per slot per day per user.
///
/// # Errors
///
/// Returns `Err` if the query fails.
pub async fn add_meal_entry(pool: &SqlitePool, user_id: i64, entry: &MealEntry) -> Result<(), String> {
    let date_str = entry.date.to_string();
    let slot_str = entry.slot.to_string();

    sqlx::query!(
        "INSERT OR REPLACE INTO meal_plan (user_id, date, slot, recipe_id) VALUES (?, ?, ?, ?)",
        user_id, date_str, slot_str, entry.recipe_id
    )
    .execute(pool)
    .await
    .map_err(|e| format!("Failed to insert meal entry: {e}"))?;

    Ok(())
}

/// Removes the planned meal for the given user, date, and slot.
///
/// # Errors
///
/// Returns `Err` if no entry exists for that date and slot, or the query fails.
pub async fn delete_meal_entry(
    pool: &SqlitePool,
    user_id: i64,
    date: NaiveDate,
    slot: MealSlot,
) -> Result<(), String> {
    let date_str = date.to_string();
    let slot_str = slot.to_string();

    sqlx::query!(
        "DELETE FROM meal_plan WHERE user_id = ? AND date = ? AND slot = ?",
        user_id, date_str, slot_str
    )
    .execute(pool)
    .await
    .map_err(|e| format!("Failed to delete meal entry: {e}"))?;

    Ok(())
}

// ---------------------------------------------------------------------------
// Cooked log
// ---------------------------------------------------------------------------

/// Loads all cooked entries for a user whose date falls within `[start, end]` (inclusive).
///
/// # Errors
///
/// Returns `Err` if the query fails.
pub async fn load_cooked_entries_in_range(
    pool: &SqlitePool,
    user_id: i64,
    start: NaiveDate,
    end: NaiveDate,
) -> Result<Vec<CookedEntry>, String> {
    let start_str = start.to_string();
    let end_str = end.to_string();

    let rows = sqlx::query!(
        "SELECT date, recipe_id FROM cooked_log
         WHERE user_id = ? AND date >= ? AND date <= ?
         ORDER BY date",
        user_id, start_str, end_str
    )
    .fetch_all(pool)
    .await
    .map_err(|e| format!("Failed to query cooked entries: {e}"))?;

    rows.into_iter()
        .map(|row| {
            let date = row.date.parse::<NaiveDate>()
                .map_err(|e| format!("Failed to parse date '{}': {e}", row.date))?;
            Ok(CookedEntry { date, recipe_id: row.recipe_id })
        })
        .collect()
}

/// Records a recipe as cooked on the given date for the given user.
///
/// Duplicate entries (same user, date, recipe) are silently ignored via
/// INSERT OR IGNORE and the UNIQUE constraint.
///
/// # Errors
///
/// Returns `Err` if the query fails.
pub async fn add_cooked_entry(pool: &SqlitePool, user_id: i64, entry: &CookedEntry) -> Result<(), String> {
    let date_str = entry.date.to_string();

    sqlx::query!(
        "INSERT OR IGNORE INTO cooked_log (user_id, date, recipe_id) VALUES (?, ?, ?)",
        user_id, date_str, entry.recipe_id
    )
    .execute(pool)
    .await
    .map_err(|e| format!("Failed to insert cooked entry: {e}"))?;

    Ok(())
}

// ---------------------------------------------------------------------------
// Private helpers
// ---------------------------------------------------------------------------

fn parse_slot(s: &str) -> Result<MealSlot, String> {
    match s {
        "breakfast" => Ok(MealSlot::Breakfast),
        "lunch"     => Ok(MealSlot::Lunch),
        "dinner"    => Ok(MealSlot::Dinner),
        other       => Err(format!("Unknown meal slot: '{}'", other)),
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    async fn setup() -> SqlitePool {
        let pool = SqlitePool::connect(":memory:")
            .await
            .expect("Failed to create in-memory database");
        sqlx::query(include_str!("../migrations/001_initial.sql"))
            .execute(&pool)
            .await
            .expect("Failed to run migrations");
        sqlx::query("INSERT INTO users (id, username, password_hash) VALUES (1, 'test', 'placeholder')")
            .execute(&pool)
            .await
            .expect("Failed to insert test user");
        sqlx::query("INSERT INTO recipes (id, user_id, name, ingredients, instructions) VALUES (1, 1, 'Test Recipe', '[]', '[]')")
            .execute(&pool)
            .await
            .expect("Failed to insert test recipe");
        pool
    }

    #[tokio::test]
    async fn test_add_and_load_meal_entry() {
        let pool = setup().await;
        let entry = MealEntry {
            date: NaiveDate::from_ymd_opt(2026, 1, 1).unwrap(),
            slot: MealSlot::Lunch,
            recipe_id: 1,
        };
        add_meal_entry(&pool, 1, &entry).await.expect("Failed to add meal entry");

        let loaded = load_meal_entries_in_range(
            &pool, 1,
            NaiveDate::from_ymd_opt(2026, 1, 1).unwrap(),
            NaiveDate::from_ymd_opt(2026, 1, 1).unwrap(),
        ).await.expect("Failed to load meal entries");

        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].slot, MealSlot::Lunch);
    }

    #[tokio::test]
    async fn test_meal_entry_replace_on_same_slot() {
        let pool = setup().await;
        // Add a second recipe to replace with.
        sqlx::query("INSERT INTO recipes (id, user_id, name, ingredients, instructions) VALUES (2, 1, 'Other Recipe', '[]', '[]')")
            .execute(&pool).await.unwrap();

        let date = NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();
        let entry1 = MealEntry { date, slot: MealSlot::Dinner, recipe_id: 1 };
        let entry2 = MealEntry { date, slot: MealSlot::Dinner, recipe_id: 2 };

        add_meal_entry(&pool, 1, &entry1).await.unwrap();
        add_meal_entry(&pool, 1, &entry2).await.unwrap(); // should replace entry1

        let loaded = load_meal_entries_in_range(&pool, 1, date, date).await.unwrap();
        assert_eq!(loaded.len(), 1, "Expected only one entry per slot");
        assert_eq!(loaded[0].recipe_id, 2, "Expected the newer entry to win");
    }

    #[tokio::test]
    async fn test_add_cooked_entry_no_duplicates() {
        let pool = setup().await;
        let entry = CookedEntry {
            date: NaiveDate::from_ymd_opt(2026, 1, 1).unwrap(),
            recipe_id: 1,
        };
        add_cooked_entry(&pool, 1, &entry).await.expect("First insert failed");
        add_cooked_entry(&pool, 1, &entry).await.expect("Duplicate insert should be silently ignored");

        let loaded = load_cooked_entries_in_range(
            &pool, 1,
            NaiveDate::from_ymd_opt(2026, 1, 1).unwrap(),
            NaiveDate::from_ymd_opt(2026, 1, 1).unwrap(),
        ).await.expect("Failed to load cooked entries");

        assert_eq!(loaded.len(), 1, "Expected duplicate to be ignored");
    }

    #[tokio::test]
    async fn test_delete_meal_entry() {
        let pool = setup().await;
        let date = NaiveDate::from_ymd_opt(2026, 3, 1).unwrap();
        let entry = MealEntry { date, slot: MealSlot::Breakfast, recipe_id: 1 };
        add_meal_entry(&pool, 1, &entry).await.unwrap();
        delete_meal_entry(&pool, 1, date, MealSlot::Breakfast).await.expect("Should delete successfully");
        let loaded = load_meal_entries_in_range(&pool, 1, date, date).await.unwrap();
        assert!(loaded.is_empty());
    }

    #[tokio::test]
    async fn test_delete_meal_entry_not_found() {
        let pool = setup().await;
        let date = NaiveDate::from_ymd_opt(2026, 3, 1).unwrap();
        // Deleting a non-existent entry is a no-op — idempotent by design.
        assert!(delete_meal_entry(&pool, 1, date, MealSlot::Dinner).await.is_ok());
    }

    #[tokio::test]
    async fn test_load_meal_entries_excludes_out_of_range() {
        let pool = setup().await;
        let in_range = MealEntry {
            date: NaiveDate::from_ymd_opt(2026, 6, 15).unwrap(),
            slot: MealSlot::Lunch,
            recipe_id: 1,
        };
        let out_of_range = MealEntry {
            date: NaiveDate::from_ymd_opt(2026, 7, 1).unwrap(),
            slot: MealSlot::Lunch,
            recipe_id: 1,
        };
        add_meal_entry(&pool, 1, &in_range).await.unwrap();
        add_meal_entry(&pool, 1, &out_of_range).await.unwrap();

        let loaded = load_meal_entries_in_range(
            &pool, 1,
            NaiveDate::from_ymd_opt(2026, 6, 1).unwrap(),
            NaiveDate::from_ymd_opt(2026, 6, 30).unwrap(),
        ).await.unwrap();
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].date, in_range.date);
    }

    #[tokio::test]
    async fn test_load_cooked_entries_range_filtering() {
        let pool = setup().await;
        let in_range = CookedEntry {
            date: NaiveDate::from_ymd_opt(2026, 6, 10).unwrap(),
            recipe_id: 1,
        };
        let out_of_range = CookedEntry {
            date: NaiveDate::from_ymd_opt(2026, 7, 10).unwrap(),
            recipe_id: 1,
        };
        add_cooked_entry(&pool, 1, &in_range).await.unwrap();
        add_cooked_entry(&pool, 1, &out_of_range).await.unwrap();

        let loaded = load_cooked_entries_in_range(
            &pool, 1,
            NaiveDate::from_ymd_opt(2026, 6, 1).unwrap(),
            NaiveDate::from_ymd_opt(2026, 6, 30).unwrap(),
        ).await.unwrap();
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].date, in_range.date);
    }
}