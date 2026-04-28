/// Integration tests for the recipe app HTTP API.
///
/// These tests compile against the library's public API and exercise the full
/// stack: HTTP request → router → middleware → handler → manager → storage →
/// SQLite → response. They are the only tests that can catch bugs that span
/// multiple modules (e.g. a session cookie not being set correctly, or a
/// DELETE cascading through meal_plan entries).
///
/// # How sessions work in these tests
///
/// `tower::ServiceExt::oneshot` sends a single request to the router. Between
/// requests the session data lives in the shared in-memory SQLite pool, so a
/// Set-Cookie value obtained from POST /login can be re-sent as a Cookie header
/// in subsequent calls and the session middleware will look it up correctly.
///
/// # Rate limiting
///
/// The governor layer is omitted from the test router because it uses
/// `PeerIpKeyExtractor` which requires real socket address information that is
/// not available in `oneshot` calls. Everything else (session, body limit,
/// CSP) is kept identical to production.
use axum::{
    body::Body,
    extract::DefaultBodyLimit,
    http::{Request, StatusCode},
    middleware,
    routing::{delete, get, post},
    Router,
};
use http_body_util::BodyExt;
use recipe_app::{auth, csrf, network, storage};
use sqlx::SqlitePool;
use time::Duration as TimeDuration;
use tower::ServiceExt;
use tower_sessions::{Expiry, SessionManagerLayer};
use tower_sessions_sqlx_store::SqliteStore;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Builds the app router backed by a fresh in-memory SQLite database.
/// A single user `admin` / `password` is created on every call.
async fn build_test_app() -> (Router, SqlitePool) {
    let pool = SqlitePool::connect(":memory:").await.unwrap();
    sqlx::query(include_str!("../migrations/001_initial.sql"))
        .execute(&pool).await.unwrap();
    sqlx::query(include_str!("../migrations/002_multiple_entries_per_slot.sql"))
        .execute(&pool).await.unwrap();
    sqlx::query(include_str!("../migrations/003_add_portions_to_meal_plan.sql"))
        .execute(&pool).await.unwrap();
    sqlx::query(include_str!("../migrations/004_add_is_admin_to_users.sql"))
        .execute(&pool).await.unwrap();

    let hash = auth::hash_password("password").unwrap();
    let admin_id = storage::create_user(&pool, "admin", &hash).await.unwrap();
    storage::promote_user_to_admin(&pool, admin_id).await.unwrap();

    let session_store = SqliteStore::new(pool.clone());
    session_store.migrate().await.unwrap();

    let session_layer = SessionManagerLayer::new(session_store)
        .with_secure(false)
        .with_expiry(Expiry::OnInactivity(TimeDuration::days(1)));

    // Mirror the production router structure: CSRF middleware scoped to the
    // authenticated sub-router only, leaving POST /login unprotected (the
    // session cookie's SameSite=Strict covers login CSRF at the browser level).
    let authenticated = Router::new()
        .route("/logout", post(network::handle_logout))
        .route("/", get(network::handle_index))
        .route("/recipes", get(network::handle_all_recipes).post(network::handle_add_recipe))
        .route("/recipes/new", get(network::handle_new_recipe_page))
        .route("/recipes/:id", get(network::handle_recipe)
            .put(network::handle_update_recipe)
            .delete(network::handle_delete_recipe))
        .route("/calendar/entries", get(network::handle_get_meal_entries)
            .post(network::handle_plan_meal)
            .delete(network::handle_delete_meal_entry))
        .route("/calendar/cooked", get(network::handle_get_cooked_entries)
            .post(network::handle_mark_cooked))
        .route("/calendar/shopping-list", get(network::handle_shopping_list))
        .route("/admin", get(network::handle_admin_page))
        .route("/admin/users", get(network::handle_admin_list_users).post(network::handle_admin_create_user))
        .route("/admin/users/password", post(network::handle_admin_change_password))
        .route("/admin/users/:id", delete(network::handle_admin_delete_user))
        .route("/profile", get(network::handle_profile_page))
        .route("/profile/me", get(network::handle_profile_me))
        .route("/profile/password", post(network::handle_change_own_password))
        .layer(middleware::from_fn(csrf::check_csrf));

    let app = Router::new()
        .merge(authenticated)
        .route("/login", get(network::handle_login_page).post(network::handle_login))
        .fallback(network::handle_404)
        .layer(session_layer)
        .layer(DefaultBodyLimit::max(64 * 1024))
        .with_state(pool.clone());

    (app, pool)
}

/// Logs in as `admin` / `password` and returns the raw `id=<value>` cookie
/// string ready to be used as a `Cookie` request header.
async fn login(app: &Router) -> String {
    let request = Request::builder()
        .method("POST")
        .uri("/login")
        .header("content-type", "application/x-www-form-urlencoded")
        .body(Body::from("username=admin&password=password"))
        .unwrap();

    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(
        response.status(),
        StatusCode::SEE_OTHER,
        "Login should redirect on success"
    );

    // Extract just `id=<value>` from the full Set-Cookie header so we can
    // send it back as a plain Cookie header.
    response
        .headers()
        .get("set-cookie")
        .expect("Login response must set a session cookie")
        .to_str()
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .to_string()
}

fn get_req(uri: &str, cookie: &str) -> Request<Body> {
    Request::builder()
        .method("GET")
        .uri(uri)
        .header("cookie", cookie)
        .body(Body::empty())
        .unwrap()
}

fn json_req(method: &str, uri: &str, cookie: &str, body: serde_json::Value) -> Request<Body> {
    Request::builder()
        .method(method)
        .uri(uri)
        .header("cookie", cookie)
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap()
}

fn delete_req(uri: &str, cookie: &str) -> Request<Body> {
    Request::builder()
        .method("DELETE")
        .uri(uri)
        .header("cookie", cookie)
        .body(Body::empty())
        .unwrap()
}

// ---------------------------------------------------------------------------
// Auth tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_unauthenticated_request_redirects_to_login() {
    let (app, _pool) = build_test_app().await;
    let request = Request::builder()
        .method("GET")
        .uri("/")
        .body(Body::empty())
        .unwrap();
    let response = app.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    assert_eq!(response.headers().get("location").unwrap(), "/login");
}

#[tokio::test]
async fn test_unauthenticated_api_request_redirects_to_login() {
    let (app, _pool) = build_test_app().await;
    let request = Request::builder()
        .method("GET")
        .uri("/recipes")
        .body(Body::empty())
        .unwrap();
    let response = app.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
}

