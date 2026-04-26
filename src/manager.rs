use sqlx::SqlitePool;
use crate::auth;
use crate::model::{Recipe, User, UserInfo};
use crate::storage;
use validator::Validate;

/// Maximum number of recipes a single user may store.
#[cfg(not(test))]
const MAX_RECIPES_PER_USER: usize = 500;
#[cfg(test)]
const MAX_RECIPES_PER_USER: usize = 3;

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

/// Retrieves a recipe by its ID, scoped to the owning user.
///
/// Returns `Some(Recipe)` if found, `None` if the ID does not exist or
/// belongs to a different user.
pub async fn get_recipe_by_id(pool: &SqlitePool, user_id: i64, id: i64) -> Option<Recipe> {
    storage::load_recipe(pool, user_id, id).await.ok()
}

/// Adds a new recipe to storage for the given user.
///
/// # Errors
///
/// Returns `Err` if validation fails or the recipe could not be persisted.
pub async fn add_recipe(pool: &SqlitePool, user_id: i64, recipe: Recipe) -> Result<(), String> {
    recipe.validate().map_err(|e| format!("Validation error: {e}"))?;
    let existing = storage::load_all_recipes(pool, user_id).await?;
    if existing.len() >= MAX_RECIPES_PER_USER {
        return Err(format!("Recipe limit of {} reached", MAX_RECIPES_PER_USER));
    }
    storage::add_recipe(pool, user_id, &recipe).await?;
    Ok(())
}

/// Deletes the recipe at the given ID, scoped to the owning user.
///
/// Deleting a recipe owned by a different user is a no-op — idempotent by design.
///
/// # Errors
///
/// Returns `Err` if the query fails.
pub async fn delete_recipe(pool: &SqlitePool, user_id: i64, id: i64) -> Result<(), String> {
    storage::delete_recipe(pool, user_id, id).await
}

/// Returns all recipes for the given user.
///
/// Returns an empty Vec if the query fails.
pub async fn get_all_recipes(pool: &SqlitePool, user_id: i64) -> Vec<Recipe> {
    storage::load_all_recipes(pool, user_id).await.unwrap_or_default()
}

/// Updates an existing recipe by ID, scoped to the owning user.
///
/// # Errors
///
/// Returns `Err` if validation fails, no recipe exists at `id` for that user,
/// or the query fails.
pub async fn update_recipe(pool: &SqlitePool, user_id: i64, id: i64, recipe: Recipe) -> Result<(), String> {
    recipe.validate().map_err(|e| format!("Validation error: {e}"))?;
    storage::save_recipe(pool, user_id, id, &recipe).await
}

// ---------------------------------------------------------------------------
// Admin — user management
// ---------------------------------------------------------------------------

/// Creates a new non-admin user account.
///
/// Validates that the username is 1–50 characters and contains no whitespace,
/// and that the password is at least 8 characters. The password is hashed
/// before storage; the plaintext is never persisted.
///
/// # Errors
///
/// Returns `Err` if validation fails, the username is already taken, or the
/// query fails.
pub async fn admin_create_user(pool: &SqlitePool, username: &str, password: &str) -> Result<(), String> {
    if username.is_empty() || username.len() > 50 {
        return Err("Username must be between 1 and 50 characters".to_string());
    }
    if username.chars().any(|c| c.is_whitespace()) {
        return Err("Username must not contain whitespace".to_string());
    }
    if password.len() < 8 {
        return Err("password must be at least 8 characters".to_string());
    }
    let hash = auth::hash_password(password)?;
    storage::create_user(pool, username, &hash).await?;
    Ok(())
}

/// Changes a user's password.
///
/// Validates that the new password is at least 8 characters, then hashes and
/// persists it.
///
/// # Errors
///
/// Returns `Err` if the password is too short, the user does not exist, or
/// the query fails.
pub async fn admin_change_password(pool: &SqlitePool, target_user_id: i64, new_password: &str) -> Result<(), String> {
    if new_password.len() < 8 {
        return Err("password must be at least 8 characters".to_string());
    }
    let hash = auth::hash_password(new_password)?;
    storage::update_password(pool, target_user_id, &hash).await
}

