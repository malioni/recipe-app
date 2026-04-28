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
/// Results are ordered by date, then by slot (alphabetical on the stored string).
///
/// # Parameters
///
/// - `pool` — the SQLite connection pool.
/// - `user_id` — the ID of the authenticated user; only their entries are returned.
/// - `start` — the first date of the range to query (inclusive).
/// - `end` — the last date of the range to query (inclusive).
///
/// # Returns
///
/// `Ok(entries)` with all matching meal plan entries. Returns an empty `Vec`
/// if no entries fall in the range.
///
/// # Errors
///
/// Returns `Err` if the query fails or if a stored date or slot string cannot
/// be parsed.
pub async fn load_meal_entries_in_range(
    pool: &SqlitePool,
    user_id: i64,
    start: NaiveDate,
    end: NaiveDate,
) -> Result<Vec<MealEntry>, String> {
    let start_str = start.to_string();
    let end_str = end.to_string();

    let rows = sqlx::query!(
        "SELECT id, date, slot, recipe_id, portions FROM meal_plan
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
            Ok(MealEntry { id: Some(row.id), date, slot, recipe_id: row.recipe_id, portions: row.portions })
        })
        .collect()
}

/// Inserts a meal entry for the given user.
///
/// Multiple entries per slot are allowed; the caller is responsible for
/// enforcing any per-slot limits before calling this function.
///
/// # Parameters
///
/// - `pool` — the SQLite connection pool.
/// - `user_id` — the ID of the authenticated user who owns the entry.
/// - `entry` — the meal entry to insert; `entry.id` is ignored (SQLite assigns
///   the real ID via `AUTOINCREMENT`).
///
/// # Returns
///
/// `Ok(())` on success.
///
/// # Errors
///
/// Returns `Err` if the query fails (e.g. a foreign key constraint violation
/// because `entry.recipe_id` does not exist).
pub async fn add_meal_entry(pool: &SqlitePool, user_id: i64, entry: &MealEntry) -> Result<(), String> {
    let date_str = entry.date.to_string();
    let slot_str = entry.slot.to_string();

    sqlx::query!(
        "INSERT INTO meal_plan (user_id, date, slot, recipe_id, portions) VALUES (?, ?, ?, ?, ?)",
        user_id, date_str, slot_str, entry.recipe_id, entry.portions
    )
    .execute(pool)
    .await
    .map_err(|e| format!("Failed to insert meal entry: {e}"))?;

    Ok(())
}

/// Removes a planned meal entry by its primary key, scoped to the owning user.
///
/// Deleting a non-existent id, or an id owned by a different user, is a
/// no-op — idempotent by design.
///
/// # Parameters
///
/// - `pool` — the SQLite connection pool.
/// - `user_id` — the ID of the authenticated user; only their entries can be deleted.
/// - `id` — the primary key of the meal plan entry to delete.
///
/// # Returns
///
/// `Ok(())` on success, including when no row matched `id` and `user_id`.
///
/// # Errors
///
/// Returns `Err` if the query fails.
pub async fn delete_meal_entry(pool: &SqlitePool, user_id: i64, id: i64) -> Result<(), String> {
    sqlx::query!(
        "DELETE FROM meal_plan WHERE id = ? AND user_id = ?",
        id, user_id
    )
    .execute(pool)
    .await
    .map_err(|e| format!("Failed to delete meal entry: {e}"))?;

    Ok(())
}

/// Returns the number of planned meal entries for the given user, date, and slot.
///
/// Used by the manager layer to enforce the per-slot entry limit before
/// inserting a new entry. Uses `COUNT(*)` so no rows are loaded into memory.
///
/// # Parameters
///
/// - `pool` — the SQLite connection pool.
/// - `user_id` — the ID of the authenticated user.
/// - `date` — the specific day to count entries for.
/// - `slot` — the meal slot (`Breakfast`, `Lunch`, or `Dinner`) to count.
///
/// # Returns
///
/// `Ok(n)` where `n` is the number of entries for `(user_id, date, slot)`.
///
/// # Errors
///
/// Returns `Err` if the query fails.
pub(crate) async fn count_slot_entries(
    pool: &SqlitePool,
    user_id: i64,
    date: NaiveDate,
    slot: &MealSlot,
) -> Result<usize, String> {
    let date_str = date.to_string();
    let slot_str = slot.to_string();

    let row = sqlx::query!(
        "SELECT COUNT(*) as count FROM meal_plan WHERE user_id = ? AND date = ? AND slot = ?",
        user_id, date_str, slot_str
    )
    .fetch_one(pool)
    .await
    .map_err(|e| format!("Failed to count slot entries: {e}"))?;

    Ok(row.count as usize)
}

