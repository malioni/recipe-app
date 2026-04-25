/// Calendar manager responsible for meal planning, cooked log, and shopping list logic.
///
/// This module sits between the network layer and `calendar_storage`. It owns
/// all business logic — validation, date arithmetic, ingredient aggregation —
/// so neither the network handlers nor the storage layer need to care about it.
use sqlx::SqlitePool;
use chrono::NaiveDate;
use crate::model::{CookedEntry, Ingredient, MealEntry, MealSlot};
use crate::calendar_storage;
use crate::storage;
use crate::SINGLE_USER_ID;

/// Maximum number of meal plan entries a single user may have at once.
#[cfg(not(test))]
const MAX_MEAL_PLAN_ENTRIES: usize = 1000;
#[cfg(test)]
const MAX_MEAL_PLAN_ENTRIES: usize = 3;

// ---------------------------------------------------------------------------
// Meal plan
// ---------------------------------------------------------------------------

/// Returns all planned meals within `[start, end]` (inclusive).
///
/// # Errors
///
/// Returns `Err` if the date range is invalid or the query fails.
pub async fn get_meals_in_range(
    pool: &SqlitePool,
    start: NaiveDate,
    end: NaiveDate,
) -> Result<Vec<MealEntry>, String> {
    validate_range(start, end)?;
    calendar_storage::load_meal_entries_in_range(pool, SINGLE_USER_ID, start, end).await
}

/// Plans a recipe for a specific date and slot.
///
/// If a recipe is already planned for that slot it is silently replaced.
///
/// # Errors
///
/// Returns `Err` if the recipe ID does not exist or the query fails.
pub async fn plan_meal(
    pool: &SqlitePool,
    date: NaiveDate,
    slot: MealSlot,
    recipe_id: i64,
) -> Result<(), String> {
    // Verify the recipe exists before linking it.
    storage::load_recipe(pool, recipe_id).await
        .map_err(|_| format!("Recipe with ID {} not found", recipe_id))?;

    // Enforce per-user meal plan quota. Use a large window to count all entries.
    let all_entries = calendar_storage::load_meal_entries_in_range(
        pool, SINGLE_USER_ID,
        chrono::NaiveDate::from_ymd_opt(1970, 1, 1).unwrap(),
        chrono::NaiveDate::from_ymd_opt(9999, 12, 31).unwrap(),
    ).await?;
    if all_entries.len() >= MAX_MEAL_PLAN_ENTRIES {
        return Err(format!("Meal plan limit of {} entries reached", MAX_MEAL_PLAN_ENTRIES));
    }

    let entry = MealEntry { date, slot, recipe_id };
    calendar_storage::add_meal_entry(pool, SINGLE_USER_ID, &entry).await
}

/// Removes the planned meal at the given date and slot.
///
/// # Errors
///
/// Returns `Err` if no entry exists for that date and slot, or the query fails.
pub async fn remove_planned_meal(
    pool: &SqlitePool,
    date: NaiveDate,
    slot: MealSlot,
) -> Result<(), String> {
    calendar_storage::delete_meal_entry(pool, SINGLE_USER_ID, date, slot).await
}

// ---------------------------------------------------------------------------
// Cooked log
// ---------------------------------------------------------------------------

/// Marks a recipe as cooked on the given date.
///
/// Duplicate entries for the same date and recipe are silently ignored.
///
/// # Errors
///
/// Returns `Err` if the recipe ID does not exist or the query fails.
pub async fn mark_as_cooked(
    pool: &SqlitePool,
    date: NaiveDate,
    recipe_id: i64,
) -> Result<(), String> {
    storage::load_recipe(pool, recipe_id).await
        .map_err(|_| format!("Recipe with ID {} not found", recipe_id))?;

    let entry = CookedEntry { date, recipe_id };
    calendar_storage::add_cooked_entry(pool, SINGLE_USER_ID, &entry).await
}

/// Returns all cooked entries within `[start, end]` (inclusive).
///
/// # Errors
///
/// Returns `Err` if the date range is invalid or the query fails.
pub async fn get_cooked_in_range(
    pool: &SqlitePool,
    start: NaiveDate,
    end: NaiveDate,
) -> Result<Vec<CookedEntry>, String> {
    validate_range(start, end)?;
    calendar_storage::load_cooked_entries_in_range(pool, SINGLE_USER_ID, start, end).await
}

