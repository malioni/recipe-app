/// Calendar manager responsible for meal planning, cooked log, and shopping list logic.
///
/// This module sits between the network layer and `calendar_storage`. It owns
/// all business logic — validation, date arithmetic, ingredient aggregation —
/// so neither the network handlers nor the storage layer need to care about it.
use std::borrow::Cow;
use sqlx::SqlitePool;
use chrono::NaiveDate;
use crate::model::{CookedEntry, Ingredient, MealEntry, MealSlot, ShoppingListItem};
use crate::calendar_storage;
use crate::storage;

/// Maximum number of meal plan entries a single user may have at once.
///
/// Acts as an application-level storage cap: a user with thousands of entries
/// would cause the weekly calendar queries to become slow and the shopping list
/// to be unusably large. 1 000 covers roughly 2–3 years of daily planning.
#[cfg(not(test))]
const MAX_MEAL_PLAN_ENTRIES: usize = 1000;
#[cfg(test)]
const MAX_MEAL_PLAN_ENTRIES: usize = 3;

/// Maximum number of entries allowed per (user, date, slot) combination.
///
/// Allows a user to plan a main dish plus one or two sides for the same meal
/// slot without filling the UI card with an unbounded number of recipes.
#[cfg(not(test))]
const MAX_ENTRIES_PER_SLOT: usize = 3;
#[cfg(test)]
const MAX_ENTRIES_PER_SLOT: usize = 2;

/// Maximum number of portions (serving multiplier) allowed per meal plan entry.
///
/// Portions scale all ingredient quantities in the shopping list. A cap of 10
/// prevents accidental overflow values that would produce nonsensical quantities
/// (e.g. 500 portions of flour).
const MAX_PORTIONS: i64 = 10;

// ---------------------------------------------------------------------------
// Meal plan
// ---------------------------------------------------------------------------

/// Returns all planned meals within `[start, end]` (inclusive) for the given user.
///
/// Validates that `start <= end` before querying storage. Results are ordered
/// by date then by slot.
///
/// # Parameters
///
/// - `pool` — the SQLite connection pool.
/// - `user_id` — the ID of the authenticated user; only their entries are returned.
/// - `start` — the first date of the range (inclusive).
/// - `end` — the last date of the range (inclusive).
///
/// # Returns
///
/// `Ok(entries)` with all planned meals in the range. Returns an empty `Vec`
/// if no entries fall within `[start, end]`.
///
/// # Errors
///
/// Returns `Err` if `start > end` or the storage query fails.
pub async fn get_meals_in_range(
    pool: &SqlitePool,
    user_id: i64,
    start: NaiveDate,
    end: NaiveDate,
) -> Result<Vec<MealEntry>, String> {
    validate_range(start, end)?;
    calendar_storage::load_meal_entries_in_range(pool, user_id, start, end).await
}

/// Plans a recipe for a specific date and slot for the given user.
///
/// Performs three checks before inserting: (1) `portions` is within `[1, MAX_PORTIONS]`;
/// (2) `recipe_id` exists and is owned by `user_id`; (3) neither the per-user
/// total quota nor the per-slot limit has been reached.
///
/// # Parameters
///
/// - `pool` — the SQLite connection pool.
/// - `user_id` — the ID of the authenticated user planning the meal.
/// - `date` — the day on which the meal is planned.
/// - `slot` — which meal of the day (`Breakfast`, `Lunch`, or `Dinner`).
/// - `recipe_id` — the primary key of the recipe to plan; must be owned by `user_id`.
/// - `portions` — the serving multiplier applied to ingredient quantities in the
///   shopping list. Must be between 1 and `MAX_PORTIONS` (10 in production).
///
/// # Returns
///
/// `Ok(())` on success.
///
/// # Errors
///
/// Returns `Err` if `portions` is out of range, the recipe does not exist for
/// the user, the per-user plan quota is reached, the per-slot limit is reached,
/// or the storage query fails.
pub async fn plan_meal(
    pool: &SqlitePool,
    user_id: i64,
    date: NaiveDate,
    slot: MealSlot,
    recipe_id: i64,
    portions: i64,
) -> Result<(), String> {
    if !(1..=MAX_PORTIONS).contains(&portions) {
        return Err(format!("portions must be between 1 and {MAX_PORTIONS}"));
    }

    // Verify the recipe exists and belongs to this user before linking it.
    storage::load_recipe(pool, user_id, recipe_id).await
        .map_err(|_| format!("Recipe with ID {} not found", recipe_id))?;

    // Enforce per-user meal plan quota using a COUNT(*) query — no rows loaded.
    let total = calendar_storage::count_all_meal_entries(pool, user_id).await?;
    if total >= MAX_MEAL_PLAN_ENTRIES {
        return Err(format!("Meal plan limit of {} entries reached", MAX_MEAL_PLAN_ENTRIES));
    }

    // Enforce per-slot limit.
    let slot_count = calendar_storage::count_slot_entries(pool, user_id, date, &slot).await?;
    if slot_count >= MAX_ENTRIES_PER_SLOT {
        return Err(format!("Slot limit of {} entries reached", MAX_ENTRIES_PER_SLOT));
    }

    let entry = MealEntry { id: None, date, slot, recipe_id, portions };
    calendar_storage::add_meal_entry(pool, user_id, &entry).await
}