/// Returns the total number of planned meal entries for the given user across
/// all dates and slots.
///
/// Used by the manager layer to enforce the per-user meal plan quota before
/// inserting a new entry. Uses `COUNT(*)` so no rows are loaded into memory,
/// making it efficient even when a user has many entries.
///
/// # Parameters
///
/// - `pool` — the SQLite connection pool.
/// - `user_id` — the ID of the authenticated user whose entries are counted.
///
/// # Returns
///
/// `Ok(n)` where `n` is the total number of meal plan entries owned by
/// `user_id`.
///
/// # Errors
///
/// Returns `Err` if the query fails.
pub(crate) async fn count_all_meal_entries(pool: &SqlitePool, user_id: i64) -> Result<usize, String> {
    let row = sqlx::query!(
        "SELECT COUNT(*) as count FROM meal_plan WHERE user_id = ?",
        user_id
    )
    .fetch_one(pool)
    .await
    .map_err(|e| format!("Failed to count meal entries: {e}"))?;

    Ok(row.count as usize)
}

// ---------------------------------------------------------------------------
// Cooked log
// ---------------------------------------------------------------------------

/// Loads all cooked entries for a user whose date falls within `[start, end]` (inclusive).
///
/// Results are ordered by date ascending.
///
/// # Parameters
///
/// - `pool` — the SQLite connection pool.
/// - `user_id` — the ID of the authenticated user; only their entries are returned.
/// - `start` — the first date of the range to query (inclusive).
/// - `end` — the last date of the range to query (inclusive).
///
/// # Returns
///
/// `Ok(entries)` with all matching cooked log entries. Returns an empty `Vec`
/// if no entries fall in the range.
///
/// # Errors
///
/// Returns `Err` if the query fails or a stored date string cannot be parsed.
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
/// `INSERT OR IGNORE` and the `UNIQUE(user_id, date, recipe_id)` constraint.
/// Callers need not check for duplicates before calling.
///
/// # Parameters
///
/// - `pool` — the SQLite connection pool.
/// - `user_id` — the ID of the authenticated user who cooked the recipe.
/// - `entry` — the cooked log entry specifying which recipe was cooked and when.
///
/// # Returns
///
/// `Ok(())` on success, including when an identical entry already exists
/// (duplicate is silently ignored).
///
/// # Errors
///
/// Returns `Err` if the query fails for a reason other than a duplicate
/// (e.g. a foreign key violation because `entry.recipe_id` does not exist).
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
            .execute(&pool).await.expect("Failed to run migration 001");
        sqlx::query(include_str!("../migrations/002_multiple_entries_per_slot.sql"))
            .execute(&pool).await.expect("Failed to run migration 002");
        sqlx::query(include_str!("../migrations/003_add_portions_to_meal_plan.sql"))
            .execute(&pool).await.expect("Failed to run migration 003");
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
            id: None,
            date: NaiveDate::from_ymd_opt(2026, 1, 1).unwrap(),
            slot: MealSlot::Lunch,
            recipe_id: 1,
            portions: 1,
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
    async fn test_multiple_entries_same_slot() {
        let pool = setup().await;
        sqlx::query("INSERT INTO recipes (id, user_id, name, ingredients, instructions) VALUES (2, 1, 'Other Recipe', '[]', '[]')")
            .execute(&pool).await.unwrap();

        let date = NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();
        let entry1 = MealEntry { id: None, date, slot: MealSlot::Dinner, recipe_id: 1, portions: 1 };
        let entry2 = MealEntry { id: None, date, slot: MealSlot::Dinner, recipe_id: 2, portions: 1 };

        add_meal_entry(&pool, 1, &entry1).await.unwrap();
        add_meal_entry(&pool, 1, &entry2).await.unwrap();

        let loaded = load_meal_entries_in_range(&pool, 1, date, date).await.unwrap();
        assert_eq!(loaded.len(), 2, "Both entries should persist in the same slot");
        let ids: Vec<i64> = loaded.iter().map(|e| e.recipe_id).collect();
        assert!(ids.contains(&1) && ids.contains(&2));
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
        let entry = MealEntry { id: None, date, slot: MealSlot::Breakfast, recipe_id: 1, portions: 1 };
        add_meal_entry(&pool, 1, &entry).await.unwrap();
        let loaded = load_meal_entries_in_range(&pool, 1, date, date).await.unwrap();
        let id = loaded[0].id.unwrap();
        delete_meal_entry(&pool, 1, id).await.expect("Should delete successfully");
        let loaded = load_meal_entries_in_range(&pool, 1, date, date).await.unwrap();
        assert!(loaded.is_empty());
    }

    #[tokio::test]
    async fn test_delete_meal_entry_not_found() {
        let pool = setup().await;
        // Deleting a non-existent id is a no-op — idempotent by design.
        assert!(delete_meal_entry(&pool, 1, 999_999).await.is_ok());
    }

    #[tokio::test]
    async fn test_delete_by_id_removes_only_target() {
        let pool = setup().await;
        sqlx::query("INSERT INTO recipes (id, user_id, name, ingredients, instructions) VALUES (2, 1, 'Other Recipe', '[]', '[]')")
            .execute(&pool).await.unwrap();

        let date = NaiveDate::from_ymd_opt(2026, 3, 1).unwrap();
        add_meal_entry(&pool, 1, &MealEntry { id: None, date, slot: MealSlot::Lunch, recipe_id: 1, portions: 1 }).await.unwrap();
        add_meal_entry(&pool, 1, &MealEntry { id: None, date, slot: MealSlot::Lunch, recipe_id: 2, portions: 1 }).await.unwrap();

        let loaded = load_meal_entries_in_range(&pool, 1, date, date).await.unwrap();
        assert_eq!(loaded.len(), 2);

        // Delete only the first entry by id
        delete_meal_entry(&pool, 1, loaded[0].id.unwrap()).await.unwrap();

        let remaining = load_meal_entries_in_range(&pool, 1, date, date).await.unwrap();
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].recipe_id, loaded[1].recipe_id);
    }

    #[tokio::test]
    async fn test_load_meal_entries_excludes_out_of_range() {
        let pool = setup().await;
        let in_range = MealEntry {
            id: None,
            date: NaiveDate::from_ymd_opt(2026, 6, 15).unwrap(),
            slot: MealSlot::Lunch,
            recipe_id: 1,
            portions: 1,
        };
        let out_of_range = MealEntry {
            id: None,
            date: NaiveDate::from_ymd_opt(2026, 7, 1).unwrap(),
            slot: MealSlot::Lunch,
            recipe_id: 1,
            portions: 1,
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

    // -------------------------------------------------------------------------
    // count_all_meal_entries
    // -------------------------------------------------------------------------

    #[tokio::test]
    async fn test_count_all_meal_entries_empty() {
        let pool = setup().await;
        let count = count_all_meal_entries(&pool, 1).await.unwrap();
        assert_eq!(count, 0);
    }

    #[tokio::test]
    async fn test_count_all_meal_entries_one() {
        let pool = setup().await;
        let entry = MealEntry {
            id: None,
            date: NaiveDate::from_ymd_opt(2026, 1, 1).unwrap(),
            slot: MealSlot::Lunch,
            recipe_id: 1,
            portions: 1,
        };
        add_meal_entry(&pool, 1, &entry).await.unwrap();
        let count = count_all_meal_entries(&pool, 1).await.unwrap();
        assert_eq!(count, 1);
    }

    #[tokio::test]
    async fn test_count_all_meal_entries_multiple() {
        let pool = setup().await;
        let date = NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();
        add_meal_entry(&pool, 1, &MealEntry { id: None, date, slot: MealSlot::Breakfast, recipe_id: 1, portions: 1 }).await.unwrap();
        add_meal_entry(&pool, 1, &MealEntry { id: None, date, slot: MealSlot::Lunch, recipe_id: 1, portions: 1 }).await.unwrap();
        add_meal_entry(&pool, 1, &MealEntry { id: None, date, slot: MealSlot::Dinner, recipe_id: 1, portions: 1 }).await.unwrap();
        let count = count_all_meal_entries(&pool, 1).await.unwrap();
        assert_eq!(count, 3);
    }

    // -------------------------------------------------------------------------
    // count_slot_entries
    // -------------------------------------------------------------------------

    #[tokio::test]
    async fn test_count_slot_entries_empty() {
        let pool = setup().await;
        let date = NaiveDate::from_ymd_opt(2026, 6, 1).unwrap();
        let count = count_slot_entries(&pool, 1, date, &MealSlot::Lunch).await.unwrap();
        assert_eq!(count, 0);
    }

    #[tokio::test]
    async fn test_count_slot_entries_one() {
        let pool = setup().await;
        let date = NaiveDate::from_ymd_opt(2026, 6, 1).unwrap();
        add_meal_entry(&pool, 1, &MealEntry { id: None, date, slot: MealSlot::Lunch, recipe_id: 1, portions: 1 }).await.unwrap();
        let count = count_slot_entries(&pool, 1, date, &MealSlot::Lunch).await.unwrap();
        assert_eq!(count, 1);
    }

    #[tokio::test]
    async fn test_count_slot_entries_different_slot_not_counted() {
        let pool = setup().await;
        let date = NaiveDate::from_ymd_opt(2026, 6, 1).unwrap();
        add_meal_entry(&pool, 1, &MealEntry { id: None, date, slot: MealSlot::Breakfast, recipe_id: 1, portions: 1 }).await.unwrap();
        // Lunch slot should still be 0
        let count = count_slot_entries(&pool, 1, date, &MealSlot::Lunch).await.unwrap();
        assert_eq!(count, 0);
    }
}