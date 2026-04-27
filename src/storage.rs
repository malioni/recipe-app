/// Storage module responsible for all recipe persistence.
///
/// This module owns the storage backend entirely. No other module should know
/// about SQL queries, table names, or how recipes are physically stored.
/// When the backend changes (e.g. SQLite → Postgres), only this file changes.
use sqlx::SqlitePool;
use crate::model::{Ingredient, Recipe, User, UserInfo};

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
        "SELECT id, username, password_hash, is_admin FROM users WHERE username = ?",
        username
    )
    .fetch_optional(pool)
    .await
    .map_err(|e| format!("Failed to query user: {e}"))?;

    Ok(row.map(|r| User {
        id: r.id.unwrap_or(0),
        username: r.username,
        password_hash: r.password_hash,
        is_admin: r.is_admin != 0,
    }))
}

/// Loads a user by their primary key.
///
/// Returns `None` if no user exists with that ID.
///
/// # Errors
///
/// Returns `Err` if the query fails.
pub async fn load_user_by_id(pool: &SqlitePool, user_id: i64) -> Result<Option<User>, String> {
    let row = sqlx::query!(
        "SELECT id, username, password_hash, is_admin FROM users WHERE id = ?",
        user_id
    )
    .fetch_optional(pool)
    .await
    .map_err(|e| format!("Failed to query user: {e}"))?;

    Ok(row.map(|r| User {
        id: r.id,
        username: r.username,
        password_hash: r.password_hash,
        is_admin: r.is_admin != 0,
    }))
}

/// Returns all users as public `UserInfo` records (no password hashes).
///
/// # Errors
///
/// Returns `Err` if the query fails.
pub async fn load_all_users(pool: &SqlitePool) -> Result<Vec<UserInfo>, String> {
    let rows = sqlx::query!(
        "SELECT id, username, is_admin, created_at FROM users ORDER BY id"
    )
    .fetch_all(pool)
    .await
    .map_err(|e| format!("Failed to query users: {e}"))?;

    Ok(rows.into_iter().map(|r| UserInfo {
        id: r.id,
        username: r.username,
        is_admin: r.is_admin != 0,
        created_at: r.created_at,
    }).collect())
}

/// Sets `is_admin = 1` for the given user.
///
/// Called once at first-boot after the initial user is created, so the
/// seeded account has admin privileges without needing a separate step.
///
/// # Errors
///
/// Returns `Err` if the query fails.
pub async fn promote_user_to_admin(pool: &SqlitePool, user_id: i64) -> Result<(), String> {
    sqlx::query!(
        "UPDATE users SET is_admin = 1 WHERE id = ?",
        user_id
    )
    .execute(pool)
    .await
    .map_err(|e| format!("Failed to promote user to admin: {e}"))?;

    Ok(())
}

/// Updates the stored password hash for the given user.
///
/// The caller is responsible for hashing the new password before calling this.
///
/// # Errors
///
/// Returns `Err` if no user exists at `user_id` or the query fails.
pub async fn update_password(pool: &SqlitePool, user_id: i64, password_hash: &str) -> Result<(), String> {
    let result = sqlx::query!(
        "UPDATE users SET password_hash = ? WHERE id = ?",
        password_hash,
        user_id
    )
    .execute(pool)
    .await
    .map_err(|e| format!("Failed to update password: {e}"))?;

    if result.rows_affected() == 0 {
        return Err(format!("User with ID {} not found", user_id));
    }
    Ok(())
}