/// Removes a planned meal entry by its primary key, scoped to the owning user.
///
/// Deleting a non-existent id or one owned by a different user is a no-op —
/// idempotent by design.
///
/// # Parameters
///
/// - `pool` — the SQLite connection pool.
/// - `user_id` — the ID of the authenticated user; only their entries can be deleted.
/// - `id` — the primary key of the meal plan entry to remove.
///
/// # Returns
///
/// `Ok(())` on success, including when no row matched `id` and `user_id`.
///
/// # Errors
///
/// Returns `Err` if the storage query fails.
pub async fn remove_planned_meal(pool: &SqlitePool, user_id: i64, id: i64) -> Result<(), String> {
    calendar_storage::delete_meal_entry(pool, user_id, id).await
}

// ---------------------------------------------------------------------------
// Cooked log
// ---------------------------------------------------------------------------

/// Marks a recipe as cooked on the given date for the given user.
///
/// Verifies the recipe exists and belongs to `user_id` before inserting.
/// Duplicate entries for the same `(user_id, date, recipe_id)` are silently
/// ignored via `INSERT OR IGNORE`.
///
/// # Parameters
///
/// - `pool` — the SQLite connection pool.
/// - `user_id` — the ID of the authenticated user marking the recipe as cooked.
/// - `date` — the day on which the recipe was cooked.
/// - `recipe_id` — the primary key of the recipe; must be owned by `user_id`.
///
/// # Returns
///
/// `Ok(())` on success, including when an identical entry already exists.
///
/// # Errors
///
/// Returns `Err` if the recipe does not exist for the user, or the storage
/// query fails.
pub async fn mark_as_cooked(
    pool: &SqlitePool,
    user_id: i64,
    date: NaiveDate,
    recipe_id: i64,
) -> Result<(), String> {
    storage::load_recipe(pool, user_id, recipe_id).await
        .map_err(|_| format!("Recipe with ID {} not found", recipe_id))?;

    let entry = CookedEntry { date, recipe_id };
    calendar_storage::add_cooked_entry(pool, user_id, &entry).await
}

/// Returns all cooked entries within `[start, end]` (inclusive) for the given user.
///
/// Validates that `start <= end` before querying storage.
///
/// # Parameters
///
/// - `pool` — the SQLite connection pool.
/// - `user_id` — the ID of the authenticated user; only their entries are returned.
/// - `start` — the first date of the range (inclusive).
/// - `end` — the last date of the range (inclusive).
///
/// # Returns
///
/// `Ok(entries)` with all cooked log entries in the range. Returns an empty
/// `Vec` if no entries fall within `[start, end]`.
///
/// # Errors
///
/// Returns `Err` if `start > end` or the storage query fails.
pub async fn get_cooked_in_range(
    pool: &SqlitePool,
    user_id: i64,
    start: NaiveDate,
    end: NaiveDate,
) -> Result<Vec<CookedEntry>, String> {
    validate_range(start, end)?;
    calendar_storage::load_cooked_entries_in_range(pool, user_id, start, end).await
}

// ---------------------------------------------------------------------------
// Shopping list
// ---------------------------------------------------------------------------

/// Aggregates ingredients across all planned meals within `[start, end]`.
///
/// Internally accumulates weights in `g` and volumes in `ml` for precision.
/// The returned [`ShoppingListItem`] values carry display-ready quantities:
/// metric values are ceiled to a human-friendly step (10 g / 10 ml below the
/// 100 g/ml threshold; 100 g / 100 ml above it, then expressed in kg/l), and
/// imperial values are ceiled to the nearest whole `oz` (weight) or `fl oz`
/// (volume). Count-based or unrecognised units pass through unchanged with
/// `imperial_quantity = None`. All ingredient names in the output are
/// lowercased so that `"Flour"` and `"flour"` from different recipes merge
/// into a single line.
///
/// # Parameters
///
/// - `pool` — the SQLite connection pool.
/// - `user_id` — the ID of the authenticated user; only their planned meals are used.
/// - `start` — the first date of the range (inclusive).
/// - `end` — the last date of the range (inclusive).
///
/// # Returns
///
/// `Ok(items)` with one [`ShoppingListItem`] per unique `(lowercase_name,
/// canonical_unit)` pair across all planned meals in the range. Returns an
/// empty `Vec` if no meals are planned.
///
/// # Errors
///
/// Returns `Err` if `start > end`, the query fails, or a recipe referenced by
/// a meal plan entry no longer exists in the database.
pub async fn get_shopping_list(
    pool: &SqlitePool,
    user_id: i64,
    start: NaiveDate,
    end: NaiveDate,
) -> Result<Vec<ShoppingListItem>, String> {
    validate_range(start, end)?;

    let entries = calendar_storage::load_meal_entries_in_range(
        pool, user_id, start, end
    ).await?;

    // Accumulate in base units (g / ml) keyed by (lowercase_name, canonical_unit).
    // Using lowercase name means "Flour" and "flour" merge into one entry.
    // Using canonical unit means weight-flour (g) and volume-flour (ml) stay
    // separate, because they are physically different measurements that cannot
    // be summed.
    let mut aggregated: Vec<Ingredient> = Vec::new();

    for entry in entries {
        let recipe = storage::load_recipe(pool, user_id, entry.recipe_id).await
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
                    unit: canonical_unit.into_owned(),
                }),
            }
        }
    }

    Ok(aggregated
        .into_iter()
        .map(|i| to_display_item(i.name, &i.unit, i.quantity))
        .collect())
}

