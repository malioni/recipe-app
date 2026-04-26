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
    routing::{get, post},
    Router,
};
use http_body_util::BodyExt;
use recipe_app::{auth, network, storage};
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

    let hash = auth::hash_password("password").unwrap();
    storage::create_user(&pool, "admin", &hash).await.unwrap();

    let session_store = SqliteStore::new(pool.clone());
    session_store.migrate().await.unwrap();

    let session_layer = SessionManagerLayer::new(session_store)
        .with_secure(false)
        .with_expiry(Expiry::OnInactivity(TimeDuration::days(1)));

    let app = Router::new()
        .route("/login",  get(network::handle_login_page).post(network::handle_login))
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