/// Deletes the user with the given ID.
///
/// All of the user's recipes, meal plan entries, and cooked log entries are
/// removed automatically via `ON DELETE CASCADE`. Deleting a non-existent ID
/// is a no-op — idempotent by design.
///
/// # Errors
///
/// Returns `Err` if the query fails.
pub async fn delete_user(pool: &SqlitePool, user_id: i64) -> Result<(), String> {
    sqlx::query!("DELETE FROM users WHERE id = ?", user_id)
        .execute(pool)
        .await
        .map_err(|e| format!("Failed to delete user: {e}"))?;
    Ok(())
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

/// Loads a single recipe by its ID, scoped to the owning user.
///
/// Returns `Err` if the recipe does not exist or belongs to a different user.
///
/// # Errors
///
/// Returns `Err` if the query fails or no matching recipe exists.
pub async fn load_recipe(pool: &SqlitePool, user_id: i64, id: i64) -> Result<Recipe, String> {
    let row = sqlx::query!(
        "SELECT id, name, source_url, ingredients, instructions FROM recipes WHERE id = ? AND user_id = ?",
        id,
        user_id
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

/// Replaces the recipe at `id` with the provided data, scoped to the owning user.
///
/// Returns `Err` if no recipe exists at `id` for that user, or the update query fails.
///
/// # Errors
///
/// Returns `Err` if the query fails or `rows_affected == 0`.
pub async fn save_recipe(pool: &SqlitePool, user_id: i64, id: i64, recipe: &Recipe) -> Result<(), String> {
    let ingredients_json = serde_json::to_string(&recipe.ingredients)
        .map_err(|e| format!("Failed to serialize ingredients: {e}"))?;
    let instructions_json = serde_json::to_string(&recipe.instructions)
        .map_err(|e| format!("Failed to serialize instructions: {e}"))?;

    let result = sqlx::query!(
        "UPDATE recipes SET name = ?, source_url = ?, ingredients = ?, instructions = ?
         WHERE id = ? AND user_id = ?",
        recipe.name,
        recipe.source_url,
        ingredients_json,
        instructions_json,
        id,
        user_id,
    )
    .execute(pool)
    .await
    .map_err(|e| format!("Failed to update recipe: {e}"))?;

    if result.rows_affected() == 0 {
        return Err(format!("Recipe with ID {} not found", id));
    }
    Ok(())
}

/// Deletes the recipe at `id`, scoped to the owning user.
///
/// meal_plan and cooked_log entries referencing this recipe are removed
/// automatically via ON DELETE CASCADE. Deleting a non-existent id or one
/// owned by a different user is a no-op — idempotent by design.
///
/// # Errors
///
/// Returns `Err` if the delete query fails.
pub async fn delete_recipe(pool: &SqlitePool, user_id: i64, id: i64) -> Result<(), String> {
    sqlx::query!("DELETE FROM recipes WHERE id = ? AND user_id = ?", id, user_id)
        .execute(pool)
        .await
        .map_err(|e| format!("Failed to delete recipe: {e}"))?;

    Ok(())
}

// ---------------------------------------------------------------------------
// Schema migrations
// ---------------------------------------------------------------------------

/// Creates the `_schema_migrations` tracking table if it does not already exist.
///
/// Must be called once at startup before any call to `is_migration_applied` or
/// `record_migration`. Uses `CREATE TABLE IF NOT EXISTS` so it is safe to call
/// on every boot.
///
/// # Errors
///
/// Returns `Err` if the query fails.
pub async fn ensure_migrations_table(pool: &SqlitePool) -> Result<(), String> {
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS _schema_migrations (
             version    TEXT PRIMARY KEY,
             applied_at TEXT NOT NULL DEFAULT (datetime('now'))
         )",
    )
    .execute(pool)
    .await
    .map_err(|e| format!("Failed to create _schema_migrations table: {e}"))?;

    Ok(())
}

/// Returns `true` if the given migration version has already been recorded in
/// `_schema_migrations`.
///
/// Call `ensure_migrations_table` before this function.
///
/// # Errors
///
/// Returns `Err` if the query fails.
pub async fn is_migration_applied(pool: &SqlitePool, version: &str) -> Result<bool, String> {
    let row = sqlx::query("SELECT version FROM _schema_migrations WHERE version = ?")
        .bind(version)
        .fetch_optional(pool)
        .await
        .map_err(|e| format!("Failed to check migration {version}: {e}"))?;

    Ok(row.is_some())
}

/// Records a migration version in `_schema_migrations` after it has been applied.
///
/// Returns `Err` if the version is already recorded (PRIMARY KEY violation) or
/// the query otherwise fails.
///
/// # Errors
///
/// Returns `Err` if the query fails.
pub async fn record_migration(pool: &SqlitePool, version: &str) -> Result<(), String> {
    sqlx::query("INSERT INTO _schema_migrations (version) VALUES (?)")
        .bind(version)
        .execute(pool)
        .await
        .map_err(|e| format!("Failed to record migration {version}: {e}"))?;

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
            .execute(&pool).await.expect("Failed to run migration 001");
        sqlx::query(include_str!("../migrations/002_multiple_entries_per_slot.sql"))
            .execute(&pool).await.expect("Failed to run migration 002");
        sqlx::query(include_str!("../migrations/003_add_portions_to_meal_plan.sql"))
            .execute(&pool).await.expect("Failed to run migration 003");
        sqlx::query(include_str!("../migrations/004_add_is_admin_to_users.sql"))
            .execute(&pool).await.expect("Failed to run migration 004");
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
        sqlx::query(include_str!("../migrations/002_multiple_entries_per_slot.sql"))
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
        let loaded = load_recipe(&pool, 1, id).await.expect("Failed to load recipe");
        assert_eq!(loaded.name, "Pasta");
        assert_eq!(loaded.instructions, vec!["Boil water"]);
    }

    #[tokio::test]
    async fn test_load_recipe_invalid_id() {
        let pool = setup().await;
        let result = load_recipe(&pool, 1, 999_999).await;
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
        delete_recipe(&pool, 1, id).await.expect("Failed to delete recipe");
        assert!(load_recipe(&pool, 1, id).await.is_err());
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
        save_recipe(&pool, 1, id, &updated).await.expect("Failed to update recipe");
        let loaded = load_recipe(&pool, 1, id).await.expect("Failed to load recipe");
        assert_eq!(loaded.name, "Updated");
        assert_eq!(loaded.source_url, Some("https://updated.com".to_string()));
    }

    #[tokio::test]
    async fn test_create_user() {
        let pool = SqlitePool::connect(":memory:").await.unwrap();
        sqlx::query(include_str!("../migrations/001_initial.sql"))
            .execute(&pool).await.unwrap();
        sqlx::query(include_str!("../migrations/002_multiple_entries_per_slot.sql"))
            .execute(&pool).await.unwrap();
        sqlx::query(include_str!("../migrations/003_add_portions_to_meal_plan.sql"))
            .execute(&pool).await.unwrap();
        sqlx::query(include_str!("../migrations/004_add_is_admin_to_users.sql"))
            .execute(&pool).await.unwrap();
        let id = create_user(&pool, "alice", "hash123").await.expect("Failed to create user");
        assert!(id > 0);
        let user = load_user_by_username(&pool, "alice").await.unwrap().unwrap();
        assert_eq!(user.username, "alice");
        assert_eq!(user.password_hash, "hash123");
    }

    #[tokio::test]
    async fn test_create_user_duplicate_fails() {
        let pool = setup().await;
        // "test" user already inserted by setup()
        let result = create_user(&pool, "test", "anotherhash").await;
        assert!(result.is_err(), "Duplicate username should fail");
    }

    #[tokio::test]
    async fn test_save_recipe_not_found() {
        let pool = setup().await;
        let recipe = Recipe {
            id: 0, name: "Ghost".to_string(), source_url: None,
            ingredients: vec![], instructions: vec![],
        };
        let result = save_recipe(&pool, 1, 999_999, &recipe).await;
        assert!(result.is_err(), "Updating a non-existent recipe should fail");
    }

    #[tokio::test]
    async fn test_delete_recipe_not_found() {
        let pool = setup().await;
        // Deleting a non-existent ID is a no-op — idempotent by design.
        assert!(delete_recipe(&pool, 1, 999_999).await.is_ok());
    }

    #[tokio::test]
    async fn test_add_recipe_with_ingredients_roundtrip() {
        let pool = setup().await;
        let recipe = Recipe {
            id: 0,
            name: "Soup".to_string(),
            source_url: None,
            ingredients: vec![
                Ingredient { name: "Water".to_string(), quantity: 1.5, unit: "L".to_string() },
                Ingredient { name: "Salt".to_string(), quantity: 5.0, unit: "g".to_string() },
            ],
            instructions: vec!["Boil water".to_string(), "Add salt".to_string()],
        };
        let id = add_recipe(&pool, 1, &recipe).await.unwrap();
        let loaded = load_recipe(&pool, 1, id).await.unwrap();
        assert_eq!(loaded.ingredients.len(), 2);
        assert_eq!(loaded.ingredients[0].name, "Water");
        assert!((loaded.ingredients[0].quantity - 1.5).abs() < f32::EPSILON);
        assert_eq!(loaded.ingredients[1].unit, "g");
        assert_eq!(loaded.instructions, vec!["Boil water", "Add salt"]);
    }

    #[tokio::test]
    async fn test_load_all_recipes_ordered_by_id() {
        let pool = setup().await;
        for name in ["Zucchini Soup", "Apple Pie", "Bread"] {
            let r = Recipe { id: 0, name: name.to_string(), source_url: None, ingredients: vec![], instructions: vec![] };
            add_recipe(&pool, 1, &r).await.unwrap();
        }
        let recipes = load_all_recipes(&pool, 1).await.unwrap();
        assert_eq!(recipes.len(), 3);
        // ORDER BY id means insertion order is preserved
        assert_eq!(recipes[0].name, "Zucchini Soup");
        assert_eq!(recipes[2].name, "Bread");
    }

    // -------------------------------------------------------------------------
    // Migration tracking tests (bare pool — no app schema needed)
    // -------------------------------------------------------------------------

    #[tokio::test]
    async fn test_load_user_by_id_found() {
        let pool = setup().await;
        let user = load_user_by_id(&pool, 1).await.unwrap();
        assert!(user.is_some());
        assert_eq!(user.unwrap().username, "test");
    }

    #[tokio::test]
    async fn test_load_user_by_id_not_found() {
        let pool = setup().await;
        let user = load_user_by_id(&pool, 999_999).await.unwrap();
        assert!(user.is_none());
    }

    #[tokio::test]
    async fn test_load_all_users_returns_list() {
        let pool = setup().await;
        create_user(&pool, "alice", "hash1").await.unwrap();
        create_user(&pool, "bob", "hash2").await.unwrap();
        let users = load_all_users(&pool).await.unwrap();
        // setup() inserts user id=1 ("test") plus alice and bob
        assert_eq!(users.len(), 3);
        assert!(users.iter().any(|u| u.username == "alice"));
        assert!(users.iter().any(|u| u.username == "bob"));
        // password hashes must never appear in UserInfo
    }

    #[tokio::test]
    async fn test_promote_user_to_admin() {
        let pool = setup().await;
        let id = create_user(&pool, "candidate", "hash").await.unwrap();
        let before = load_user_by_id(&pool, id).await.unwrap().unwrap();
        assert!(!before.is_admin);
        promote_user_to_admin(&pool, id).await.unwrap();
        let after = load_user_by_id(&pool, id).await.unwrap().unwrap();
        assert!(after.is_admin);
    }

    #[tokio::test]
    async fn test_update_password() {
        let pool = setup().await;
        let id = create_user(&pool, "pwuser", "oldhash").await.unwrap();
        update_password(&pool, id, "newhash").await.unwrap();
        let user = load_user_by_id(&pool, id).await.unwrap().unwrap();
        assert_eq!(user.password_hash, "newhash");
    }

    #[tokio::test]
    async fn test_update_password_nonexistent_user() {
        let pool = setup().await;
        let result = update_password(&pool, 999_999, "hash").await;
        assert!(result.is_err(), "Updating a non-existent user should fail");
    }

    #[tokio::test]
    async fn test_ensure_migrations_table_is_idempotent() {
        let pool = SqlitePool::connect(":memory:").await.unwrap();
        ensure_migrations_table(&pool).await.expect("first call should succeed");
        ensure_migrations_table(&pool).await.expect("second call should be a no-op");
    }

    #[tokio::test]
    async fn test_migration_tracking_lifecycle() {
        let pool = SqlitePool::connect(":memory:").await.unwrap();
        ensure_migrations_table(&pool).await.unwrap();

        // Before recording: not applied
        assert!(!is_migration_applied(&pool, "001").await.unwrap());

        // Record and verify
        record_migration(&pool, "001").await.expect("first record should succeed");
        assert!(is_migration_applied(&pool, "001").await.unwrap());

        // Double-recording violates PRIMARY KEY — must return Err
        assert!(
            record_migration(&pool, "001").await.is_err(),
            "recording the same version twice should fail"
        );

        // Other versions are unaffected
        assert!(!is_migration_applied(&pool, "002").await.unwrap());
    }

    #[tokio::test]
    async fn test_delete_user() {
        let pool = setup().await;
        let id = create_user(&pool, "todelete", "hash").await.unwrap();
        delete_user(&pool, id).await.expect("delete should succeed");
        assert!(load_user_by_id(&pool, id).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn test_delete_user_not_found() {
        let pool = setup().await;
        // Deleting a non-existent user is a no-op — idempotent by design.
        assert!(delete_user(&pool, 999_999).await.is_ok());
    }

    #[tokio::test]
    async fn test_delete_user_cascades_recipes() {
        let pool = setup().await;
        let recipe = Recipe {
            id: 0, name: "User Recipe".to_string(), source_url: None,
            ingredients: vec![], instructions: vec![],
        };
        let recipe_id = add_recipe(&pool, 1, &recipe).await.unwrap();
        delete_user(&pool, 1).await.expect("delete should succeed");
        // ON DELETE CASCADE must have removed the recipe
        assert!(load_recipe(&pool, 1, recipe_id).await.is_err());
    }
}
