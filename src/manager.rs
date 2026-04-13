use sqlx::SqlitePool;
use crate::model::{Recipe, User};
use crate::storage;
use crate::SINGLE_USER_ID;
use validator::Validate;

/// Maximum number of recipes a single user may store.
const MAX_RECIPES_PER_USER: usize = 500;

// ---------------------------------------------------------------------------
// Auth
// ---------------------------------------------------------------------------

/// Looks up a user by username.
///
/// Returns `None` if no user exists with that username.
/// Used by the login handler to retrieve the stored hash for verification.
///
/// # Errors
///
/// Returns `Err` if the query fails.
pub async fn get_user_by_username(pool: &SqlitePool, username: &str) -> Result<Option<User>, String> {
    storage::load_user_by_username(pool, username).await
}

// ---------------------------------------------------------------------------
// Recipes
// ---------------------------------------------------------------------------

/// Retrieves a recipe by its ID.
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
    recipe.validate().map_err(|e| format!("Validation error: {e}"))?;
    let existing = storage::load_all_recipes(pool, SINGLE_USER_ID).await?;
    if existing.len() >= MAX_RECIPES_PER_USER {
        return Err(format!("Recipe limit of {} reached", MAX_RECIPES_PER_USER));
    }
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
    recipe.validate().map_err(|e| format!("Validation error: {e}"))?;
    storage::save_recipe(pool, id, &recipe).await
}