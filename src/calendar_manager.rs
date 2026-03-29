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
}