#[tokio::test]
async fn test_login_wrong_password_redirects_with_error() {
    let (app, _pool) = build_test_app().await;
    let request = Request::builder()
        .method("POST")
        .uri("/login")
        .header("content-type", "application/x-www-form-urlencoded")
        .body(Body::from("username=admin&password=wrongpassword"))
        .unwrap();
    let response = app.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    let location = response.headers().get("location").unwrap().to_str().unwrap();
    assert!(location.contains("error=1"), "Wrong password should redirect with error flag");
}

#[tokio::test]
async fn test_login_unknown_user_redirects_with_error() {
    let (app, _pool) = build_test_app().await;
    let request = Request::builder()
        .method("POST")
        .uri("/login")
        .header("content-type", "application/x-www-form-urlencoded")
        .body(Body::from("username=nobody&password=password"))
        .unwrap();
    let response = app.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    let location = response.headers().get("location").unwrap().to_str().unwrap();
    assert!(location.contains("error=1"));
}

// ---------------------------------------------------------------------------
// Recipe CRUD integration test
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_recipe_crud_flow() {
    let (app, _pool) = build_test_app().await;
    let cookie = login(&app).await;

    // Initially no recipes
    let response = app.clone().oneshot(get_req("/recipes", &cookie)).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = response.into_body().collect().await.unwrap().to_bytes();
    let recipes: Vec<serde_json::Value> = serde_json::from_slice(&body).unwrap();
    assert!(recipes.is_empty());

    // Create a recipe
    let new_recipe = serde_json::json!({
        "name": "Integration Test Bread",
        "source_url": null,
        "ingredients": [{"name": "Flour", "quantity": 500.0, "unit": "g"}],
        "instructions": ["Mix", "Bake"]
    });
    let response = app.clone()
        .oneshot(json_req("POST", "/recipes", &cookie, new_recipe))
        .await.unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);

    // List — should now have one recipe
    let response = app.clone().oneshot(get_req("/recipes", &cookie)).await.unwrap();
    let body = response.into_body().collect().await.unwrap().to_bytes();
    let recipes: Vec<serde_json::Value> = serde_json::from_slice(&body).unwrap();
    assert_eq!(recipes.len(), 1);
    let id = recipes[0]["id"].as_i64().unwrap();
    assert_eq!(recipes[0]["name"], "Integration Test Bread");

    // Get by ID
    let response = app.clone()
        .oneshot(get_req(&format!("/recipes/{}", id), &cookie))
        .await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    // Update
    let updated = serde_json::json!({
        "name": "Updated Bread",
        "source_url": "https://example.com/bread",
        "ingredients": [],
        "instructions": []
    });
    let response = app.clone()
        .oneshot(json_req("PUT", &format!("/recipes/{}", id), &cookie, updated))
        .await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    // Verify update
    let response = app.clone()
        .oneshot(get_req(&format!("/recipes/{}", id), &cookie))
        .await.unwrap();
    let body = response.into_body().collect().await.unwrap().to_bytes();
    let recipe: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(recipe["name"], "Updated Bread");
    assert_eq!(recipe["source_url"], "https://example.com/bread");

    // Delete
    let response = app.clone()
        .oneshot(delete_req(&format!("/recipes/{}", id), &cookie))
        .await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    // Verify gone
    let response = app.clone()
        .oneshot(get_req(&format!("/recipes/{}", id), &cookie))
        .await.unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

// ---------------------------------------------------------------------------
// Calendar integration test
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_calendar_plan_and_shopping_list() {
    let (app, _pool) = build_test_app().await;
    let cookie = login(&app).await;

    // Create a recipe with ingredients
    let new_recipe = serde_json::json!({
        "name": "Omelette",
        "source_url": null,
        "ingredients": [
            {"name": "Eggs", "quantity": 3.0, "unit": ""},
            {"name": "Butter", "quantity": 10.0, "unit": "g"}
        ],
        "instructions": ["Beat eggs", "Cook in butter"]
    });
    app.clone()
        .oneshot(json_req("POST", "/recipes", &cookie, new_recipe))
        .await.unwrap();

    let recipes_response = app.clone().oneshot(get_req("/recipes", &cookie)).await.unwrap();
    let body = recipes_response.into_body().collect().await.unwrap().to_bytes();
    let recipes: Vec<serde_json::Value> = serde_json::from_slice(&body).unwrap();
    let recipe_id = recipes[0]["id"].as_i64().unwrap();

    // Plan the meal
    let plan_body = serde_json::json!({
        "date": "2026-05-01",
        "slot": "breakfast",
        "recipe_id": recipe_id
    });
    let response = app.clone()
        .oneshot(json_req("POST", "/calendar/entries", &cookie, plan_body))
        .await.unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);

    // Retrieve planned entries
    let response = app.clone()
        .oneshot(get_req("/calendar/entries?start=2026-05-01&end=2026-05-01", &cookie))
        .await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = response.into_body().collect().await.unwrap().to_bytes();
    let entries: Vec<serde_json::Value> = serde_json::from_slice(&body).unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0]["slot"], "breakfast");

    // Shopping list should contain the recipe's ingredients
    let response = app.clone()
        .oneshot(get_req("/calendar/shopping-list?start=2026-05-01&end=2026-05-01", &cookie))
        .await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = response.into_body().collect().await.unwrap().to_bytes();
    let ingredients: Vec<serde_json::Value> = serde_json::from_slice(&body).unwrap();
    assert_eq!(ingredients.len(), 2);

    // Mark as cooked
    let cooked_body = serde_json::json!({"date": "2026-05-01", "recipe_id": recipe_id});
    let response = app.clone()
        .oneshot(json_req("POST", "/calendar/cooked", &cookie, cooked_body))
        .await.unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);

    // Verify cooked entry appears
    let response = app.clone()
        .oneshot(get_req("/calendar/cooked?start=2026-05-01&end=2026-05-01", &cookie))
        .await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = response.into_body().collect().await.unwrap().to_bytes();
    let cooked: Vec<serde_json::Value> = serde_json::from_slice(&body).unwrap();
    assert_eq!(cooked.len(), 1);
    assert_eq!(cooked[0]["recipe_id"], recipe_id);
}

