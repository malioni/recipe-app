/// Storage module responsible for all recipe persistence.
///
/// This module owns the storage backend entirely. No other module should know
/// about SQL queries, table names, or how recipes are physically stored.
/// When the backend changes (e.g. SQLite → Postgres), only this file changes.
use sqlx::SqlitePool;
use crate::model::{Ingredient, Recipe, User};

// ---------------------------------------------------------------------------
// Users
// ---------------------------------------------------------------------------

/// Loads a user by username.
///
/// Used by the login flow to retrieve the stored password hash for
/// verification. Returns `None` if no user exists with that username.
///
/// # Errors
///
/// Returns `Err` if the query fails.
pub async fn load_user_by_username(pool: &SqlitePool, username: &str) -> Result<Option<User>, String> {
    let row = sqlx::query!(
        "SELECT id, username, password_hash FROM users WHERE username = ?",
        username
    )
    .fetch_optional(pool)
    .await
    .map_err(|e| format!("Failed to query user: {e}"))?;

    Ok(row.map(|r| User {
        id: r.id.unwrap_or(0),
        username: r.username,
        password_hash: r.password_hash,
    }))
}

/// Inserts a new user with a pre-hashed password.
///
/// The caller is responsible for hashing the password before calling this.
/// Returns the assigned user ID.
///
/// # Errors
///
/// Returns `Err` if the username is already taken or the query fails.
pub async fn create_user(pool: &SqlitePool, username: &str, password_hash: &str) -> Result<i64, String> {
    let result = sqlx::query!(
        "INSERT INTO users (username, password_hash) VALUES (?, ?)",
        username,
        password_hash,
    )
    .execute(pool)
    .await
    .map_err(|e| format!("Failed to create user: {e}"))?;

    Ok(result.last_insert_rowid())
}

/// Returns true if the users table contains at least one row.
///
/// Used on startup to decide whether to seed the initial user from
/// environment variables.
///
/// # Errors
///
/// Returns `Err` if the query fails.
pub async fn any_users_exist(pool: &SqlitePool) -> Result<bool, String> {
    let row = sqlx::query!("SELECT COUNT(*) as count FROM users")
        .fetch_one(pool)
        .await
        .map_err(|e| format!("Failed to count users: {e}"))?;

    Ok(row.count > 0)
}

// ---------------------------------------------------------------------------
// Recipes
// ---------------------------------------------------------------------------

/// Loads a single recipe by its ID.
///
/// # Errors
///
/// Returns `Err` if the query fails or no recipe exists at the given ID.
pub async fn load_recipe(pool: &SqlitePool, id: i64) -> Result<Recipe, String> {
    let row = sqlx::query!(
        "SELECT id, name, source_url, ingredients, instructions FROM recipes WHERE id = ?",
        id
    )
    .fetch_optional(pool)
    .await
    .map_err(|e| format!("Failed to query recipe: {e}"))?
    .ok_or_else(|| format!("Recipe with ID {} not found", id))?;

    Ok(Recipe {
        id: row.id,
        name: row.name,
        source_url: row.source_url,
        ingredients: parse_ingredients(&row.ingredients)?,
        instructions: parse_instructions(&row.instructions)?,
    })
}

/// Loads every recipe from storage for the given user.
///
/// # Errors
///
/// Returns `Err` if the query fails.
pub async fn load_all_recipes(pool: &SqlitePool, user_id: i64) -> Result<Vec<Recipe>, String> {
    let rows = sqlx::query!(
        "SELECT id, name, source_url, ingredients, instructions
         FROM recipes WHERE user_id = ? ORDER BY id",
        user_id
    )
    .fetch_all(pool)
    .await
    .map_err(|e| format!("Failed to query recipes: {e}"))?;

    rows.into_iter()
        .map(|row| {
            Ok(Recipe {
                id: row.id,
                name: row.name,
                source_url: row.source_url,
                ingredients: parse_ingredients(&row.ingredients)?,
                instructions: parse_instructions(&row.instructions)?,
            })
        })
        .collect()
}

/// Inserts a new recipe for the given user, returning the assigned ID.
///
/// # Errors
///
/// Returns `Err` if serialization or the insert query fails.
pub async fn add_recipe(pool: &SqlitePool, user_id: i64, recipe: &Recipe) -> Result<i64, String> {
    let ingredients_json = serde_json::to_string(&recipe.ingredients)
        .map_err(|e| format!("Failed to serialize ingredients: {e}"))?;
    let instructions_json = serde_json::to_string(&recipe.instructions)
        .map_err(|e| format!("Failed to serialize instructions: {e}"))?;

    let result = sqlx::query!(
        "INSERT INTO recipes (user_id, name, source_url, ingredients, instructions)
         VALUES (?, ?, ?, ?, ?)",
        user_id,
        recipe.name,
        recipe.source_url,
        ingredients_json,
        instructions_json,
    )
    .execute(pool)
    .await
    .map_err(|e| format!("Failed to insert recipe: {e}"))?;

    Ok(result.last_insert_rowid())
}