/// Converts an internally-accumulated `(name, base_unit, base_qty)` triple into
/// a display-ready [`ShoppingListItem`].
///
/// **`base_unit`** must be one of `"g"`, `"ml"`, or an unrecognised passthrough
/// (e.g. `"clove"`, `"piece"`, `""`).
///
/// **Metric rounding** — quantities are ceiled to a human-friendly step:
/// - Below 100: step of 10 → stays in `g` / `ml`  (e.g. 61 g → 70 g)
/// - 100 or above: step of 100, then divided by 1 000 → expressed in `kg` / `l`
///   (e.g. 650 g → 0.7 kg)
///
/// The threshold `< 100.0` means exactly 100 g goes to the kg branch (0.1 kg).
///
/// **Imperial conversion factors** (applied to the raw base quantity before ceiling):
/// - Weight: 1 g = 0.035274 oz  → `ceil(base_qty × 0.035274)` oz
/// - Volume: 1 ml = 0.033814 fl oz → `ceil(base_qty × 0.033814)` fl oz
///
/// Passthrough units have no imperial equivalent; `imperial_quantity` and
/// `imperial_unit` are `None`.
///
/// # Example
///
/// `to_display_item("flour", "g", 650.0)` → `ShoppingListItem { metric_quantity: 0.7,
/// metric_unit: "kg", imperial_quantity: Some(23.0), imperial_unit: Some("oz") }`
fn to_display_item(name: String, base_unit: &str, base_qty: f32) -> ShoppingListItem {
    match base_unit {
        "g" => {
            let (metric_quantity, metric_unit) = if base_qty < 100.0 {
                (ceil_to(base_qty, 10.0), "g".to_string())
            } else {
                (ceil_to(base_qty, 100.0) / 1_000.0, "kg".to_string())
            };
            let imperial_quantity = Some((base_qty * 0.035274_f32).ceil());
            ShoppingListItem {
                name,
                metric_quantity,
                metric_unit,
                imperial_quantity,
                imperial_unit: Some("oz".to_string()),
            }
        }
        "ml" => {
            let (metric_quantity, metric_unit) = if base_qty < 100.0 {
                (ceil_to(base_qty, 10.0), "ml".to_string())
            } else {
                (ceil_to(base_qty, 100.0) / 1_000.0, "l".to_string())
            };
            let imperial_quantity = Some((base_qty * 0.033814_f32).ceil());
            ShoppingListItem {
                name,
                metric_quantity,
                metric_unit,
                imperial_quantity,
                imperial_unit: Some("fl oz".to_string()),
            }
        }
        other => ShoppingListItem {
            name,
            metric_quantity: base_qty,
            metric_unit: other.to_string(),
            imperial_quantity: None,
            imperial_unit: None,
        },
    }
}

/// Ceils `value` to the nearest multiple of `step`.
///
/// The result is always ≥ `value` (i.e. never rounded down). Exact multiples
/// are returned unchanged.
///
/// Used by [`to_display_item`] to produce human-friendly shopping quantities
/// (e.g. "buy 70 g" rather than "buy 61 g").
///
/// # Examples
///
/// - `ceil_to(61.0, 10.0)` → `70.0`
/// - `ceil_to(70.0, 10.0)` → `70.0` (exact multiple, unchanged)
/// - `ceil_to(650.0, 100.0)` → `700.0`
fn ceil_to(value: f32, step: f32) -> f32 {
    (value / step).ceil() * step
}

