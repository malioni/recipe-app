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

/// Maximum number of entries allowed per (user, date, slot) combination.
#[cfg(not(test))]
const MAX_ENTRIES_PER_SLOT: usize = 3;
#[cfg(test)]
const MAX_ENTRIES_PER_SLOT: usize = 2;

/// Maximum number of portions allowed per meal plan entry.
const MAX_PORTIONS: i64 = 10;

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
/// Multiple entries per slot are allowed up to `MAX_ENTRIES_PER_SLOT`.
///
/// # Errors
///
/// Returns `Err` if the recipe ID does not exist, a quota is exceeded, or the query fails.
pub async fn plan_meal(
    pool: &SqlitePool,
    date: NaiveDate,
    slot: MealSlot,
    recipe_id: i64,
    portions: i64,
) -> Result<(), String> {
    if !(1..=MAX_PORTIONS).contains(&portions) {
        return Err(format!("portions must be between 1 and {MAX_PORTIONS}"));
    }

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

    // Enforce per-slot limit.
    let slot_count = calendar_storage::count_slot_entries(pool, SINGLE_USER_ID, date, &slot).await?;
    if slot_count >= MAX_ENTRIES_PER_SLOT {
        return Err(format!("Slot limit of {} entries reached", MAX_ENTRIES_PER_SLOT));
    }

    let entry = MealEntry { id: None, date, slot, recipe_id, portions };
    calendar_storage::add_meal_entry(pool, SINGLE_USER_ID, &entry).await
}

/// Removes a planned meal entry by its primary key.
///
/// Deleting a non-existent id is a no-op — idempotent by design.
///
/// # Errors
///
/// Returns `Err` if the query fails.
pub async fn remove_planned_meal(pool: &SqlitePool, id: i64) -> Result<(), String> {
    calendar_storage::delete_meal_entry(pool, SINGLE_USER_ID, id).await
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
/// Ingredients with the same name (case-insensitive) and physical dimension
/// are merged by converting all quantities to a canonical unit and summing.
/// Weight units (g, kg, oz, lb) all normalise to `g`; volume units (ml, l,
/// tsp, tbsp, cup) all normalise to `ml`. Units from different physical
/// dimensions (e.g. weight vs. volume) are kept as separate entries.
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

        let scale = entry.portions as f32;
        for ingredient in recipe.ingredients {
            let (canonical_unit, scaled_qty) = normalise_unit(&ingredient.unit, ingredient.quantity * scale);
            match aggregated
                .iter_mut()
                .find(|i| i.name.to_lowercase() == ingredient.name.to_lowercase() && i.unit == canonical_unit)
            {
                Some(existing) => existing.quantity += scaled_qty,
                None => aggregated.push(Ingredient {
                    name: ingredient.name.to_lowercase(),
                    quantity: scaled_qty,
                    unit: canonical_unit,
                }),
            }
        }
    }

    Ok(aggregated)
}