/// Replaces the recipe at `id` with the provided data.
///
/// # Errors
///
/// Returns `Err` if no recipe exists at `id` or the update query fails.
pub async fn save_recipe(pool: &SqlitePool, id: i64, recipe: &Recipe) -> Result<(), String> {
    let ingredients_json = serde_json::to_string(&recipe.ingredients)
        .map_err(|e| format!("Failed to serialize ingredients: {e}"))?;
    let instructions_json = serde_json::to_string(&recipe.instructions)
        .map_err(|e| format!("Failed to serialize instructions: {e}"))?;

    let result = sqlx::query!(
        "UPDATE recipes SET name = ?, source_url = ?, ingredients = ?, instructions = ?
         WHERE id = ?",
        recipe.name,
        recipe.source_url,
        ingredients_json,
        instructions_json,
        id,
    )
    .execute(pool)
    .await
    .map_err(|e| format!("Failed to update recipe: {e}"))?;

    if result.rows_affected() == 0 {
        return Err(format!("Recipe with ID {} not found", id));
    }
    Ok(())
}

/// Deletes the recipe at `id`.
///
/// meal_plan and cooked_log entries referencing this recipe are removed
/// automatically via ON DELETE CASCADE.
///
/// # Errors
///
/// Returns `Err` if no recipe exists at `id` or the delete query fails.
pub async fn delete_recipe(pool: &SqlitePool, id: i64) -> Result<(), String> {
    let result = sqlx::query!("DELETE FROM recipes WHERE id = ?", id)
        .execute(pool)
        .await
        .map_err(|e| format!("Failed to delete recipe: {e}"))?;

    if result.rows_affected() == 0 {
        return Err(format!("Recipe with ID {} not found", id));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Private helpers
// ---------------------------------------------------------------------------

fn parse_ingredients(json: &str) -> Result<Vec<Ingredient>, String> {
    serde_json::from_str(json)
        .map_err(|e| format!("Failed to parse ingredients: {e}"))
}

fn parse_instructions(json: &str) -> Result<Vec<String>, String> {
    serde_json::from_str(json)
        .map_err(|e| format!("Failed to parse instructions: {e}"))
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
        sqlx::query(
            "INSERT INTO users (id, username, password_hash) VALUES (1, 'test', 'placeholder')"
        )
        .execute(&pool)
        .await
        .expect("Failed to insert test user");
        pool
    }

    #[tokio::test]
    async fn test_any_users_exist_true() {
        let pool = setup().await;
        assert!(any_users_exist(&pool).await.unwrap());
    }

    #[tokio::test]
    async fn test_any_users_exist_false() {
        let pool = SqlitePool::connect(":memory:").await.unwrap();
        sqlx::query(include_str!("../migrations/001_initial.sql"))
            .execute(&pool).await.unwrap();
        assert!(!any_users_exist(&pool).await.unwrap());
    }

    #[tokio::test]
    async fn test_load_user_by_username_found() {
        let pool = setup().await;
        let user = load_user_by_username(&pool, "test").await.unwrap();
        assert!(user.is_some());
        assert_eq!(user.unwrap().username, "test");
    }

    #[tokio::test]
    async fn test_load_user_by_username_not_found() {
        let pool = setup().await;
        let user = load_user_by_username(&pool, "nobody").await.unwrap();
        assert!(user.is_none());
    }

    #[tokio::test]
    async fn test_add_and_load_recipe() {
        let pool = setup().await;
        let recipe = Recipe {
            id: 0,
            name: "Pasta".to_string(),
            source_url: None,
            ingredients: vec![],
            instructions: vec!["Boil water".to_string()],
        };
        let id = add_recipe(&pool, 1, &recipe).await.expect("Failed to add recipe");
        let loaded = load_recipe(&pool, id).await.expect("Failed to load recipe");
        assert_eq!(loaded.name, "Pasta");
        assert_eq!(loaded.instructions, vec!["Boil water"]);
    }

    #[tokio::test]
    async fn test_load_recipe_invalid_id() {
        let pool = setup().await;
        let result = load_recipe(&pool, 999_999).await;
        assert!(result.is_err(), "Expected an error for a missing ID");
    }

    #[tokio::test]
    async fn test_load_all_recipes_returns_vec() {
        let pool = setup().await;
        let recipe = Recipe {
            id: 0,
            name: "Salad".to_string(),
            source_url: Some("https://example.com".to_string()),
            ingredients: vec![],
            instructions: vec![],
        };
        add_recipe(&pool, 1, &recipe).await.expect("Failed to add recipe");
        let recipes = load_all_recipes(&pool, 1).await.expect("Failed to load all recipes");
        assert!(!recipes.is_empty());
        assert_eq!(recipes[0].source_url, Some("https://example.com".to_string()));
    }

    #[tokio::test]
    async fn test_delete_recipe() {
        let pool = setup().await;
        let recipe = Recipe {
            id: 0, name: "To Delete".to_string(), source_url: None,
            ingredients: vec![], instructions: vec![],
        };
        let id = add_recipe(&pool, 1, &recipe).await.expect("Failed to add recipe");
        delete_recipe(&pool, id).await.expect("Failed to delete recipe");
        assert!(load_recipe(&pool, id).await.is_err());
    }

    #[tokio::test]
    async fn test_save_recipe_updates_fields() {
        let pool = setup().await;
        let recipe = Recipe {
            id: 0, name: "Original".to_string(), source_url: None,
            ingredients: vec![], instructions: vec![],
        };
        let id = add_recipe(&pool, 1, &recipe).await.expect("Failed to add recipe");
        let updated = Recipe {
            id,
            name: "Updated".to_string(),
            source_url: Some("https://updated.com".to_string()),
            ingredients: vec![],
            instructions: vec![],
        };
        save_recipe(&pool, id, &updated).await.expect("Failed to update recipe");
        let loaded = load_recipe(&pool, id).await.expect("Failed to load recipe");
        assert_eq!(loaded.name, "Updated");
        assert_eq!(loaded.source_url, Some("https://updated.com".to_string()));
    }
}