/// Returns all registered users as public records (no password hashes).
///
/// # Errors
///
/// Returns `Err` if the query fails.
pub async fn admin_list_users(pool: &SqlitePool) -> Result<Vec<UserInfo>, String> {
    storage::load_all_users(pool).await
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
        pool
    }

    fn bare_recipe(name: &str) -> Recipe {
        Recipe { id: 0, name: name.to_string(), source_url: None, ingredients: vec![], instructions: vec![] }
    }

    #[tokio::test]
    async fn test_add_recipe_valid() {
        let pool = setup().await;
        assert!(add_recipe(&pool, 1, bare_recipe("Pasta")).await.is_ok());
    }

    #[tokio::test]
    async fn test_add_recipe_name_too_long() {
        let pool = setup().await;
        let result = add_recipe(&pool, 1, bare_recipe(&"a".repeat(201))).await;
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
        assert!(add_recipe(&pool, 1, r).await.is_err());
    }

    #[tokio::test]
    async fn test_add_recipe_negative_ingredient_quantity() {
        let pool = setup().await;
        let r = Recipe {
            id: 0, name: "Bad".to_string(), source_url: None,
            ingredients: vec![Ingredient { name: "X".to_string(), quantity: -1.0, unit: "g".to_string() }],
            instructions: vec![],
        };
        assert!(add_recipe(&pool, 1, r).await.is_err());
    }

    #[tokio::test]
    async fn test_get_recipe_by_id_found() {
        let pool = setup().await;
        add_recipe(&pool, 1, bare_recipe("Soup")).await.unwrap();
        let id = get_all_recipes(&pool, 1).await[0].id;
        let found = get_recipe_by_id(&pool, 1, id).await;
        assert!(found.is_some());
        assert_eq!(found.unwrap().name, "Soup");
    }

    #[tokio::test]
    async fn test_get_recipe_by_id_not_found() {
        let pool = setup().await;
        assert!(get_recipe_by_id(&pool, 1, 999_999).await.is_none());
    }

    #[tokio::test]
    async fn test_get_all_recipes_empty_then_populated() {
        let pool = setup().await;
        assert!(get_all_recipes(&pool, 1).await.is_empty());
        add_recipe(&pool, 1, bare_recipe("Cake")).await.unwrap();
        assert_eq!(get_all_recipes(&pool, 1).await.len(), 1);
    }

    #[tokio::test]
    async fn test_delete_recipe() {
        let pool = setup().await;
        add_recipe(&pool, 1, bare_recipe("Stew")).await.unwrap();
        let id = get_all_recipes(&pool, 1).await[0].id;
        delete_recipe(&pool, 1, id).await.expect("Delete should succeed");
        assert!(get_recipe_by_id(&pool, 1, id).await.is_none());
    }

    #[tokio::test]
    async fn test_update_recipe_valid() {
        let pool = setup().await;
        add_recipe(&pool, 1, bare_recipe("Old Name")).await.unwrap();
        let id = get_all_recipes(&pool, 1).await[0].id;
        let updated = Recipe { id, name: "New Name".to_string(), source_url: None, ingredients: vec![], instructions: vec![] };
        update_recipe(&pool, 1, id, updated).await.expect("Update should succeed");
        assert_eq!(get_recipe_by_id(&pool, 1, id).await.unwrap().name, "New Name");
    }

    #[tokio::test]
    async fn test_update_recipe_invalid_name() {
        let pool = setup().await;
        add_recipe(&pool, 1, bare_recipe("Valid")).await.unwrap();
        let id = get_all_recipes(&pool, 1).await[0].id;
        let bad = Recipe { id, name: "a".repeat(201), source_url: None, ingredients: vec![], instructions: vec![] };
        assert!(update_recipe(&pool, 1, id, bad).await.is_err());
    }

    #[tokio::test]
    async fn test_update_recipe_not_found() {
        let pool = setup().await;
        let r = Recipe { id: 999_999, name: "Ghost".to_string(), source_url: None, ingredients: vec![], instructions: vec![] };
        assert!(update_recipe(&pool, 1, 999_999, r).await.is_err());
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

    #[tokio::test]
    async fn test_recipe_quota_enforced() {
        let pool = setup().await;
        for i in 0..MAX_RECIPES_PER_USER {
            add_recipe(&pool, 1, bare_recipe(&format!("Recipe {i}"))).await
                .expect("should succeed within quota");
        }
        let result = add_recipe(&pool, 1, bare_recipe("One Too Many")).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("limit"));
    }

    #[tokio::test]
    async fn test_admin_create_user_valid() {
        let pool = setup().await;
        admin_create_user(&pool, "alice", "securepassword").await.expect("should create user");
        let user = get_user_by_username(&pool, "alice").await.unwrap();
        assert!(user.is_some());
        assert_eq!(user.unwrap().username, "alice");
    }

    #[tokio::test]
    async fn test_admin_create_user_duplicate_username() {
        let pool = setup().await;
        admin_create_user(&pool, "bob", "password123").await.expect("first should succeed");
        let result = admin_create_user(&pool, "bob", "password456").await;
        assert!(result.is_err(), "Duplicate username should fail");
    }

    #[tokio::test]
    async fn test_admin_create_user_password_too_short() {
        let pool = setup().await;
        let result = admin_create_user(&pool, "charlie", "short").await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("password"));
    }

    #[tokio::test]
    async fn test_admin_change_password_valid() {
        let pool = setup().await;
        admin_create_user(&pool, "diana", "oldpassword").await.unwrap();
        let user = get_user_by_username(&pool, "diana").await.unwrap().unwrap();
        admin_change_password(&pool, user.id, "newpassword1").await.expect("change should succeed");
        let updated = get_user_by_username(&pool, "diana").await.unwrap().unwrap();
        assert!(crate::auth::verify_password("newpassword1", &updated.password_hash).unwrap());
        assert!(!crate::auth::verify_password("oldpassword", &updated.password_hash).unwrap());
    }

    #[tokio::test]
    async fn test_recipe_isolation_between_users() {
        let pool = setup().await;
        // Insert a second user
        sqlx::query("INSERT INTO users (id, username, password_hash) VALUES (2, 'user2', 'placeholder')")
            .execute(&pool).await.unwrap();
        // User 1 adds a recipe
        add_recipe(&pool, 1, bare_recipe("User1 Recipe")).await.unwrap();
        // User 2 should see no recipes
        assert!(get_all_recipes(&pool, 2).await.is_empty());
        // User 1 should see their recipe
        assert_eq!(get_all_recipes(&pool, 1).await.len(), 1);
    }
}
