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

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Ingredient;

    async fn setup() -> SqlitePool {
        let pool = SqlitePool::connect(":memory:").await.unwrap();
        sqlx::query(include_str!("../migrations/001_initial.sql"))
            .execute(&pool).await.unwrap();
        sqlx::query(
            "INSERT INTO users (id, username, password_hash) VALUES (1, 'test', 'placeholder')"
        )
        .execute(&pool).await.unwrap();
        pool
    }

    fn bare_recipe(name: &str) -> Recipe {
        Recipe { id: 0, name: name.to_string(), source_url: None, ingredients: vec![], instructions: vec![] }
    }

    #[tokio::test]
    async fn test_add_recipe_valid() {
        let pool = setup().await;
        assert!(add_recipe(&pool, bare_recipe("Pasta")).await.is_ok());
    }

    #[tokio::test]
    async fn test_add_recipe_name_too_long() {
        let pool = setup().await;
        let result = add_recipe(&pool, bare_recipe(&"a".repeat(201))).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Validation"));
    }

    #[tokio::test]
    async fn test_add_recipe_invalid_source_url() {
        let pool = setup().await;
        let r = Recipe {
            id: 0, name: "Soup".to_string(),
            source_url: Some("not-a-url".to_string()),
            ingredients: vec![], instructions: vec![],
        };
        assert!(add_recipe(&pool, r).await.is_err());
    }

    #[tokio::test]
    async fn test_add_recipe_negative_ingredient_quantity() {
        let pool = setup().await;
        let r = Recipe {
            id: 0, name: "Bad".to_string(), source_url: None,
            ingredients: vec![Ingredient { name: "X".to_string(), quantity: -1.0, unit: "g".to_string() }],
            instructions: vec![],
        };
        assert!(add_recipe(&pool, r).await.is_err());
    }

    #[tokio::test]
    async fn test_get_recipe_by_id_found() {
        let pool = setup().await;
        add_recipe(&pool, bare_recipe("Soup")).await.unwrap();
        let id = get_all_recipes(&pool).await[0].id;
        let found = get_recipe_by_id(&pool, id).await;
        assert!(found.is_some());
        assert_eq!(found.unwrap().name, "Soup");
    }

    #[tokio::test]
    async fn test_get_recipe_by_id_not_found() {
        let pool = setup().await;
        assert!(get_recipe_by_id(&pool, 999_999).await.is_none());
    }

    #[tokio::test]
    async fn test_get_all_recipes_empty_then_populated() {
        let pool = setup().await;
        assert!(get_all_recipes(&pool).await.is_empty());
        add_recipe(&pool, bare_recipe("Cake")).await.unwrap();
        assert_eq!(get_all_recipes(&pool).await.len(), 1);
    }

    #[tokio::test]
    async fn test_delete_recipe() {
        let pool = setup().await;
        add_recipe(&pool, bare_recipe("Stew")).await.unwrap();
        let id = get_all_recipes(&pool).await[0].id;
        delete_recipe(&pool, id).await.expect("Delete should succeed");
        assert!(get_recipe_by_id(&pool, id).await.is_none());
    }

    #[tokio::test]
    async fn test_update_recipe_valid() {
        let pool = setup().await;
        add_recipe(&pool, bare_recipe("Old Name")).await.unwrap();
        let id = get_all_recipes(&pool).await[0].id;
        let updated = Recipe { id, name: "New Name".to_string(), source_url: None, ingredients: vec![], instructions: vec![] };
        update_recipe(&pool, id, updated).await.expect("Update should succeed");
        assert_eq!(get_recipe_by_id(&pool, id).await.unwrap().name, "New Name");
    }

    #[tokio::test]
    async fn test_update_recipe_invalid_name() {
        let pool = setup().await;
        add_recipe(&pool, bare_recipe("Valid")).await.unwrap();
        let id = get_all_recipes(&pool).await[0].id;
        let bad = Recipe { id, name: "a".repeat(201), source_url: None, ingredients: vec![], instructions: vec![] };
        assert!(update_recipe(&pool, id, bad).await.is_err());
    }

    #[tokio::test]
    async fn test_update_recipe_not_found() {
        let pool = setup().await;
        let r = Recipe { id: 999_999, name: "Ghost".to_string(), source_url: None, ingredients: vec![], instructions: vec![] };
        assert!(update_recipe(&pool, 999_999, r).await.is_err());
    }

    #[tokio::test]
    async fn test_get_user_by_username_found() {
        let pool = setup().await;
        let user = get_user_by_username(&pool, "test").await.unwrap();
        assert!(user.is_some());
        assert_eq!(user.unwrap().username, "test");
    }

    #[tokio::test]
    async fn test_get_user_by_username_not_found() {
        let pool = setup().await;
        assert!(get_user_by_username(&pool, "nobody").await.unwrap().is_none());
    }
}