// ---------------------------------------------------------------------------
// Shopping list
// ---------------------------------------------------------------------------

/// Aggregates ingredients across all planned meals within `[start, end]`.
///
/// Ingredients with the same name and unit are merged by summing their
/// quantities, so the caller receives a deduplicated shopping list.
///
/// # Errors
///
/// Returns `Err` if the date range is invalid, the query fails, or a
/// referenced recipe no longer exists.
pub async fn get_shopping_list(
    pool: &SqlitePool,
    start: NaiveDate,
    end: NaiveDate,
) -> Result<Vec<Ingredient>, String> {
    validate_range(start, end)?;

    let entries = calendar_storage::load_meal_entries_in_range(
        pool, SINGLE_USER_ID, start, end
    ).await?;

    let mut aggregated: Vec<Ingredient> = Vec::new();

    for entry in entries {
        let recipe = storage::load_recipe(pool, entry.recipe_id).await
            .map_err(|_| format!("Recipe with ID {} not found", entry.recipe_id))?;

        for ingredient in recipe.ingredients {
            match aggregated
                .iter_mut()
                .find(|i| i.name == ingredient.name && i.unit == ingredient.unit)
            {
                Some(existing) => existing.quantity += ingredient.quantity,
                None => aggregated.push(ingredient),
            }
        }
    }

    Ok(aggregated)
}

// ---------------------------------------------------------------------------
// Private helpers
// ---------------------------------------------------------------------------