// ---------------------------------------------------------------------------
// Validation rejection test
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_invalid_recipe_rejected() {
    let (app, _pool) = build_test_app().await;
    let cookie = login(&app).await;

    // Name exceeds 200-character limit
    let bad_recipe = serde_json::json!({
        "name": "a".repeat(201),
        "source_url": null,
        "ingredients": [],
        "instructions": []
    });
    let response = app.clone()
        .oneshot(json_req("POST", "/recipes", &cookie, bad_recipe))
        .await.unwrap();
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
}

// ---------------------------------------------------------------------------
// Cascade delete test — deleting a recipe removes its meal plan entries
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_delete_recipe_cascades_to_meal_plan() {
    let (app, _pool) = build_test_app().await;
    let cookie = login(&app).await;

    // Create recipe and plan it
    let new_recipe = serde_json::json!({
        "name": "Temporary Recipe", "source_url": null,
        "ingredients": [], "instructions": []
    });
    app.clone()
        .oneshot(json_req("POST", "/recipes", &cookie, new_recipe))
        .await.unwrap();

    let body = app.clone()
        .oneshot(get_req("/recipes", &cookie))
        .await.unwrap()
        .into_body().collect().await.unwrap().to_bytes();
    let recipes: Vec<serde_json::Value> = serde_json::from_slice(&body).unwrap();
    let id = recipes[0]["id"].as_i64().unwrap();

    let plan_body = serde_json::json!({"date": "2026-06-01", "slot": "dinner", "recipe_id": id});
    app.clone()
        .oneshot(json_req("POST", "/calendar/entries", &cookie, plan_body))
        .await.unwrap();

    // Delete the recipe
    app.clone()
        .oneshot(delete_req(&format!("/recipes/{}", id), &cookie))
        .await.unwrap();

    // The meal plan entry should be gone (ON DELETE CASCADE)
    let response = app.clone()
        .oneshot(get_req("/calendar/entries?start=2026-06-01&end=2026-06-01", &cookie))
        .await.unwrap();
    let body = response.into_body().collect().await.unwrap().to_bytes();
    let entries: Vec<serde_json::Value> = serde_json::from_slice(&body).unwrap();
    assert!(entries.is_empty(), "Meal plan entries should be cascade-deleted with the recipe");
}

// ---------------------------------------------------------------------------
// Multiple entries per slot — integration tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_multiple_entries_same_slot_api() {
    let (app, _pool) = build_test_app().await;
    let cookie = login(&app).await;

    // Create two recipes
    for name in ["Recipe A", "Recipe B"] {
        app.clone()
            .oneshot(json_req("POST", "/recipes", &cookie, serde_json::json!({
                "name": name, "source_url": null, "ingredients": [], "instructions": []
            })))
            .await.unwrap();
    }
    let body = app.clone().oneshot(get_req("/recipes", &cookie)).await.unwrap()
        .into_body().collect().await.unwrap().to_bytes();
    let recipes: Vec<serde_json::Value> = serde_json::from_slice(&body).unwrap();
    let id_a = recipes[0]["id"].as_i64().unwrap();
    let id_b = recipes[1]["id"].as_i64().unwrap();

    // Plan both to the same slot
    for recipe_id in [id_a, id_b] {
        let res = app.clone()
            .oneshot(json_req("POST", "/calendar/entries", &cookie, serde_json::json!({
                "date": "2026-07-01", "slot": "dinner", "recipe_id": recipe_id
            })))
            .await.unwrap();
        assert_eq!(res.status(), StatusCode::CREATED);
    }

    // Both entries should be returned
    let body = app.clone()
        .oneshot(get_req("/calendar/entries?start=2026-07-01&end=2026-07-01", &cookie))
        .await.unwrap()
        .into_body().collect().await.unwrap().to_bytes();
    let entries: Vec<serde_json::Value> = serde_json::from_slice(&body).unwrap();
    assert_eq!(entries.len(), 2, "Both entries should persist in the same slot");
    assert!(entries.iter().all(|e| e["id"].as_i64().unwrap() > 0), "Each entry must have a non-zero id");
}

#[tokio::test]
async fn test_delete_meal_entry_by_id_api() {
    let (app, _pool) = build_test_app().await;
    let cookie = login(&app).await;

    // Create two recipes and plan both to the same slot
    for name in ["Recipe X", "Recipe Y"] {
        app.clone()
            .oneshot(json_req("POST", "/recipes", &cookie, serde_json::json!({
                "name": name, "source_url": null, "ingredients": [], "instructions": []
            })))
            .await.unwrap();
    }
    let body = app.clone().oneshot(get_req("/recipes", &cookie)).await.unwrap()
        .into_body().collect().await.unwrap().to_bytes();
    let recipes: Vec<serde_json::Value> = serde_json::from_slice(&body).unwrap();

    for r in &recipes {
        app.clone()
            .oneshot(json_req("POST", "/calendar/entries", &cookie, serde_json::json!({
                "date": "2026-08-01", "slot": "lunch", "recipe_id": r["id"]
            })))
            .await.unwrap();
    }

    // Get both entries and capture their ids
    let body = app.clone()
        .oneshot(get_req("/calendar/entries?start=2026-08-01&end=2026-08-01", &cookie))
        .await.unwrap()
        .into_body().collect().await.unwrap().to_bytes();
    let entries: Vec<serde_json::Value> = serde_json::from_slice(&body).unwrap();
    assert_eq!(entries.len(), 2);

    let delete_id = entries[0]["id"].as_i64().unwrap();
    let keep_recipe_id = entries[1]["recipe_id"].as_i64().unwrap();

    // Delete only the first entry by id
    let res = app.clone()
        .oneshot(delete_req(&format!("/calendar/entries?id={}", delete_id), &cookie))
        .await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    // Only the second entry should remain
    let body = app.clone()
        .oneshot(get_req("/calendar/entries?start=2026-08-01&end=2026-08-01", &cookie))
        .await.unwrap()
        .into_body().collect().await.unwrap().to_bytes();
    let remaining: Vec<serde_json::Value> = serde_json::from_slice(&body).unwrap();
    assert_eq!(remaining.len(), 1, "Only the un-deleted entry should remain");
    assert_eq!(remaining[0]["recipe_id"].as_i64().unwrap(), keep_recipe_id);
}

