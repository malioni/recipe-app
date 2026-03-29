use sqlx::SqlitePool;
use crate::model::Recipe;
use crate::storage;

// When authentication is implemented, replace SINGLE_USER_ID with the
// session user's ID passed through from the network layer.
use crate::SINGLE_USER_ID;

/// Retrieves a recipe by its ID.
///
/// # Returns
///
/// Returns `Some(Recipe)` if found, `None` if the ID does not exist or
/// the query fails.
pub async fn get_recipe_by_id(pool: &SqlitePool, id: i64) -> Option<Recipe> {
    storage::load_recipe(pool, id).await.ok()
}

/// Adds a new recipe to storage.
///
/// # Errors
///
/// Returns `Err` if the recipe could not be persisted.
pub async fn add_recipe(pool: &SqlitePool, recipe: Recipe) -> Result<(), String> {
    storage::add_recipe(pool, SINGLE_USER_ID, &recipe).await?;
    Ok(())
}

/// Deletes the recipe at the given ID.
///
/// # Errors
///
/// Returns `Err` if no recipe exists at `id` or the query fails.
pub async fn delete_recipe(pool: &SqlitePool, id: i64) -> Result<(), String> {
    storage::delete_recipe(pool, id).await
}

/// Returns all recipes for the current user.
///
/// Returns an empty Vec if the query fails.
pub async fn get_all_recipes(pool: &SqlitePool) -> Vec<Recipe> {
    storage::load_all_recipes(pool, SINGLE_USER_ID).await.unwrap_or_default()
}

/// Updates an existing recipe by ID.
///
/// # Errors
///
/// Returns `Err` if no recipe exists at `id` or the query fails.
pub async fn update_recipe(pool: &SqlitePool, id: i64, recipe: Recipe) -> Result<(), String> {
    storage::save_recipe(pool, id, &recipe).await
}