/// Maps a unit string to its canonical form and scales the quantity accordingly.
///
/// All weight units normalise to `"g"`; all volume units normalise to `"ml"`.
/// Unrecognised units are returned unchanged so ingredients with exotic or
/// count-based units (e.g. "clove", "piece") still aggregate by exact match.
fn normalise_unit(unit: &str, quantity: f32) -> (String, f32) {
    match unit.to_lowercase().as_str() {
        // Weight → g
        "g"                              => ("g".to_string(),  quantity),
        "kg"                             => ("g".to_string(),  quantity * 1_000.0),
        "oz" | "ounce" | "ounces"        => ("g".to_string(),  quantity * 28.3495),
        "lb" | "lbs" | "pound" | "pounds" => ("g".to_string(), quantity * 453.592),
        // Volume → ml
        "ml"                                         => ("ml".to_string(), quantity),
        "l" | "liter" | "litre"                      => ("ml".to_string(), quantity * 1_000.0),
        "tsp" | "teaspoon" | "teaspoons"             => ("ml".to_string(), quantity * 4.92892),
        "tbsp" | "tablespoon" | "tablespoons"        => ("ml".to_string(), quantity * 14.7868),
        "cup" | "cups"                               => ("ml".to_string(), quantity * 236.588),
        // Unknown — pass through unchanged
        _ => (unit.to_string(), quantity),
    }
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
        sqlx::query(include_str!("../migrations/002_multiple_entries_per_slot.sql"))
            .execute(&pool).await.unwrap();
        sqlx::query(include_str!("../migrations/003_add_portions_to_meal_plan.sql"))
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
        plan_meal(&pool, date, MealSlot::Lunch, 1, 1).await.expect("plan_meal should succeed");
        let meals = get_meals_in_range(&pool, date, date).await.unwrap();
        assert_eq!(meals.len(), 1);
        assert_eq!(meals[0].recipe_id, 1);
        assert_eq!(meals[0].slot, MealSlot::Lunch);
    }

    #[tokio::test]
    async fn test_plan_meal_invalid_recipe_id() {
        let pool = setup().await;
        let date = NaiveDate::from_ymd_opt(2026, 4, 1).unwrap();
        let result = plan_meal(&pool, date, MealSlot::Lunch, 999_999, 1).await;
        assert!(result.is_err(), "Planning with a non-existent recipe should fail");
    }

    #[tokio::test]
    async fn test_remove_planned_meal() {
        let pool = setup().await;
        let date = NaiveDate::from_ymd_opt(2026, 4, 1).unwrap();
        plan_meal(&pool, date, MealSlot::Dinner, 1, 1).await.unwrap();
        let meals = get_meals_in_range(&pool, date, date).await.unwrap();
        let id = meals[0].id.unwrap();
        remove_planned_meal(&pool, id).await.expect("Remove should succeed");
        let meals = get_meals_in_range(&pool, date, date).await.unwrap();
        assert!(meals.is_empty());
    }

    #[tokio::test]
    async fn test_remove_planned_meal_not_found() {
        let pool = setup().await;
        // Removing a non-existent id is a no-op — idempotent by design.
        assert!(remove_planned_meal(&pool, 999_999).await.is_ok());
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
        plan_meal(&pool, date, MealSlot::Lunch, 1, 1).await.unwrap();
        let list = get_shopping_list(&pool, date, date).await.unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].name, "flour");
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
            plan_meal(&pool, date, slot, 1, 1).await
                .expect("should succeed within quota");
            last_date = date;
        }
        let overflow_date = last_date + chrono::Duration::days(1);
        let result = plan_meal(&pool, overflow_date, MealSlot::Breakfast, 1, 1).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("limit"));
    }

    #[tokio::test]
    async fn test_slot_quota_enforced() {
        let pool = setup().await;
        let date = NaiveDate::from_ymd_opt(2026, 4, 1).unwrap();
        for _ in 0..MAX_ENTRIES_PER_SLOT {
            plan_meal(&pool, date, MealSlot::Dinner, 1, 1).await
                .expect("should succeed within slot quota");
        }
        let result = plan_meal(&pool, date, MealSlot::Dinner, 1, 1).await;
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
        plan_meal(&pool, date, MealSlot::Lunch, 1, 1).await.unwrap();   // 200g Flour
        plan_meal(&pool, date, MealSlot::Dinner, 2, 1).await.unwrap();  // 100g Flour

        let list = get_shopping_list(&pool, date, date).await.unwrap();
        assert_eq!(list.len(), 1, "Same ingredient+unit should be merged into one entry");
        assert!((list[0].quantity - 300.0).abs() < f32::EPSILON, "Quantities should sum to 300g");
    }

    #[tokio::test]
    async fn test_plan_meal_portions_scaling() {
        let pool = setup().await;
        let date = NaiveDate::from_ymd_opt(2026, 4, 1).unwrap();
        // Recipe has 200g Flour; 2 portions should give 400g in shopping list.
        plan_meal(&pool, date, MealSlot::Lunch, 1, 2).await.unwrap();
        let list = get_shopping_list(&pool, date, date).await.unwrap();
        assert_eq!(list.len(), 1);
        assert!((list[0].quantity - 400.0).abs() < f32::EPSILON, "2 portions should double the quantity");
    }

    #[tokio::test]
    async fn test_plan_meal_portions_invalid() {
        let pool = setup().await;
        let date = NaiveDate::from_ymd_opt(2026, 4, 1).unwrap();
        assert!(plan_meal(&pool, date, MealSlot::Lunch, 1, 0).await.is_err(), "0 portions should fail");
        assert!(plan_meal(&pool, date, MealSlot::Lunch, 1, 11).await.is_err(), "11 portions should fail");
    }

    #[tokio::test]
    async fn test_get_shopping_list_case_insensitive_name() {
        let pool = setup().await;
        // Recipe 1 has "Flour" 200g (from setup), recipe 2 has "flour" 100g
        sqlx::query(
            "INSERT INTO recipes (id, user_id, name, ingredients, instructions) \
             VALUES (2, 1, 'Cake', '[{\"name\":\"flour\",\"quantity\":100.0,\"unit\":\"g\"}]', '[]')"
        )
        .execute(&pool).await.unwrap();

        let date = NaiveDate::from_ymd_opt(2026, 4, 2).unwrap();
        plan_meal(&pool, date, MealSlot::Breakfast, 1, 1).await.unwrap();
        plan_meal(&pool, date, MealSlot::Dinner, 2, 1).await.unwrap();

        let list = get_shopping_list(&pool, date, date).await.unwrap();
        assert_eq!(list.len(), 1, "\"Flour\" and \"flour\" should merge");
        assert_eq!(list[0].name, "flour", "Output name should be lowercased");
        assert!((list[0].quantity - 300.0).abs() < 0.001);
    }

    #[tokio::test]
    async fn test_get_shopping_list_merges_lb_and_oz() {
        let pool = setup().await;
        sqlx::query(
            "INSERT INTO recipes (id, user_id, name, ingredients, instructions) \
             VALUES (2, 1, 'Bread', '[{\"name\":\"Butter\",\"quantity\":1.0,\"unit\":\"lb\"}]', '[]')"
        )
        .execute(&pool).await.unwrap();
        sqlx::query(
            "INSERT INTO recipes (id, user_id, name, ingredients, instructions) \
             VALUES (3, 1, 'Cake', '[{\"name\":\"Butter\",\"quantity\":8.0,\"unit\":\"oz\"}]', '[]')"
        )
        .execute(&pool).await.unwrap();

        let date = NaiveDate::from_ymd_opt(2026, 4, 3).unwrap();
        plan_meal(&pool, date, MealSlot::Lunch, 2, 1).await.unwrap();   // 1 lb Butter
        plan_meal(&pool, date, MealSlot::Dinner, 3, 1).await.unwrap();  // 8 oz Butter

        let list = get_shopping_list(&pool, date, date).await.unwrap();
        let butter: Vec<_> = list.iter().filter(|i| i.name == "butter").collect();
        assert_eq!(butter.len(), 1, "lb and oz should merge into one entry");
        assert_eq!(butter[0].unit, "g");
        let expected = 453.592 + 8.0 * 28.3495; // ≈ 680.388
        assert!((butter[0].quantity - expected).abs() < 0.01);
    }

    #[tokio::test]
    async fn test_get_shopping_list_merges_kg_and_g() {
        let pool = setup().await;
        sqlx::query(
            "INSERT INTO recipes (id, user_id, name, ingredients, instructions) \
             VALUES (2, 1, 'Syrup', '[{\"name\":\"Sugar\",\"quantity\":0.5,\"unit\":\"kg\"}]', '[]')"
        )
        .execute(&pool).await.unwrap();
        sqlx::query(
            "INSERT INTO recipes (id, user_id, name, ingredients, instructions) \
             VALUES (3, 1, 'Cookie', '[{\"name\":\"Sugar\",\"quantity\":200.0,\"unit\":\"g\"}]', '[]')"
        )
        .execute(&pool).await.unwrap();

        let date = NaiveDate::from_ymd_opt(2026, 4, 4).unwrap();
        plan_meal(&pool, date, MealSlot::Lunch, 2, 1).await.unwrap();
        plan_meal(&pool, date, MealSlot::Dinner, 3, 1).await.unwrap();

        let list = get_shopping_list(&pool, date, date).await.unwrap();
        let sugar: Vec<_> = list.iter().filter(|i| i.name == "sugar").collect();
        assert_eq!(sugar.len(), 1, "kg and g should merge");
        assert_eq!(sugar[0].unit, "g");
        assert!((sugar[0].quantity - 700.0).abs() < 0.001);
    }

    #[tokio::test]
    async fn test_get_shopping_list_merges_oz_and_g() {
        let pool = setup().await;
        sqlx::query(
            "INSERT INTO recipes (id, user_id, name, ingredients, instructions) \
             VALUES (2, 1, 'Soup', '[{\"name\":\"Salt\",\"quantity\":2.0,\"unit\":\"oz\"}]', '[]')"
        )
        .execute(&pool).await.unwrap();
        sqlx::query(
            "INSERT INTO recipes (id, user_id, name, ingredients, instructions) \
             VALUES (3, 1, 'Stew', '[{\"name\":\"Salt\",\"quantity\":10.0,\"unit\":\"g\"}]', '[]')"
        )
        .execute(&pool).await.unwrap();

        let date = NaiveDate::from_ymd_opt(2026, 4, 5).unwrap();
        plan_meal(&pool, date, MealSlot::Lunch, 2, 1).await.unwrap();
        plan_meal(&pool, date, MealSlot::Dinner, 3, 1).await.unwrap();

        let list = get_shopping_list(&pool, date, date).await.unwrap();
        let salt: Vec<_> = list.iter().filter(|i| i.name == "salt").collect();
        assert_eq!(salt.len(), 1, "oz and g should merge");
        assert_eq!(salt[0].unit, "g");
        let expected = 2.0 * 28.3495 + 10.0; // ≈ 66.699
        assert!((salt[0].quantity - expected).abs() < 0.01);
    }

    #[tokio::test]
    async fn test_get_shopping_list_merges_volume_units() {
        let pool = setup().await;
        sqlx::query(
            "INSERT INTO recipes (id, user_id, name, ingredients, instructions) \
             VALUES (2, 1, 'Tea', '[{\"name\":\"Water\",\"quantity\":0.5,\"unit\":\"l\"}]', '[]')"
        )
        .execute(&pool).await.unwrap();
        sqlx::query(
            "INSERT INTO recipes (id, user_id, name, ingredients, instructions) \
             VALUES (3, 1, 'Sauce', '[{\"name\":\"Water\",\"quantity\":2.0,\"unit\":\"tbsp\"},{\"name\":\"Water\",\"quantity\":3.0,\"unit\":\"tsp\"}]', '[]')"
        )
        .execute(&pool).await.unwrap();

        let date = NaiveDate::from_ymd_opt(2026, 4, 6).unwrap();
        plan_meal(&pool, date, MealSlot::Lunch, 2, 1).await.unwrap();   // 0.5 l
        plan_meal(&pool, date, MealSlot::Dinner, 3, 1).await.unwrap();  // 2 tbsp + 3 tsp

        let list = get_shopping_list(&pool, date, date).await.unwrap();
        let water: Vec<_> = list.iter().filter(|i| i.name == "water").collect();
        assert_eq!(water.len(), 1, "l, tbsp, and tsp should all merge");
        assert_eq!(water[0].unit, "ml");
        let expected = 500.0 + 2.0 * 14.7868 + 3.0 * 4.92892; // ≈ 544.337
        assert!((water[0].quantity - expected).abs() < 0.01);
    }

    #[tokio::test]
    async fn test_get_shopping_list_weight_and_volume_stay_separate() {
        let pool = setup().await;
        // Recipe 1 already has Flour 200g; add a recipe with Flour 100ml
        sqlx::query(
            "INSERT INTO recipes (id, user_id, name, ingredients, instructions) \
             VALUES (2, 1, 'Batter', '[{\"name\":\"Flour\",\"quantity\":100.0,\"unit\":\"ml\"}]', '[]')"
        )
        .execute(&pool).await.unwrap();

        let date = NaiveDate::from_ymd_opt(2026, 4, 7).unwrap();
        plan_meal(&pool, date, MealSlot::Lunch, 1, 1).await.unwrap();   // 200g Flour
        plan_meal(&pool, date, MealSlot::Dinner, 2, 1).await.unwrap();  // 100ml Flour

        let list = get_shopping_list(&pool, date, date).await.unwrap();
        let flour: Vec<_> = list.iter().filter(|i| i.name == "flour").collect();
        assert_eq!(flour.len(), 2, "Weight and volume must not be merged");
        let units: Vec<&str> = flour.iter().map(|i| i.unit.as_str()).collect();
        assert!(units.contains(&"g") && units.contains(&"ml"));
    }

    #[test]
    fn test_normalise_unit() {
        // Weight → g
        let (u, q) = normalise_unit("g", 100.0);
        assert_eq!(u, "g"); assert!((q - 100.0).abs() < 0.001);

        let (u, q) = normalise_unit("kg", 1.0);
        assert_eq!(u, "g"); assert!((q - 1000.0).abs() < 0.001);

        let (u, q) = normalise_unit("oz", 1.0);
        assert_eq!(u, "g"); assert!((q - 28.3495).abs() < 0.001);

        let (u, q) = normalise_unit("Oz", 1.0); // case-insensitive
        assert_eq!(u, "g"); assert!((q - 28.3495).abs() < 0.001);

        let (u, q) = normalise_unit("lb", 1.0);
        assert_eq!(u, "g"); assert!((q - 453.592).abs() < 0.001);

        let (u, q) = normalise_unit("pounds", 1.0);
        assert_eq!(u, "g"); assert!((q - 453.592).abs() < 0.001);

        // Volume → ml
        let (u, q) = normalise_unit("ml", 100.0);
        assert_eq!(u, "ml"); assert!((q - 100.0).abs() < 0.001);

        let (u, q) = normalise_unit("l", 1.0);
        assert_eq!(u, "ml"); assert!((q - 1000.0).abs() < 0.001);

        let (u, q) = normalise_unit("tsp", 1.0);
        assert_eq!(u, "ml"); assert!((q - 4.92892).abs() < 0.001);

        let (u, q) = normalise_unit("tbsp", 1.0);
        assert_eq!(u, "ml"); assert!((q - 14.7868).abs() < 0.001);

        let (u, q) = normalise_unit("cup", 1.0);
        assert_eq!(u, "ml"); assert!((q - 236.588).abs() < 0.001);

        let (u, q) = normalise_unit("cups", 2.0);
        assert_eq!(u, "ml"); assert!((q - 473.176).abs() < 0.001);

        // Unknown → passthrough
        let (u, q) = normalise_unit("clove", 3.0);
        assert_eq!(u, "clove"); assert!((q - 3.0).abs() < 0.001);

        let (u, q) = normalise_unit("", 1.0);
        assert_eq!(u, ""); assert!((q - 1.0).abs() < 0.001);
    }
}