#[tokio::test]
async fn test_slot_quota_rejected_api() {
    let (app, _pool) = build_test_app().await;
    let cookie = login(&app).await;

    app.clone()
        .oneshot(json_req("POST", "/recipes", &cookie, serde_json::json!({
            "name": "Quota Recipe", "source_url": null, "ingredients": [], "instructions": []
        })))
        .await.unwrap();
    let body = app.clone().oneshot(get_req("/recipes", &cookie)).await.unwrap()
        .into_body().collect().await.unwrap().to_bytes();
    let recipes: Vec<serde_json::Value> = serde_json::from_slice(&body).unwrap();
    let recipe_id = recipes[0]["id"].as_i64().unwrap();

    // Fill the slot to the production limit (3)
    for _ in 0..3 {
        let res = app.clone()
            .oneshot(json_req("POST", "/calendar/entries", &cookie, serde_json::json!({
                "date": "2026-09-01", "slot": "breakfast", "recipe_id": recipe_id
            })))
            .await.unwrap();
        assert_eq!(res.status(), StatusCode::CREATED);
    }

    // The 4th should be rejected
    let res = app.clone()
        .oneshot(json_req("POST", "/calendar/entries", &cookie, serde_json::json!({
            "date": "2026-09-01", "slot": "breakfast", "recipe_id": recipe_id
        })))
        .await.unwrap();
    assert_eq!(res.status(), StatusCode::BAD_REQUEST, "4th entry in same slot should be rejected");
}

// ---------------------------------------------------------------------------
// Migration tracking tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_migration_idempotent() {
    // Use a bare pool so we can observe the full migration sequence.
    let pool = SqlitePool::connect(":memory:").await.unwrap();

    // Helper: simulate the startup sequence exactly as main.rs does it.
    let run_sequence = |pool: &SqlitePool| {
        let pool = pool.clone();
        async move {
            storage::ensure_migrations_table(&pool).await.expect("ensure_migrations_table");
            for (version, sql) in [
                ("001", include_str!("../migrations/001_initial.sql")),
                ("002", include_str!("../migrations/002_multiple_entries_per_slot.sql")),
                ("003", include_str!("../migrations/003_add_portions_to_meal_plan.sql")),
                ("004", include_str!("../migrations/004_add_is_admin_to_users.sql")),
            ] {
                if !storage::is_migration_applied(&pool, version).await.expect("is_migration_applied") {
                    sqlx::query(sql).execute(&pool).await.expect("run migration sql");
                    storage::record_migration(&pool, version).await.expect("record_migration");
                }
            }
        }
    };

    // First pass: applies both migrations.
    run_sequence(&pool).await;

    // Seed data so we can verify the table isn't wiped on second pass.
    sqlx::query(
        "INSERT INTO users (id, username, password_hash) VALUES (1, 'u', 'h')"
    ).execute(&pool).await.unwrap();
    sqlx::query(
        "INSERT INTO recipes (id, user_id, name, ingredients, instructions) VALUES (1, 1, 'R', '[]', '[]')"
    ).execute(&pool).await.unwrap();
    sqlx::query(
        "INSERT INTO meal_plan (user_id, date, slot, recipe_id) VALUES (1, '2026-01-01', 'lunch', 1)"
    ).execute(&pool).await.unwrap();

    let count_before: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM meal_plan")
        .fetch_one(&pool).await.unwrap();

    // Second pass: all migrations already applied — must be a complete no-op.
    run_sequence(&pool).await;

    let count_after: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM meal_plan")
        .fetch_one(&pool).await.unwrap();

    assert_eq!(count_before, count_after, "second migration pass must not alter meal_plan data");
    assert_eq!(count_after, 1);
}

// ---------------------------------------------------------------------------
// Multi-user isolation and admin route tests
// ---------------------------------------------------------------------------

/// User B logs in after admin creates a recipe; B's recipe list must be empty.
#[tokio::test]
async fn test_user_a_cannot_see_user_b_recipes() {
    let (app, pool) = build_test_app().await;
    let admin_cookie = login(&app).await;

    // Admin creates a recipe.
    app.clone()
        .oneshot(json_req("POST", "/recipes", &admin_cookie, serde_json::json!({
            "name": "Admin Secret Recipe", "source_url": null,
            "ingredients": [], "instructions": []
        })))
        .await.unwrap();

    // Insert a second non-admin user directly and log in as them.
    let hash = auth::hash_password("password2").unwrap();
    storage::create_user(&pool, "user2", &hash).await.unwrap();

    let login_req = Request::builder()
        .method("POST")
        .uri("/login")
        .header("content-type", "application/x-www-form-urlencoded")
        .body(Body::from("username=user2&password=password2"))
        .unwrap();
    let login_res = app.clone().oneshot(login_req).await.unwrap();
    assert_eq!(login_res.status(), StatusCode::SEE_OTHER);
    let user2_cookie = login_res
        .headers()
        .get("set-cookie")
        .unwrap()
        .to_str().unwrap()
        .split(';').next().unwrap()
        .to_string();

    // User 2's recipe list must be empty.
    let response = app.clone()
        .oneshot(get_req("/recipes", &user2_cookie))
        .await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = response.into_body().collect().await.unwrap().to_bytes();
    let recipes: Vec<serde_json::Value> = serde_json::from_slice(&body).unwrap();
    assert!(recipes.is_empty(), "User 2 must not see User 1's recipes");
}

/// Admin can create a new user via POST /admin/users; the new user can then log in.
#[tokio::test]
async fn test_admin_create_user_api() {
    let (app, _pool) = build_test_app().await;
    let cookie = login(&app).await;

    let response = app.clone()
        .oneshot(json_req("POST", "/admin/users", &cookie, serde_json::json!({
            "username": "newuser",
            "password": "strongpassword"
        })))
        .await.unwrap();
    assert_eq!(response.status(), StatusCode::CREATED, "Admin should be able to create a user");

    // New user can log in.
    let login_req = Request::builder()
        .method("POST")
        .uri("/login")
        .header("content-type", "application/x-www-form-urlencoded")
        .body(Body::from("username=newuser&password=strongpassword"))
        .unwrap();
    let login_res = app.clone().oneshot(login_req).await.unwrap();
    assert_eq!(login_res.status(), StatusCode::SEE_OTHER);
    assert_ne!(
        login_res.headers().get("location").unwrap().to_str().unwrap(),
        "/login?error=1",
        "New user should be able to log in"
    );
}