fn validate_range(start: NaiveDate, end: NaiveDate) -> Result<(), String> {
    if start > end {
        Err(format!(
            "Invalid date range: start ({}) is after end ({})",
            start, end
        ))
    } else {
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    async fn setup() -> SqlitePool {
        let pool = SqlitePool::connect(":memory:").await.unwrap();
        sqlx::query(include_str!("../migrations/001_initial.sql"))
            .execute(&pool).await.unwrap();
        sqlx::query(
            "INSERT INTO users (id, username, password_hash) VALUES (1, 'test', 'placeholder')"
        )
        .execute(&pool).await.unwrap();
        // Recipe with one ingredient so the shopping list has something to return.
        sqlx::query(
            "INSERT INTO recipes (id, user_id, name, ingredients, instructions) \
             VALUES (1, 1, 'Test Recipe', '[{\"name\":\"Flour\",\"quantity\":200.0,\"unit\":\"g\"}]', '[]')"
        )
        .execute(&pool).await.unwrap();
        pool
    }

    #[test]
    fn test_validate_range_valid() {
        let start = NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();
        let end = NaiveDate::from_ymd_opt(2026, 1, 7).unwrap();
        assert!(validate_range(start, end).is_ok());
    }

    #[test]
    fn test_validate_range_same_day() {
        let date = NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();
        assert!(validate_range(date, date).is_ok());
    }

    #[test]
    fn test_validate_range_invalid() {
        let start = NaiveDate::from_ymd_opt(2026, 1, 7).unwrap();
        let end = NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();
        assert!(validate_range(start, end).is_err());
    }

    #[tokio::test]
    async fn test_plan_meal_happy_path() {
        let pool = setup().await;
        let date = NaiveDate::from_ymd_opt(2026, 4, 1).unwrap();
        plan_meal(&pool, date, MealSlot::Lunch, 1).await.expect("plan_meal should succeed");
        let meals = get_meals_in_range(&pool, date, date).await.unwrap();
        assert_eq!(meals.len(), 1);
        assert_eq!(meals[0].recipe_id, 1);
        assert_eq!(meals[0].slot, MealSlot::Lunch);
    }

    #[tokio::test]
    async fn test_plan_meal_invalid_recipe_id() {
        let pool = setup().await;
        let date = NaiveDate::from_ymd_opt(2026, 4, 1).unwrap();
        let result = plan_meal(&pool, date, MealSlot::Lunch, 999_999).await;
        assert!(result.is_err(), "Planning with a non-existent recipe should fail");
    }

    #[tokio::test]
    async fn test_remove_planned_meal() {
        let pool = setup().await;
        let date = NaiveDate::from_ymd_opt(2026, 4, 1).unwrap();
        plan_meal(&pool, date, MealSlot::Dinner, 1).await.unwrap();
        remove_planned_meal(&pool, date, MealSlot::Dinner).await.expect("Remove should succeed");
        let meals = get_meals_in_range(&pool, date, date).await.unwrap();
        assert!(meals.is_empty());
    }

    #[tokio::test]
    async fn test_remove_planned_meal_not_found() {
        let pool = setup().await;
        let date = NaiveDate::from_ymd_opt(2026, 4, 1).unwrap();
        // Removing a non-existent entry is a no-op — idempotent by design.
        assert!(remove_planned_meal(&pool, date, MealSlot::Breakfast).await.is_ok());
    }

    #[tokio::test]
    async fn test_mark_as_cooked_happy_path() {
        let pool = setup().await;
        let date = NaiveDate::from_ymd_opt(2026, 4, 1).unwrap();
        mark_as_cooked(&pool, date, 1).await.expect("mark_as_cooked should succeed");
        let cooked = get_cooked_in_range(&pool, date, date).await.unwrap();
        assert_eq!(cooked.len(), 1);
        assert_eq!(cooked[0].recipe_id, 1);
    }

    #[tokio::test]
    async fn test_mark_as_cooked_invalid_recipe() {
        let pool = setup().await;
        let date = NaiveDate::from_ymd_opt(2026, 4, 1).unwrap();
        let result = mark_as_cooked(&pool, date, 999_999).await;
        assert!(result.is_err(), "Marking a non-existent recipe as cooked should fail");
    }

    #[tokio::test]
    async fn test_get_cooked_in_range_invalid_range() {
        let pool = setup().await;
        let start = NaiveDate::from_ymd_opt(2026, 4, 7).unwrap();
        let end = NaiveDate::from_ymd_opt(2026, 4, 1).unwrap();
        assert!(get_cooked_in_range(&pool, start, end).await.is_err());
    }

    #[tokio::test]
    async fn test_get_shopping_list_empty_range() {
        let pool = setup().await;
        let start = NaiveDate::from_ymd_opt(2099, 1, 1).unwrap();
        let end = NaiveDate::from_ymd_opt(2099, 1, 7).unwrap();
        let list = get_shopping_list(&pool, start, end).await.unwrap();
        assert!(list.is_empty());
    }

    #[tokio::test]
    async fn test_get_shopping_list_returns_ingredients() {
        let pool = setup().await;
        let date = NaiveDate::from_ymd_opt(2026, 4, 1).unwrap();
        plan_meal(&pool, date, MealSlot::Lunch, 1).await.unwrap();
        let list = get_shopping_list(&pool, date, date).await.unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].name, "Flour");
        assert!((list[0].quantity - 200.0).abs() < f32::EPSILON);
    }

    #[tokio::test]
    async fn test_meal_plan_quota_enforced() {
        let pool = setup().await;
        let slots = [MealSlot::Breakfast, MealSlot::Lunch, MealSlot::Dinner];
        let base = NaiveDate::from_ymd_opt(2030, 1, 1).unwrap();
        let mut last_date = base;
        for i in 0..MAX_MEAL_PLAN_ENTRIES {
            let date = base + chrono::Duration::days((i / slots.len()) as i64);
            let slot = slots[i % slots.len()].clone();
            plan_meal(&pool, date, slot, 1).await
                .expect("should succeed within quota");
            last_date = date;
        }
        let overflow_date = last_date + chrono::Duration::days(1);
        let result = plan_meal(&pool, overflow_date, MealSlot::Breakfast, 1).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("limit"));
    }

    #[tokio::test]
    async fn test_get_shopping_list_merges_same_ingredient() {
        let pool = setup().await;
        // Add a second recipe that also has Flour (100g)
        sqlx::query(
            "INSERT INTO recipes (id, user_id, name, ingredients, instructions) \
             VALUES (2, 1, 'Cake', '[{\"name\":\"Flour\",\"quantity\":100.0,\"unit\":\"g\"}]', '[]')"
        )
        .execute(&pool).await.unwrap();

        let date = NaiveDate::from_ymd_opt(2026, 4, 1).unwrap();
        plan_meal(&pool, date, MealSlot::Lunch, 1).await.unwrap();   // 200g Flour
        plan_meal(&pool, date, MealSlot::Dinner, 2).await.unwrap();  // 100g Flour

        let list = get_shopping_list(&pool, date, date).await.unwrap();
        assert_eq!(list.len(), 1, "Same ingredient+unit should be merged into one entry");
        assert!((list[0].quantity - 300.0).abs() < f32::EPSILON, "Quantities should sum to 300g");
    }
}