/// Maps a unit string to its canonical base unit and scales the quantity accordingly.
///
/// All weight units normalise to `"g"`; all volume units normalise to `"ml"`.
/// Matching is case-insensitive. Unrecognised units (including empty string and
/// count-based labels like `"clove"` or `"piece"`) are returned unchanged so
/// they still aggregate correctly by exact string match.
///
/// **Conversion factors used:**
/// - `kg` → g: × 1 000
/// - `oz` / `ounce` → g: × 28.3495
/// - `lb` / `lbs` / `pound` → g: × 453.592
/// - `l` / `liter` / `litre` → ml: × 1 000
/// - `tsp` / `teaspoon` → ml: × 4.92892
/// - `tbsp` / `tablespoon` → ml: × 14.7868
/// - `cup` / `cups` → ml: × 236.588
///
/// Returns a `Cow` so known units borrow a `'static` literal (no allocation);
/// only the passthrough branch borrows the caller's input slice.
///
/// # Example
///
/// `normalise_unit("kg", 0.5)` → `(Cow::Borrowed("g"), 500.0)`
fn normalise_unit<'a>(unit: &'a str, quantity: f32) -> (Cow<'a, str>, f32) {
    match unit.to_lowercase().as_str() {
        // Weight → g
        "g"                               => (Cow::Borrowed("g"),  quantity),
        "kg"                              => (Cow::Borrowed("g"),  quantity * 1_000.0),
        "oz" | "ounce" | "ounces"         => (Cow::Borrowed("g"),  quantity * 28.3495),
        "lb" | "lbs" | "pound" | "pounds" => (Cow::Borrowed("g"),  quantity * 453.592),
        // Volume → ml
        "ml"                                  => (Cow::Borrowed("ml"), quantity),
        "l" | "liter" | "litre"               => (Cow::Borrowed("ml"), quantity * 1_000.0),
        "tsp" | "teaspoon" | "teaspoons"      => (Cow::Borrowed("ml"), quantity * 4.92892),
        "tbsp" | "tablespoon" | "tablespoons" => (Cow::Borrowed("ml"), quantity * 14.7868),
        "cup" | "cups"                        => (Cow::Borrowed("ml"), quantity * 236.588),
        // Unknown — pass through unchanged
        _ => (Cow::Borrowed(unit), quantity),
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
        sqlx::query(include_str!("../migrations/004_add_is_admin_to_users.sql"))
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
        plan_meal(&pool, 1, date, MealSlot::Lunch, 1, 1).await.expect("plan_meal should succeed");
        let meals = get_meals_in_range(&pool, 1, date, date).await.unwrap();
        assert_eq!(meals.len(), 1);
        assert_eq!(meals[0].recipe_id, 1);
        assert_eq!(meals[0].slot, MealSlot::Lunch);
    }

    #[tokio::test]
    async fn test_plan_meal_invalid_recipe_id() {
        let pool = setup().await;
        let date = NaiveDate::from_ymd_opt(2026, 4, 1).unwrap();
        let result = plan_meal(&pool, 1, date, MealSlot::Lunch, 999_999, 1).await;
        assert!(result.is_err(), "Planning with a non-existent recipe should fail");
    }

    #[tokio::test]
    async fn test_remove_planned_meal() {
        let pool = setup().await;
        let date = NaiveDate::from_ymd_opt(2026, 4, 1).unwrap();
        plan_meal(&pool, 1, date, MealSlot::Dinner, 1, 1).await.unwrap();
        let meals = get_meals_in_range(&pool, 1, date, date).await.unwrap();
        let id = meals[0].id.unwrap();
        remove_planned_meal(&pool, 1, id).await.expect("Remove should succeed");
        let meals = get_meals_in_range(&pool, 1, date, date).await.unwrap();
        assert!(meals.is_empty());
    }

    #[tokio::test]
    async fn test_remove_planned_meal_not_found() {
        let pool = setup().await;
        // Removing a non-existent id is a no-op — idempotent by design.
        assert!(remove_planned_meal(&pool, 1, 999_999).await.is_ok());
    }

    #[tokio::test]
    async fn test_mark_as_cooked_happy_path() {
        let pool = setup().await;
        let date = NaiveDate::from_ymd_opt(2026, 4, 1).unwrap();
        mark_as_cooked(&pool, 1, date, 1).await.expect("mark_as_cooked should succeed");
        let cooked = get_cooked_in_range(&pool, 1, date, date).await.unwrap();
        assert_eq!(cooked.len(), 1);
        assert_eq!(cooked[0].recipe_id, 1);
    }

    #[tokio::test]
    async fn test_mark_as_cooked_invalid_recipe() {
        let pool = setup().await;
        let date = NaiveDate::from_ymd_opt(2026, 4, 1).unwrap();
        let result = mark_as_cooked(&pool, 1, date, 999_999).await;
        assert!(result.is_err(), "Marking a non-existent recipe as cooked should fail");
    }

    #[tokio::test]
    async fn test_get_cooked_in_range_invalid_range() {
        let pool = setup().await;
        let start = NaiveDate::from_ymd_opt(2026, 4, 7).unwrap();
        let end = NaiveDate::from_ymd_opt(2026, 4, 1).unwrap();
        assert!(get_cooked_in_range(&pool, 1, start, end).await.is_err());
    }

    #[tokio::test]
    async fn test_get_shopping_list_empty_range() {
        let pool = setup().await;
        let start = NaiveDate::from_ymd_opt(2099, 1, 1).unwrap();
        let end = NaiveDate::from_ymd_opt(2099, 1, 7).unwrap();
        let list = get_shopping_list(&pool, 1, start, end).await.unwrap();
        assert!(list.is_empty());
    }

    #[tokio::test]
    async fn test_get_shopping_list_returns_ingredients() {
        let pool = setup().await;
        let date = NaiveDate::from_ymd_opt(2026, 4, 1).unwrap();
        plan_meal(&pool, 1, date, MealSlot::Lunch, 1, 1).await.unwrap();
        let list = get_shopping_list(&pool, 1, date, date).await.unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].name, "flour");
        // 200 g ≥ 100 g threshold → 0.2 kg; imperial: ceil(200 × 0.035274) = ceil(7.055) = 8 oz
        assert!((list[0].metric_quantity - 0.2).abs() < 0.001);
        assert_eq!(list[0].metric_unit, "kg");
        assert_eq!(list[0].imperial_quantity, Some(8.0));
        assert_eq!(list[0].imperial_unit, Some("oz".to_string()));
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
            plan_meal(&pool, 1, date, slot, 1, 1).await
                .expect("should succeed within quota");
            last_date = date;
        }
        let overflow_date = last_date + chrono::Duration::days(1);
        let result = plan_meal(&pool, 1, overflow_date, MealSlot::Breakfast, 1, 1).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("limit"));
    }

    #[tokio::test]
    async fn test_slot_quota_enforced() {
        let pool = setup().await;
        let date = NaiveDate::from_ymd_opt(2026, 4, 1).unwrap();
        for _ in 0..MAX_ENTRIES_PER_SLOT {
            plan_meal(&pool, 1, date, MealSlot::Dinner, 1, 1).await
                .expect("should succeed within slot quota");
        }
        let result = plan_meal(&pool, 1, date, MealSlot::Dinner, 1, 1).await;
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
        plan_meal(&pool, 1, date, MealSlot::Lunch, 1, 1).await.unwrap();   // 200g Flour
        plan_meal(&pool, 1, date, MealSlot::Dinner, 2, 1).await.unwrap();  // 100g Flour

        let list = get_shopping_list(&pool, 1, date, date).await.unwrap();
        assert_eq!(list.len(), 1, "Same ingredient+unit should be merged into one entry");
        // 300 g → 0.3 kg; imperial: ceil(300 × 0.035274) = ceil(10.582) = 11 oz
        assert!((list[0].metric_quantity - 0.3).abs() < 0.001);
        assert_eq!(list[0].metric_unit, "kg");
        assert_eq!(list[0].imperial_quantity, Some(11.0));
    }

    #[tokio::test]
    async fn test_plan_meal_portions_scaling() {
        let pool = setup().await;
        let date = NaiveDate::from_ymd_opt(2026, 4, 1).unwrap();
        // Recipe has 200g Flour; 2 portions should give 400g in shopping list.
        plan_meal(&pool, 1, date, MealSlot::Lunch, 1, 2).await.unwrap();
        let list = get_shopping_list(&pool, 1, date, date).await.unwrap();
        assert_eq!(list.len(), 1);
        // 2 × 200 g = 400 g → 0.4 kg; imperial: ceil(400 × 0.035274) = ceil(14.11) = 15 oz
        assert!((list[0].metric_quantity - 0.4).abs() < 0.001, "2 portions should double the quantity");
        assert_eq!(list[0].metric_unit, "kg");
        assert_eq!(list[0].imperial_quantity, Some(15.0));
    }

    #[tokio::test]
    async fn test_plan_meal_portions_invalid() {
        let pool = setup().await;
        let date = NaiveDate::from_ymd_opt(2026, 4, 1).unwrap();
        assert!(plan_meal(&pool, 1, date, MealSlot::Lunch, 1, 0).await.is_err(), "0 portions should fail");
        assert!(plan_meal(&pool, 1, date, MealSlot::Lunch, 1, 11).await.is_err(), "11 portions should fail");
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
        plan_meal(&pool, 1, date, MealSlot::Breakfast, 1, 1).await.unwrap();
        plan_meal(&pool, 1, date, MealSlot::Dinner, 2, 1).await.unwrap();

        let list = get_shopping_list(&pool, 1, date, date).await.unwrap();
        assert_eq!(list.len(), 1, "\"Flour\" and \"flour\" should merge");
        assert_eq!(list[0].name, "flour", "Output name should be lowercased");
        // 300 g ≥ 100 g → 0.3 kg; imperial: ceil(300 × 0.035274) = ceil(10.582) = 11 oz
        assert!((list[0].metric_quantity - 0.3).abs() < 0.001);
        assert_eq!(list[0].metric_unit, "kg");
        assert_eq!(list[0].imperial_quantity, Some(11.0));
        assert_eq!(list[0].imperial_unit, Some("oz".to_string()));
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
        plan_meal(&pool, 1, date, MealSlot::Lunch, 2, 1).await.unwrap();   // 1 lb Butter
        plan_meal(&pool, 1, date, MealSlot::Dinner, 3, 1).await.unwrap();  // 8 oz Butter

        let list = get_shopping_list(&pool, 1, date, date).await.unwrap();
        let butter: Vec<_> = list.iter().filter(|i| i.name == "butter").collect();
        assert_eq!(butter.len(), 1, "lb and oz should merge into one entry");
        // 453.592 + 226.796 = 680.388 g ≥ 100 g → ceil_to(680.388, 100)/1000 = 0.7 kg
        // imperial: ceil(680.388 × 0.035274) — f32 precision lands just above 24, so ceils to 25 oz
        assert_eq!(butter[0].metric_unit, "kg");
        assert!((butter[0].metric_quantity - 0.7).abs() < 0.001);
        assert_eq!(butter[0].imperial_quantity, Some(25.0));
        assert_eq!(butter[0].imperial_unit, Some("oz".to_string()));
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
        plan_meal(&pool, 1, date, MealSlot::Lunch, 2, 1).await.unwrap();
        plan_meal(&pool, 1, date, MealSlot::Dinner, 3, 1).await.unwrap();

        let list = get_shopping_list(&pool, 1, date, date).await.unwrap();
        let sugar: Vec<_> = list.iter().filter(|i| i.name == "sugar").collect();
        assert_eq!(sugar.len(), 1, "kg and g should merge");
        // 500 g + 200 g = 700 g ≥ 100 g → 0.7 kg; imperial: ceil(700 × 0.035274) = ceil(24.692) = 25 oz
        assert_eq!(sugar[0].metric_unit, "kg");
        assert!((sugar[0].metric_quantity - 0.7).abs() < 0.001);
        assert_eq!(sugar[0].imperial_quantity, Some(25.0));
        assert_eq!(sugar[0].imperial_unit, Some("oz".to_string()));
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
        plan_meal(&pool, 1, date, MealSlot::Lunch, 2, 1).await.unwrap();
        plan_meal(&pool, 1, date, MealSlot::Dinner, 3, 1).await.unwrap();

        let list = get_shopping_list(&pool, 1, date, date).await.unwrap();
        let salt: Vec<_> = list.iter().filter(|i| i.name == "salt").collect();
        assert_eq!(salt.len(), 1, "oz and g should merge");
        // 2 oz + 10 g = 66.699 g < 100 g → ceil_to(66.699, 10) = 70 g; imperial: ceil(66.699 × 0.035274) = ceil(2.353) = 3 oz
        assert_eq!(salt[0].metric_unit, "g");
        assert!((salt[0].metric_quantity - 70.0).abs() < 0.001);
        assert_eq!(salt[0].imperial_quantity, Some(3.0));
        assert_eq!(salt[0].imperial_unit, Some("oz".to_string()));
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
        plan_meal(&pool, 1, date, MealSlot::Lunch, 2, 1).await.unwrap();   // 0.5 l
        plan_meal(&pool, 1, date, MealSlot::Dinner, 3, 1).await.unwrap();  // 2 tbsp + 3 tsp

        let list = get_shopping_list(&pool, 1, date, date).await.unwrap();
        let water: Vec<_> = list.iter().filter(|i| i.name == "water").collect();
        assert_eq!(water.len(), 1, "l, tbsp, and tsp should all merge");
        // 500 + 29.574 + 14.787 ≈ 544.36 ml ≥ 100 ml → ceil_to(544.36, 100)/1000 = 0.6 l
        // imperial: ceil(544.36 × 0.033814) = ceil(18.41) = 19 fl oz
        assert_eq!(water[0].metric_unit, "l");
        assert!((water[0].metric_quantity - 0.6).abs() < 0.001);
        assert_eq!(water[0].imperial_quantity, Some(19.0));
        assert_eq!(water[0].imperial_unit, Some("fl oz".to_string()));
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
        plan_meal(&pool, 1, date, MealSlot::Lunch, 1, 1).await.unwrap();   // 200g Flour
        plan_meal(&pool, 1, date, MealSlot::Dinner, 2, 1).await.unwrap();  // 100ml Flour

        let list = get_shopping_list(&pool, 1, date, date).await.unwrap();
        let flour: Vec<_> = list.iter().filter(|i| i.name == "flour").collect();
        assert_eq!(flour.len(), 2, "Weight and volume must not be merged");
        // 200 g → metric "kg"; 100 ml → metric "l" (exactly at threshold, ≥ 100 → kg/l)
        let metric_units: Vec<&str> = flour.iter().map(|i| i.metric_unit.as_str()).collect();
        assert!(metric_units.contains(&"kg") && metric_units.contains(&"l"));
    }

    #[tokio::test]
    async fn test_shopping_list_imperial_weight_oz() {
        let pool = setup().await;
        sqlx::query(
            "INSERT INTO recipes (id, user_id, name, ingredients, instructions) \
             VALUES (2, 1, 'Heavy', '[{\"name\":\"Lead\",\"quantity\":1.0,\"unit\":\"kg\"}]', '[]')"
        )
        .execute(&pool).await.unwrap();

        let date = NaiveDate::from_ymd_opt(2026, 5, 1).unwrap();
        plan_meal(&pool, 1, date, MealSlot::Lunch, 2, 1).await.unwrap();
        let list = get_shopping_list(&pool, 1, date, date).await.unwrap();
        let item = list.iter().find(|i| i.name == "lead").unwrap();
        // 1000 g → imperial: ceil(1000 × 0.035274) = ceil(35.274) = 36 oz
        assert_eq!(item.imperial_quantity, Some(36.0));
        assert_eq!(item.imperial_unit, Some("oz".to_string()));
    }

    #[tokio::test]
    async fn test_shopping_list_imperial_volume_fl_oz() {
        let pool = setup().await;
        sqlx::query(
            "INSERT INTO recipes (id, user_id, name, ingredients, instructions) \
             VALUES (2, 1, 'Drink', '[{\"name\":\"Juice\",\"quantity\":1.0,\"unit\":\"l\"}]', '[]')"
        )
        .execute(&pool).await.unwrap();

        let date = NaiveDate::from_ymd_opt(2026, 5, 2).unwrap();
        plan_meal(&pool, 1, date, MealSlot::Lunch, 2, 1).await.unwrap();
        let list = get_shopping_list(&pool, 1, date, date).await.unwrap();
        let item = list.iter().find(|i| i.name == "juice").unwrap();
        // 1000 ml → imperial: ceil(1000 × 0.033814) = ceil(33.814) = 34 fl oz
        assert_eq!(item.imperial_quantity, Some(34.0));
        assert_eq!(item.imperial_unit, Some("fl oz".to_string()));
    }

    #[tokio::test]
    async fn test_shopping_list_no_imperial_for_unknown_unit() {
        let pool = setup().await;
        sqlx::query(
            "INSERT INTO recipes (id, user_id, name, ingredients, instructions) \
             VALUES (2, 1, 'Garlic Bread', '[{\"name\":\"Garlic\",\"quantity\":3.0,\"unit\":\"clove\"}]', '[]')"
        )
        .execute(&pool).await.unwrap();

        let date = NaiveDate::from_ymd_opt(2026, 5, 3).unwrap();
        plan_meal(&pool, 1, date, MealSlot::Lunch, 2, 1).await.unwrap();
        let list = get_shopping_list(&pool, 1, date, date).await.unwrap();
        let item = list.iter().find(|i| i.name == "garlic").unwrap();
        assert_eq!(item.imperial_quantity, None);
        assert_eq!(item.imperial_unit, None);
        assert_eq!(item.metric_unit, "clove");
    }

    #[tokio::test]
    async fn test_shopping_list_metric_threshold_weight() {
        let pool = setup().await;
        // 50 g < 100 g → display in g; 200 g ≥ 100 g → display in kg
        sqlx::query(
            "INSERT INTO recipes (id, user_id, name, ingredients, instructions) \
             VALUES (2, 1, 'A', '[{\"name\":\"SmallWeight\",\"quantity\":50.0,\"unit\":\"g\"}]', '[]')"
        )
        .execute(&pool).await.unwrap();
        sqlx::query(
            "INSERT INTO recipes (id, user_id, name, ingredients, instructions) \
             VALUES (3, 1, 'B', '[{\"name\":\"LargeWeight\",\"quantity\":200.0,\"unit\":\"g\"}]', '[]')"
        )
        .execute(&pool).await.unwrap();

        let date = NaiveDate::from_ymd_opt(2026, 5, 4).unwrap();
        plan_meal(&pool, 1, date, MealSlot::Lunch, 2, 1).await.unwrap();
        plan_meal(&pool, 1, date, MealSlot::Dinner, 3, 1).await.unwrap();
        let list = get_shopping_list(&pool, 1, date, date).await.unwrap();

        let small = list.iter().find(|i| i.name == "smallweight").unwrap();
        assert_eq!(small.metric_unit, "g");
        assert!((small.metric_quantity - 50.0).abs() < 0.001);

        let large = list.iter().find(|i| i.name == "largeweight").unwrap();
        assert_eq!(large.metric_unit, "kg");
        assert!((large.metric_quantity - 0.2).abs() < 0.001);
    }

    #[tokio::test]
    async fn test_shopping_list_metric_threshold_volume() {
        let pool = setup().await;
        // 50 ml < 100 ml → display in ml; 200 ml ≥ 100 ml → display in l
        sqlx::query(
            "INSERT INTO recipes (id, user_id, name, ingredients, instructions) \
             VALUES (2, 1, 'A', '[{\"name\":\"SmallVol\",\"quantity\":50.0,\"unit\":\"ml\"}]', '[]')"
        )
        .execute(&pool).await.unwrap();
        sqlx::query(
            "INSERT INTO recipes (id, user_id, name, ingredients, instructions) \
             VALUES (3, 1, 'B', '[{\"name\":\"LargeVol\",\"quantity\":200.0,\"unit\":\"ml\"}]', '[]')"
        )
        .execute(&pool).await.unwrap();

        let date = NaiveDate::from_ymd_opt(2026, 5, 5).unwrap();
        plan_meal(&pool, 1, date, MealSlot::Lunch, 2, 1).await.unwrap();
        plan_meal(&pool, 1, date, MealSlot::Dinner, 3, 1).await.unwrap();
        let list = get_shopping_list(&pool, 1, date, date).await.unwrap();

        let small = list.iter().find(|i| i.name == "smallvol").unwrap();
        assert_eq!(small.metric_unit, "ml");
        assert!((small.metric_quantity - 50.0).abs() < 0.001);

        let large = list.iter().find(|i| i.name == "largevol").unwrap();
        assert_eq!(large.metric_unit, "l");
        assert!((large.metric_quantity - 0.2).abs() < 0.001);
    }

    #[tokio::test]
    async fn test_shopping_list_ceiling_below_threshold() {
        let pool = setup().await;
        // Verifies ceiling behaviour: 61 g → 70 g, 70 g → 70 g, 71 g → 80 g
        sqlx::query(
            "INSERT INTO recipes (id, user_id, name, ingredients, instructions) \
             VALUES (2, 1, 'A', '[{\"name\":\"X61\",\"quantity\":61.0,\"unit\":\"g\"}]', '[]')"
        )
        .execute(&pool).await.unwrap();
        sqlx::query(
            "INSERT INTO recipes (id, user_id, name, ingredients, instructions) \
             VALUES (3, 1, 'B', '[{\"name\":\"X70\",\"quantity\":70.0,\"unit\":\"g\"}]', '[]')"
        )
        .execute(&pool).await.unwrap();
        sqlx::query(
            "INSERT INTO recipes (id, user_id, name, ingredients, instructions) \
             VALUES (4, 1, 'C', '[{\"name\":\"X71\",\"quantity\":71.0,\"unit\":\"g\"}]', '[]')"
        )
        .execute(&pool).await.unwrap();

        let date = NaiveDate::from_ymd_opt(2026, 5, 6).unwrap();
        plan_meal(&pool, 1, date, MealSlot::Breakfast, 2, 1).await.unwrap();
        plan_meal(&pool, 1, date, MealSlot::Lunch, 3, 1).await.unwrap();
        plan_meal(&pool, 1, date, MealSlot::Dinner, 4, 1).await.unwrap();
        let list = get_shopping_list(&pool, 1, date, date).await.unwrap();

        let x61 = list.iter().find(|i| i.name == "x61").unwrap();
        assert!((x61.metric_quantity - 70.0).abs() < 0.001, "61 g should ceil to 70 g");

        let x70 = list.iter().find(|i| i.name == "x70").unwrap();
        assert!((x70.metric_quantity - 70.0).abs() < 0.001, "70 g exact multiple should stay 70 g");

        let x71 = list.iter().find(|i| i.name == "x71").unwrap();
        assert!((x71.metric_quantity - 80.0).abs() < 0.001, "71 g should ceil to 80 g");
    }

    #[test]
    fn test_ceil_to() {
        // Exact multiples stay unchanged
        assert!((ceil_to(70.0, 10.0) - 70.0).abs() < f32::EPSILON);
        assert!((ceil_to(100.0, 100.0) - 100.0).abs() < f32::EPSILON);

        // Just above a multiple → rounds up to next step
        assert!((ceil_to(71.0, 10.0) - 80.0).abs() < f32::EPSILON);
        assert!((ceil_to(101.0, 100.0) - 200.0).abs() < f32::EPSILON);

        // Just below a multiple → rounds up to that multiple
        assert!((ceil_to(61.0, 10.0) - 70.0).abs() < f32::EPSILON);
        assert!((ceil_to(99.0, 100.0) - 100.0).abs() < f32::EPSILON);

        // Result is never smaller than the input
        for v in [1.0_f32, 9.9, 10.0, 10.1, 55.5, 99.9, 100.0] {
            assert!(ceil_to(v, 10.0) >= v, "ceil_to({v}, 10) must be >= {v}");
        }
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

        let (u, q) = normalise_unit("lbs", 1.0);
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

    // -------------------------------------------------------------------------
    // Quota isolation and release tests
    // -------------------------------------------------------------------------

    #[tokio::test]
    async fn test_quota_per_user_isolation() {
        let pool = setup().await;
        // Insert a second user and a recipe they own.
        sqlx::query("INSERT INTO users (id, username, password_hash) VALUES (2, 'user2', 'placeholder')")
            .execute(&pool).await.unwrap();
        sqlx::query(
            "INSERT INTO recipes (id, user_id, name, ingredients, instructions) \
             VALUES (2, 2, 'User2 Recipe', '[]', '[]')"
        )
        .execute(&pool).await.unwrap();

        // Fill user 1's quota (MAX_MEAL_PLAN_ENTRIES = 3 in test mode).
        let slots = [MealSlot::Breakfast, MealSlot::Lunch, MealSlot::Dinner];
        let base = NaiveDate::from_ymd_opt(2031, 1, 1).unwrap();
        for i in 0..MAX_MEAL_PLAN_ENTRIES {
            let date = base + chrono::Duration::days((i / slots.len()) as i64);
            let slot = slots[i % slots.len()].clone();
            plan_meal(&pool, 1, date, slot, 1, 1).await
                .expect("user1 should succeed within quota");
        }
        // Confirm user 1 is at quota.
        assert!(plan_meal(&pool, 1, base + chrono::Duration::days(99), MealSlot::Breakfast, 1, 1).await.is_err());

        // User 2's quota is independent — they must still be able to plan.
        let result = plan_meal(&pool, 2, base, MealSlot::Breakfast, 2, 1).await;
        assert!(result.is_ok(), "User 2 must not be blocked by User 1's quota");
    }

    #[tokio::test]
    async fn test_quota_release_after_delete() {
        let pool = setup().await;
        let base = NaiveDate::from_ymd_opt(2032, 1, 1).unwrap();
        let slots = [MealSlot::Breakfast, MealSlot::Lunch, MealSlot::Dinner];

        // Fill quota (3 in test mode).
        for i in 0..MAX_MEAL_PLAN_ENTRIES {
            let date = base + chrono::Duration::days((i / slots.len()) as i64);
            let slot = slots[i % slots.len()].clone();
            plan_meal(&pool, 1, date, slot, 1, 1).await
                .expect("should succeed within quota");
        }
        // Adding one more should fail.
        assert!(plan_meal(&pool, 1, base + chrono::Duration::days(99), MealSlot::Breakfast, 1, 1).await.is_err());

        // Delete one entry to free a slot.
        let meals = get_meals_in_range(
            &pool, 1,
            NaiveDate::from_ymd_opt(1970, 1, 1).unwrap(),
            NaiveDate::from_ymd_opt(9999, 12, 31).unwrap(),
        ).await.unwrap();
        let entry_id = meals[0].id.unwrap();
        remove_planned_meal(&pool, 1, entry_id).await.expect("delete should succeed");

        // Now adding one more must succeed.
        let result = plan_meal(&pool, 1, base + chrono::Duration::days(99), MealSlot::Breakfast, 1, 1).await;
        assert!(result.is_ok(), "Adding after deleting one entry should succeed");
    }

    // -------------------------------------------------------------------------
    // Shopping list edge case tests
    // -------------------------------------------------------------------------

    #[tokio::test]
    async fn test_shopping_list_sub_one_gram() {
        let pool = setup().await;
        sqlx::query(
            "INSERT INTO recipes (id, user_id, name, ingredients, instructions) \
             VALUES (2, 1, 'Tiny', '[{\"name\":\"Spice\",\"quantity\":0.5,\"unit\":\"g\"}]', '[]')"
        )
        .execute(&pool).await.unwrap();

        let date = NaiveDate::from_ymd_opt(2026, 6, 1).unwrap();
        plan_meal(&pool, 1, date, MealSlot::Lunch, 2, 1).await.unwrap();
        let list = get_shopping_list(&pool, 1, date, date).await.unwrap();
        let spice = list.iter().find(|i| i.name == "spice").unwrap();
        // 0.5 g < 100 g → ceil_to(0.5, 10) = 10 g
        assert_eq!(spice.metric_unit, "g");
        assert!((spice.metric_quantity - 10.0).abs() < 0.001, "0.5 g should ceil to 10 g");
        // imperial: ceil(0.5 × 0.035274) = ceil(0.01764) = 1 oz
        assert_eq!(spice.imperial_quantity, Some(1.0));
        assert_eq!(spice.imperial_unit, Some("oz".to_string()));
    }

    #[tokio::test]
    async fn test_shopping_list_exact_threshold_boundary() {
        let pool = setup().await;
        // Exactly 100 g → must go to the kg branch (threshold is `< 100.0`).
        sqlx::query(
            "INSERT INTO recipes (id, user_id, name, ingredients, instructions) \
             VALUES (2, 1, 'Exact', '[{\"name\":\"Boundary\",\"quantity\":100.0,\"unit\":\"g\"}]', '[]')"
        )
        .execute(&pool).await.unwrap();

        let date = NaiveDate::from_ymd_opt(2026, 6, 2).unwrap();
        plan_meal(&pool, 1, date, MealSlot::Lunch, 2, 1).await.unwrap();
        let list = get_shopping_list(&pool, 1, date, date).await.unwrap();
        let item = list.iter().find(|i| i.name == "boundary").unwrap();
        // 100 g ≥ 100 → ceil_to(100, 100) / 1000 = 0.1 kg
        assert_eq!(item.metric_unit, "kg", "Exactly 100 g must use the kg branch");
        assert!((item.metric_quantity - 0.1).abs() < 0.001);
        // imperial: ceil(100 × 0.035274) = ceil(3.5274) = 4 oz
        assert_eq!(item.imperial_quantity, Some(4.0));
        assert_eq!(item.imperial_unit, Some("oz".to_string()));
    }
}