/// A non-admin user receives 403 when accessing any admin route.
#[tokio::test]
async fn test_non_admin_cannot_access_admin_routes() {
    let (app, pool) = build_test_app().await;

    // Create a non-admin user.
    let hash = auth::hash_password("password3").unwrap();
    storage::create_user(&pool, "regularuser", &hash).await.unwrap();

    let login_req = Request::builder()
        .method("POST")
        .uri("/login")
        .header("content-type", "application/x-www-form-urlencoded")
        .body(Body::from("username=regularuser&password=password3"))
        .unwrap();
    let login_res = app.clone().oneshot(login_req).await.unwrap();
    let user_cookie = login_res
        .headers()
        .get("set-cookie")
        .unwrap()
        .to_str().unwrap()
        .split(';').next().unwrap()
        .to_string();

    // GET /admin should return 403.
    let response = app.clone()
        .oneshot(get_req("/admin", &user_cookie))
        .await.unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN, "Non-admin must be forbidden from /admin");

    // GET /admin/users should also return 403.
    let response = app.clone()
        .oneshot(get_req("/admin/users", &user_cookie))
        .await.unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN, "Non-admin must be forbidden from /admin/users");
}

// ---------------------------------------------------------------------------
// Admin user deletion tests
// ---------------------------------------------------------------------------

/// Admin can delete another user; that user no longer appears in the list.
#[tokio::test]
async fn test_admin_delete_user_api() {
    let (app, pool) = build_test_app().await;
    let admin_cookie = login(&app).await;

    // Create a second user to delete.
    let hash = auth::hash_password("password2").unwrap();
    let victim_id = storage::create_user(&pool, "victim", &hash).await.unwrap();

    let response = app.clone()
        .oneshot(delete_req(&format!("/admin/users/{}", victim_id), &admin_cookie))
        .await.unwrap();
    assert_eq!(response.status(), StatusCode::OK, "Admin should be able to delete another user");

    // Verify the user is gone.
    let list_response = app.clone()
        .oneshot(get_req("/admin/users", &admin_cookie))
        .await.unwrap();
    assert_eq!(list_response.status(), StatusCode::OK);
    let body = list_response.into_body().collect().await.unwrap().to_bytes();
    let users: Vec<serde_json::Value> = serde_json::from_slice(&body).unwrap();
    assert!(
        users.iter().all(|u| u["username"] != "victim"),
        "Deleted user must not appear in the user list"
    );
}

/// Admin receives 400 when attempting to delete their own account.
#[tokio::test]
async fn test_admin_cannot_delete_self_api() {
    let (app, _pool) = build_test_app().await;
    let admin_cookie = login(&app).await;

    // Fetch the admin's own ID via /profile/me.
    let me_response = app.clone()
        .oneshot(get_req("/profile/me", &admin_cookie))
        .await.unwrap();
    assert_eq!(me_response.status(), StatusCode::OK);
    let body = me_response.into_body().collect().await.unwrap().to_bytes();
    let me: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let admin_id = me["id"].as_i64().unwrap();

    let response = app.clone()
        .oneshot(delete_req(&format!("/admin/users/{}", admin_id), &admin_cookie))
        .await.unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST, "Self-deletion must be rejected with 400");
}

/// Deleting a non-existent user is a no-op — returns 200.
#[tokio::test]
async fn test_admin_delete_nonexistent_user_api() {
    let (app, _pool) = build_test_app().await;
    let admin_cookie = login(&app).await;

    let response = app.clone()
        .oneshot(delete_req("/admin/users/999999", &admin_cookie))
        .await.unwrap();
    assert_eq!(response.status(), StatusCode::OK, "Deleting a non-existent user must be a 200 no-op");
}

// ---------------------------------------------------------------------------
// Self-service password change tests
// ---------------------------------------------------------------------------

/// Authenticated user can change their own password successfully.
#[tokio::test]
async fn test_change_own_password_api() {
    let (app, _pool) = build_test_app().await;
    let cookie = login(&app).await;

    let response = app.clone()
        .oneshot(json_req("POST", "/profile/password", &cookie, serde_json::json!({
            "current_password": "password",
            "new_password": "newpassword1"
        })))
        .await.unwrap();
    assert_eq!(response.status(), StatusCode::OK, "Valid password change should return 200");

    // Old password must no longer work.
    let login_req = Request::builder()
        .method("POST")
        .uri("/login")
        .header("content-type", "application/x-www-form-urlencoded")
        .body(Body::from("username=admin&password=password"))
        .unwrap();
    let login_res = app.clone().oneshot(login_req).await.unwrap();
    let location = login_res.headers().get("location").unwrap().to_str().unwrap().to_string();
    assert!(location.contains("error=1"), "Old password must be rejected after change");

    // New password must work.
    let login_req2 = Request::builder()
        .method("POST")
        .uri("/login")
        .header("content-type", "application/x-www-form-urlencoded")
        .body(Body::from("username=admin&password=newpassword1"))
        .unwrap();
    let login_res2 = app.clone().oneshot(login_req2).await.unwrap();
    assert_eq!(login_res2.status(), StatusCode::SEE_OTHER);
    let location2 = login_res2.headers().get("location").unwrap().to_str().unwrap();
    assert!(!location2.contains("error=1"), "New password must be accepted");
}

/// Wrong current password returns 400.
#[tokio::test]
async fn test_change_own_password_wrong_current_api() {
    let (app, _pool) = build_test_app().await;
    let cookie = login(&app).await;

    let response = app.clone()
        .oneshot(json_req("POST", "/profile/password", &cookie, serde_json::json!({
            "current_password": "wrongpassword",
            "new_password": "newpassword1"
        })))
        .await.unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST, "Wrong current password must return 400");
}

/// New password shorter than 8 characters returns 400.
#[tokio::test]
async fn test_change_own_password_too_short_api() {
    let (app, _pool) = build_test_app().await;
    let cookie = login(&app).await;

    let response = app.clone()
        .oneshot(json_req("POST", "/profile/password", &cookie, serde_json::json!({
            "current_password": "password",
            "new_password": "short"
        })))
        .await.unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST, "Too-short new password must return 400");
}

/// Admin changes a user's password; old credentials fail and new ones succeed.
#[tokio::test]
async fn test_admin_change_password_api() {
    let (app, _pool) = build_test_app().await;
    let admin_cookie = login(&app).await;

    // Admin creates a target user.
    app.clone()
        .oneshot(json_req("POST", "/admin/users", &admin_cookie, serde_json::json!({
            "username": "targetuser",
            "password": "originalpassword"
        })))
        .await.unwrap();

    // Fetch user list to get target user's id.
    let res = app.clone()
        .oneshot(get_req("/admin/users", &admin_cookie))
        .await.unwrap();
    let body = res.into_body().collect().await.unwrap().to_bytes();
    let users: Vec<serde_json::Value> = serde_json::from_slice(&body).unwrap();
    let target = users.iter().find(|u| u["username"] == "targetuser").unwrap();
    let target_id = target["id"].as_i64().unwrap();

    // Admin changes the password.
    let response = app.clone()
        .oneshot(json_req("POST", "/admin/users/password", &admin_cookie, serde_json::json!({
            "target_user_id": target_id,
            "new_password": "brandnewpassword"
        })))
        .await.unwrap();
    assert_eq!(response.status(), StatusCode::OK, "Password change should succeed");

    // Old password now fails.
    let bad_login = Request::builder()
        .method("POST")
        .uri("/login")
        .header("content-type", "application/x-www-form-urlencoded")
        .body(Body::from("username=targetuser&password=originalpassword"))
        .unwrap();
    let bad_res = app.clone().oneshot(bad_login).await.unwrap();
    let bad_location = bad_res.headers().get("location").unwrap().to_str().unwrap();
    assert!(bad_location.contains("error=1"), "Old password should no longer work");

    // New password succeeds.
    let good_login = Request::builder()
        .method("POST")
        .uri("/login")
        .header("content-type", "application/x-www-form-urlencoded")
        .body(Body::from("username=targetuser&password=brandnewpassword"))
        .unwrap();
    let good_res = app.clone().oneshot(good_login).await.unwrap();
    assert_eq!(good_res.status(), StatusCode::SEE_OTHER);
    let good_location = good_res.headers().get("location").unwrap().to_str().unwrap();
    assert!(!good_location.contains("error=1"), "New password should allow login");
}

// ---------------------------------------------------------------------------
// Auth / session tests
// ---------------------------------------------------------------------------

/// Logging out invalidates the session: a subsequent authenticated request
/// redirects to `/login` instead of succeeding.
#[tokio::test]
async fn test_logout_invalidates_session() {
    let (app, _pool) = build_test_app().await;
    let cookie = login(&app).await;

    // POST /logout — no Origin header, so CSRF middleware passes through.
    let logout_req = Request::builder()
        .method("POST")
        .uri("/logout")
        .header("cookie", &cookie)
        .body(Body::empty())
        .unwrap();
    let logout_res = app.clone().oneshot(logout_req).await.unwrap();
    assert_eq!(logout_res.status(), StatusCode::SEE_OTHER);
    assert_eq!(logout_res.headers().get("location").unwrap(), "/login");

    // Same cookie must no longer grant access.
    let response = app.clone().oneshot(get_req("/recipes", &cookie)).await.unwrap();
    assert_eq!(
        response.status(),
        StatusCode::SEE_OTHER,
        "Invalidated session must redirect to /login"
    );
    assert_eq!(response.headers().get("location").unwrap(), "/login");
}

// ---------------------------------------------------------------------------
// Missing API integration tests (item 33)
// ---------------------------------------------------------------------------

/// POST a meal entry then DELETE it directly by id; the range query returns empty.
#[tokio::test]
async fn test_delete_meal_entry_direct() {
    let (app, _pool) = build_test_app().await;
    let cookie = login(&app).await;

    // Create a recipe.
    app.clone()
        .oneshot(json_req("POST", "/recipes", &cookie, serde_json::json!({
            "name": "Direct Delete Recipe", "source_url": null,
            "ingredients": [], "instructions": []
        })))
        .await.unwrap();

    let body = app.clone().oneshot(get_req("/recipes", &cookie)).await.unwrap()
        .into_body().collect().await.unwrap().to_bytes();
    let recipes: Vec<serde_json::Value> = serde_json::from_slice(&body).unwrap();
    let recipe_id = recipes[0]["id"].as_i64().unwrap();

    // Plan it.
    let plan_res = app.clone()
        .oneshot(json_req("POST", "/calendar/entries", &cookie, serde_json::json!({
            "date": "2026-10-01", "slot": "breakfast", "recipe_id": recipe_id
        })))
        .await.unwrap();
    assert_eq!(plan_res.status(), StatusCode::CREATED);

    // Retrieve the entry id.
    let body = app.clone()
        .oneshot(get_req("/calendar/entries?start=2026-10-01&end=2026-10-01", &cookie))
        .await.unwrap()
        .into_body().collect().await.unwrap().to_bytes();
    let entries: Vec<serde_json::Value> = serde_json::from_slice(&body).unwrap();
    assert_eq!(entries.len(), 1);
    let entry_id = entries[0]["id"].as_i64().unwrap();

    // Delete by id.
    let res = app.clone()
        .oneshot(delete_req(&format!("/calendar/entries?id={}", entry_id), &cookie))
        .await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    // Range query must now return empty.
    let body = app.clone()
        .oneshot(get_req("/calendar/entries?start=2026-10-01&end=2026-10-01", &cookie))
        .await.unwrap()
        .into_body().collect().await.unwrap().to_bytes();
    let remaining: Vec<serde_json::Value> = serde_json::from_slice(&body).unwrap();
    assert!(remaining.is_empty(), "Entry should be gone after direct DELETE");
}

/// GET /calendar/entries with start after end returns 400.
#[tokio::test]
async fn test_get_calendar_entries_invalid_range() {
    let (app, _pool) = build_test_app().await;
    let cookie = login(&app).await;

    let response = app
        .oneshot(get_req("/calendar/entries?start=2026-05-07&end=2026-05-01", &cookie))
        .await.unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

/// POST /recipes with a body exceeding 64 KB returns 413.
#[tokio::test]
async fn test_body_size_limit() {
    let (app, _pool) = build_test_app().await;
    let cookie = login(&app).await;

    let oversized_body = format!(
        r#"{{"name":"{}","source_url":null,"ingredients":[],"instructions":[]}}"#,
        "a".repeat(70_000)
    );
    let request = Request::builder()
        .method("POST")
        .uri("/recipes")
        .header("cookie", &cookie)
        .header("content-type", "application/json")
        .body(Body::from(oversized_body))
        .unwrap();
    let response = app.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
}

/// Authenticated GET / returns 200.
#[tokio::test]
async fn test_index_route_smoke() {
    let (app, _pool) = build_test_app().await;
    let cookie = login(&app).await;

    let response = app.oneshot(get_req("/", &cookie)).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
}

/// Authenticated GET to an unknown route returns 404.
#[tokio::test]
async fn test_404_fallback() {
    let (app, _pool) = build_test_app().await;
    let cookie = login(&app).await;

    let response = app.oneshot(get_req("/does-not-exist", &cookie)).await.unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

// ---------------------------------------------------------------------------
// /profile/me — nav bar data source tests
// ---------------------------------------------------------------------------

/// GET /profile/me as an admin returns is_admin: true (nav.js shows Admin link).
#[tokio::test]
async fn test_profile_me_returns_is_admin_true() {
    let (app, _pool) = build_test_app().await;
    let cookie = login(&app).await;

    let response = app.clone()
        .oneshot(get_req("/profile/me", &cookie))
        .await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = response.into_body().collect().await.unwrap().to_bytes();
    let me: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(me["is_admin"], true, "Admin user must have is_admin: true");
    assert_eq!(me["username"], "admin");
}

/// GET /profile/me as a non-admin returns is_admin: false (nav.js hides Admin link).
#[tokio::test]
async fn test_profile_me_returns_is_admin_false() {
    let (app, pool) = build_test_app().await;

    let hash = auth::hash_password("password2").unwrap();
    storage::create_user(&pool, "regularuser", &hash).await.unwrap();

    let login_req = Request::builder()
        .method("POST")
        .uri("/login")
        .header("content-type", "application/x-www-form-urlencoded")
        .body(Body::from("username=regularuser&password=password2"))
        .unwrap();
    let login_res = app.clone().oneshot(login_req).await.unwrap();
    let user_cookie = login_res
        .headers()
        .get("set-cookie")
        .unwrap()
        .to_str().unwrap()
        .split(';').next().unwrap()
        .to_string();

    let response = app.clone()
        .oneshot(get_req("/profile/me", &user_cookie))
        .await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = response.into_body().collect().await.unwrap().to_bytes();
    let me: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(me["is_admin"], false, "Non-admin user must have is_admin: false");
    assert_eq!(me["username"], "regularuser");
}

// ---------------------------------------------------------------------------
// Cross-user isolation tests — meal plan and cooked log (TEST-3)
// ---------------------------------------------------------------------------

/// User A plans a meal; User B's calendar must be empty.
#[tokio::test]
async fn test_user_isolation_meal_plan() {
    let (app, pool) = build_test_app().await;
    let admin_cookie = login(&app).await;

    // Admin creates a recipe and plans it.
    app.clone()
        .oneshot(json_req("POST", "/recipes", &admin_cookie, serde_json::json!({
            "name": "Admin Meal", "source_url": null, "ingredients": [], "instructions": []
        })))
        .await.unwrap();
    let body = app.clone().oneshot(get_req("/recipes", &admin_cookie)).await.unwrap()
        .into_body().collect().await.unwrap().to_bytes();
    let recipes: Vec<serde_json::Value> = serde_json::from_slice(&body).unwrap();
    let recipe_id = recipes[0]["id"].as_i64().unwrap();

    app.clone()
        .oneshot(json_req("POST", "/calendar/entries", &admin_cookie, serde_json::json!({
            "date": "2026-11-01", "slot": "lunch", "recipe_id": recipe_id
        })))
        .await.unwrap();

    // Create user2 and log in.
    let hash = auth::hash_password("password2").unwrap();
    storage::create_user(&pool, "user2", &hash).await.unwrap();
    let login_req = Request::builder()
        .method("POST").uri("/login")
        .header("content-type", "application/x-www-form-urlencoded")
        .body(Body::from("username=user2&password=password2")).unwrap();
    let user2_cookie = app.clone().oneshot(login_req).await.unwrap()
        .headers().get("set-cookie").unwrap()
        .to_str().unwrap().split(';').next().unwrap().to_string();

    // User 2 must see no meal plan entries.
    let response = app.clone()
        .oneshot(get_req("/calendar/entries?start=2026-11-01&end=2026-11-01", &user2_cookie))
        .await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = response.into_body().collect().await.unwrap().to_bytes();
    let entries: Vec<serde_json::Value> = serde_json::from_slice(&body).unwrap();
    assert!(entries.is_empty(), "User 2 must not see User 1's meal plan entries");
}

/// User A marks a recipe cooked; User B's cooked log must be empty.
#[tokio::test]
async fn test_user_isolation_cooked_log() {
    let (app, pool) = build_test_app().await;
    let admin_cookie = login(&app).await;

    // Admin creates a recipe and marks it cooked.
    app.clone()
        .oneshot(json_req("POST", "/recipes", &admin_cookie, serde_json::json!({
            "name": "Admin Cooked", "source_url": null, "ingredients": [], "instructions": []
        })))
        .await.unwrap();
    let body = app.clone().oneshot(get_req("/recipes", &admin_cookie)).await.unwrap()
        .into_body().collect().await.unwrap().to_bytes();
    let recipes: Vec<serde_json::Value> = serde_json::from_slice(&body).unwrap();
    let recipe_id = recipes[0]["id"].as_i64().unwrap();

    app.clone()
        .oneshot(json_req("POST", "/calendar/cooked", &admin_cookie, serde_json::json!({
            "date": "2026-11-02", "recipe_id": recipe_id
        })))
        .await.unwrap();

    // Create user2 and log in.
    let hash = auth::hash_password("password2").unwrap();
    storage::create_user(&pool, "user2", &hash).await.unwrap();
    let login_req = Request::builder()
        .method("POST").uri("/login")
        .header("content-type", "application/x-www-form-urlencoded")
        .body(Body::from("username=user2&password=password2")).unwrap();
    let user2_cookie = app.clone().oneshot(login_req).await.unwrap()
        .headers().get("set-cookie").unwrap()
        .to_str().unwrap().split(';').next().unwrap().to_string();

    // User 2 must see no cooked log entries.
    let response = app.clone()
        .oneshot(get_req("/calendar/cooked?start=2026-11-02&end=2026-11-02", &user2_cookie))
        .await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = response.into_body().collect().await.unwrap().to_bytes();
    let entries: Vec<serde_json::Value> = serde_json::from_slice(&body).unwrap();
    assert!(entries.is_empty(), "User 2 must not see User 1's cooked log entries");
}

// ---------------------------------------------------------------------------
// Admin cascade delete at API level (TEST-4)
// ---------------------------------------------------------------------------

/// Deleting a user via the admin API removes their recipes and meal plan
/// entries via ON DELETE CASCADE.
#[tokio::test]
async fn test_admin_delete_user_cascades_data_api() {
    let (app, pool) = build_test_app().await;
    let admin_cookie = login(&app).await;

    // Create user2 and note their ID.
    let hash = auth::hash_password("password2").unwrap();
    let user2_id = storage::create_user(&pool, "user2", &hash).await.unwrap();

    // Log in as user2.
    let login_req = Request::builder()
        .method("POST").uri("/login")
        .header("content-type", "application/x-www-form-urlencoded")
        .body(Body::from("username=user2&password=password2")).unwrap();
    let user2_cookie = app.clone().oneshot(login_req).await.unwrap()
        .headers().get("set-cookie").unwrap()
        .to_str().unwrap().split(';').next().unwrap().to_string();

    // User2 creates a recipe and plans a meal.
    app.clone()
        .oneshot(json_req("POST", "/recipes", &user2_cookie, serde_json::json!({
            "name": "User2 Recipe", "source_url": null, "ingredients": [], "instructions": []
        })))
        .await.unwrap();
    let body = app.clone().oneshot(get_req("/recipes", &user2_cookie)).await.unwrap()
        .into_body().collect().await.unwrap().to_bytes();
    let recipes: Vec<serde_json::Value> = serde_json::from_slice(&body).unwrap();
    let recipe_id = recipes[0]["id"].as_i64().unwrap();

    app.clone()
        .oneshot(json_req("POST", "/calendar/entries", &user2_cookie, serde_json::json!({
            "date": "2026-12-01", "slot": "dinner", "recipe_id": recipe_id
        })))
        .await.unwrap();

    // Admin deletes user2.
    let res = app.clone()
        .oneshot(delete_req(&format!("/admin/users/{}", user2_id), &admin_cookie))
        .await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    // Verify at DB level: recipes and meal_plan entries for user2 are gone.
    let recipe_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM recipes WHERE user_id = ?")
        .bind(user2_id)
        .fetch_one(&pool).await.unwrap();
    assert_eq!(recipe_count, 0, "User2's recipes must be cascade-deleted");

    let meal_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM meal_plan WHERE user_id = ?")
        .bind(user2_id)
        .fetch_one(&pool).await.unwrap();
    assert_eq!(meal_count, 0, "User2's meal plan entries must be cascade-deleted");
}

// ---------------------------------------------------------------------------
// Meal plan portions API-level tests (TEST-7)
// ---------------------------------------------------------------------------

/// POST /calendar/entries with portions=0 returns 400.
#[tokio::test]
async fn test_plan_meal_portions_zero_rejected_api() {
    let (app, _pool) = build_test_app().await;
    let cookie = login(&app).await;

    app.clone()
        .oneshot(json_req("POST", "/recipes", &cookie, serde_json::json!({
            "name": "Portions Zero", "source_url": null, "ingredients": [], "instructions": []
        })))
        .await.unwrap();
    let body = app.clone().oneshot(get_req("/recipes", &cookie)).await.unwrap()
        .into_body().collect().await.unwrap().to_bytes();
    let recipes: Vec<serde_json::Value> = serde_json::from_slice(&body).unwrap();
    let recipe_id = recipes[0]["id"].as_i64().unwrap();

    let res = app.clone()
        .oneshot(json_req("POST", "/calendar/entries", &cookie, serde_json::json!({
            "date": "2026-12-10", "slot": "lunch", "recipe_id": recipe_id, "portions": 0
        })))
        .await.unwrap();
    assert_eq!(res.status(), StatusCode::BAD_REQUEST, "portions=0 must be rejected with 400");
}

/// POST /calendar/entries with portions=-1 returns 400.
#[tokio::test]
async fn test_plan_meal_portions_negative_rejected_api() {
    let (app, _pool) = build_test_app().await;
    let cookie = login(&app).await;

    app.clone()
        .oneshot(json_req("POST", "/recipes", &cookie, serde_json::json!({
            "name": "Portions Negative", "source_url": null, "ingredients": [], "instructions": []
        })))
        .await.unwrap();
    let body = app.clone().oneshot(get_req("/recipes", &cookie)).await.unwrap()
        .into_body().collect().await.unwrap().to_bytes();
    let recipes: Vec<serde_json::Value> = serde_json::from_slice(&body).unwrap();
    let recipe_id = recipes[0]["id"].as_i64().unwrap();

    let res = app.clone()
        .oneshot(json_req("POST", "/calendar/entries", &cookie, serde_json::json!({
            "date": "2026-12-11", "slot": "dinner", "recipe_id": recipe_id, "portions": -1
        })))
        .await.unwrap();
    assert_eq!(res.status(), StatusCode::BAD_REQUEST, "portions=-1 must be rejected with 400");
}

/// POST /calendar/entries without a portions field defaults to 1.
#[tokio::test]
async fn test_plan_meal_portions_default_api() {
    let (app, _pool) = build_test_app().await;
    let cookie = login(&app).await;

    app.clone()
        .oneshot(json_req("POST", "/recipes", &cookie, serde_json::json!({
            "name": "Default Portions", "source_url": null, "ingredients": [], "instructions": []
        })))
        .await.unwrap();
    let body = app.clone().oneshot(get_req("/recipes", &cookie)).await.unwrap()
        .into_body().collect().await.unwrap().to_bytes();
    let recipes: Vec<serde_json::Value> = serde_json::from_slice(&body).unwrap();
    let recipe_id = recipes[0]["id"].as_i64().unwrap();

    // POST without a portions field.
    let res = app.clone()
        .oneshot(json_req("POST", "/calendar/entries", &cookie, serde_json::json!({
            "date": "2026-12-12", "slot": "breakfast", "recipe_id": recipe_id
        })))
        .await.unwrap();
    assert_eq!(res.status(), StatusCode::CREATED);

    // Retrieve the entry and verify portions == 1.
    let body = app.clone()
        .oneshot(get_req("/calendar/entries?start=2026-12-12&end=2026-12-12", &cookie))
        .await.unwrap()
        .into_body().collect().await.unwrap().to_bytes();
    let entries: Vec<serde_json::Value> = serde_json::from_slice(&body).unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0]["portions"], 1, "Omitted portions must default to 1");
}
