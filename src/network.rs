use axum::{
    response::{IntoResponse, Html, Redirect},
    response::Response,
    extract::{Path, Json, Query, State},
    http::StatusCode,
    Form,
};
use chrono::NaiveDate;
use serde::Deserialize;
use sqlx::SqlitePool;
use tower_sessions::Session;
use crate::auth::{self, AuthAdmin, AuthUser, SESSION_IS_ADMIN_KEY, SESSION_USER_ID_KEY};
use crate::manager;
use crate::calendar_manager;
use crate::model::{ChangePasswordForm, CreateUserForm, LoginForm, MealEntry, CookedEntry, Recipe, SelfChangePasswordForm};

// HTML pages embedded at compile time — no runtime file I/O on each request.
const HTML_LOGIN: &str = include_str!("../html/login.html");
const HTML_INDEX: &str = include_str!("../html/index.html");
const HTML_ADD_RECIPE: &str = include_str!("../html/add-recipe.html");
const HTML_CALENDAR: &str = include_str!("../html/calendar.html");
const HTML_ADMIN: &str = include_str!("../html/admin.html");
const HTML_PROFILE: &str = include_str!("../html/profile.html");
const HTML_404: &str = include_str!("../html/404.html");

// ---------------------------------------------------------------------------
// Auth handlers
// ---------------------------------------------------------------------------

/// Serves the login page, or redirects to `/` if the user is already logged in.
///
/// Reads the session to determine whether the user is already authenticated.
/// If `user_id` is present in the session the client is redirected immediately,
/// avoiding a redundant login screen.
///
/// # Parameters
///
/// - `session` — the current session provided by `tower-sessions` middleware.
///
/// # Returns
///
/// A `302` redirect to `/` if already logged in, or `200 OK` with the login
/// HTML page if not.
///
/// # Errors
///
/// Session read errors are treated as "not logged in" (no redirect).
pub async fn handle_login_page(session: Session) -> impl IntoResponse {
    let already_logged_in: Option<i64> = session
        .get(SESSION_USER_ID_KEY)
        .await
        .unwrap_or(None);

    if already_logged_in.is_some() {
        return Redirect::to("/").into_response();
    }

    Html(HTML_LOGIN).into_response()
}

/// Validates credentials submitted via the login form and creates a session.
///
/// Looks up the user by username, verifies the password with argon2id, and
/// stores `user_id` and `is_admin` in the session on success. Unknown usernames
/// and wrong passwords produce the same redirect to avoid username enumeration.
///
/// # Parameters
///
/// - `pool` — the SQLite connection pool, used to look up the user record.
/// - `session` — the current session; `user_id` and `is_admin` are inserted on success.
/// - `form` — the parsed login form containing `username` and `password` fields.
///
/// # Returns
///
/// A `302` redirect to `/` on success, or a `302` redirect to `/login?error=1`
/// on any failure (unknown user, wrong password, or session error).
///
/// # Errors
///
/// All failures produce a redirect rather than an error status code, so as not
/// to leak whether the failure was at the lookup or password-verification step.
/// Session insertion failures produce `500 Internal Server Error`.
pub async fn handle_login(
    State(pool): State<SqlitePool>,
    session: Session,
    Form(form): Form<LoginForm>,
) -> impl IntoResponse {
    // Look up the user. Treat "user not found" and "wrong password" identically
    // to avoid leaking which usernames exist.
    let user = match manager::get_user_by_username(&pool, &form.username).await {
        Ok(Some(u)) => u,
        Ok(None) => {
            tracing::warn!("Login attempt for unknown username: {}", form.username);
            return Redirect::to("/login?error=1").into_response();
        }
        Err(e) => {
            tracing::error!("Database error during login: {e}");
            return Redirect::to("/login?error=1").into_response();
        }
    };

    match auth::verify_password(&form.password, &user.password_hash) {
        Ok(true) => {}
        Ok(false) => {
            tracing::warn!("Failed login attempt for username: {}", form.username);
            return Redirect::to("/login?error=1").into_response();
        }
        Err(e) => {
            tracing::error!("Password verification error: {e}");
            return Redirect::to("/login?error=1").into_response();
        }
    }

    if session.insert(SESSION_USER_ID_KEY, user.id).await.is_err()
        || session.insert(SESSION_IS_ADMIN_KEY, user.is_admin).await.is_err()
    {
        tracing::error!("Failed to create session for user: {}", user.username);
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Html("<h1>Session error. Please try again.</h1>".to_string()),
        )
            .into_response();
    }

    tracing::info!("User logged in: {}", user.username);
    Redirect::to("/").into_response()
}

/// Destroys the current session and redirects the user to the login page.
///
/// Flushes all session data regardless of whether a session was active.
/// Any error from `flush` is silently ignored — the redirect always happens.
///
/// # Parameters
///
/// - `session` — the current session provided by `tower-sessions` middleware.
///
/// # Returns
///
/// A `302` redirect to `/login`.
///
/// # Errors
///
/// Session flush errors are silently ignored; the handler always redirects.
pub async fn handle_logout(session: Session) -> impl IntoResponse {
    let _ = session.flush().await;
    tracing::info!("User logged out");
    Redirect::to("/login").into_response()
}

// ---------------------------------------------------------------------------
// Recipes
// ---------------------------------------------------------------------------

/// Serves the main recipe list page.
///
/// The `AuthUser` extractor automatically redirects unauthenticated requests
/// to `/login` before this handler runs.
///
/// # Parameters
///
/// - `_auth` — the authenticated user extracted from the session (used only
///   to enforce authentication; the user ID is not needed to serve a static page).
///
/// # Returns
///
/// `200 OK` with the index HTML page.
///
/// # Errors
///
/// Returns a `302` redirect to `/login` (via the `AuthUser` extractor) if the
/// session is missing or expired — the handler body never executes in that case.
pub async fn handle_index(_auth: AuthUser) -> impl IntoResponse {
    Html(HTML_INDEX)
}

/// Returns all recipes for the authenticated user as a JSON array.
///
/// Delegates to [`manager::get_all_recipes`]. On a storage error the handler
/// returns `500 Internal Server Error` rather than an empty list.
///
/// # Parameters
///
/// - `auth` — the authenticated user; `auth.user_id` scopes the query.
/// - `pool` — the SQLite connection pool.
///
/// # Returns
///
/// `200 OK` with a JSON array of [`Recipe`] objects (may be empty).
///
/// # Errors
///
/// Returns `500 Internal Server Error` with a JSON `{ "error": "..." }` body
/// if the database query fails.
pub async fn handle_all_recipes(
    auth: AuthUser,
    State(pool): State<SqlitePool>,
) -> impl IntoResponse {
    match manager::get_all_recipes(&pool, auth.user_id).await {
        Ok(recipes) => (StatusCode::OK, Json(recipes)).into_response(),
        Err(err_msg) => {
            tracing::error!("Error fetching recipes: {err_msg}");
            (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({ "error": err_msg }))).into_response()
        }
    }
}

/// Returns a single recipe as JSON, scoped to the authenticated user.
///
/// A recipe owned by a different user is treated as not found. The lookup
/// delegates to [`manager::get_recipe_by_id`] which distinguishes "not found"
/// from genuine database errors.
///
/// # Parameters
///
/// - `auth` — the authenticated user; `auth.user_id` scopes the lookup.
/// - `pool` — the SQLite connection pool.
/// - `id` — the primary key of the recipe to retrieve, extracted from the URL path.
///
/// # Returns
///
/// `200 OK` with the [`Recipe`] JSON if found; `404 Not Found` if the ID does
/// not exist or belongs to another user.
///
/// # Errors
///
/// Returns `500 Internal Server Error` with a JSON `{ "error": "..." }` body
/// if the database query itself fails (as opposed to the recipe simply not existing).
pub async fn handle_recipe(
    auth: AuthUser,
    State(pool): State<SqlitePool>,
    Path(id): Path<i64>,
) -> impl IntoResponse {
    match manager::get_recipe_by_id(&pool, auth.user_id, id).await {
        Ok(Some(recipe)) => (StatusCode::OK, Json(recipe)).into_response(),
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": format!("Recipe with ID {} not found", id) })),
        )
            .into_response(),
        Err(err_msg) => {
            tracing::error!("Error fetching recipe {id}: {err_msg}");
            (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({ "error": err_msg }))).into_response()
        }
    }
}

/// Serves the recipe creation and editing form page.
///
/// This single page handles both new-recipe creation and editing an existing
/// recipe. When editing, client-side JavaScript reads `?id=<recipe_id>` from
/// the URL and pre-populates the form via a `GET /recipes/:id` request.
/// No query parameter processing is required server-side.
///
/// # Parameters
///
/// - `_auth` — the authenticated user extracted from the session (used only to
///   enforce authentication).
///
/// # Returns
///
/// `200 OK` with the add-recipe HTML page.
///
/// # Errors
///
/// Returns a `302` redirect to `/login` (via the `AuthUser` extractor) if the
/// session is missing or expired — the handler body never executes in that case.
pub async fn handle_new_recipe_page(_auth: AuthUser) -> impl IntoResponse {
    Html(HTML_ADD_RECIPE)
}

/// Validates and inserts a new recipe for the authenticated user.
///
/// Delegates validation and quota enforcement to [`manager::add_recipe`].
/// On success the new recipe's storage ID is not returned — clients should
/// refresh the recipe list if they need the assigned ID.
///
/// # Parameters
///
/// - `auth` — the authenticated user; `auth.user_id` is set as the recipe owner.
/// - `pool` — the SQLite connection pool.
/// - `new_recipe` — the recipe to create, deserialized from the JSON request body.
///
/// # Returns
///
/// `201 Created` with `{ "status": "created" }` on success.
///
/// # Errors
///
/// Returns `500 Internal Server Error` with a JSON `{ "error": "..." }` body if
/// validation fails (e.g. name too long, too many ingredients) or the insert fails.
pub async fn handle_add_recipe(
    auth: AuthUser,
    State(pool): State<SqlitePool>,
    Json(new_recipe): Json<Recipe>,
) -> impl IntoResponse {
    match manager::add_recipe(&pool, auth.user_id, new_recipe).await {
        Ok(_) => {
            tracing::info!("Recipe added");
            (StatusCode::CREATED, Json(serde_json::json!({ "status": "created" })))
        }
        Err(err_msg) => {
            tracing::error!("Error saving recipe: {err_msg}");
            (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({ "error": err_msg })))
        }
    }
}

/// Deletes a recipe owned by the authenticated user.
///
/// Meal plan and cooked log entries that reference the recipe are removed
/// automatically via `ON DELETE CASCADE`. Deleting a non-existent or
/// already-deleted recipe is a no-op (returns `200 OK`).
///
/// # Parameters
///
/// - `auth` — the authenticated user; only recipes owned by this user can be deleted.
/// - `pool` — the SQLite connection pool.
/// - `id` — the primary key of the recipe to delete, extracted from the URL path.
///
/// # Returns
///
/// `200 OK` with `{ "status": "deleted" }` on success.
///
/// # Errors
///
/// Returns `500 Internal Server Error` with a JSON `{ "error": "..." }` body if
/// the delete query itself fails.
pub async fn handle_delete_recipe(
    auth: AuthUser,
    State(pool): State<SqlitePool>,
    Path(id): Path<i64>,
) -> impl IntoResponse {
    match manager::delete_recipe(&pool, auth.user_id, id).await {
        Ok(_) => {
            tracing::info!("Recipe {} deleted", id);
            (StatusCode::OK, Json(serde_json::json!({ "status": "deleted" })))
        }
        Err(err_msg) => {
            tracing::error!("Error deleting recipe {id}: {err_msg}");
            (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({ "error": err_msg })))
        }
    }
}

/// Replaces all fields of an existing recipe owned by the authenticated user.
///
/// The update is user-scoped: a recipe ID that belongs to another user behaves
/// as if the recipe does not exist. Delegates to [`manager::update_recipe`].
///
/// # Parameters
///
/// - `auth` — the authenticated user; only recipes owned by this user can be updated.
/// - `pool` — the SQLite connection pool.
/// - `id` — the primary key of the recipe to update, extracted from the URL path.
/// - `updated_recipe` — the replacement recipe data, deserialized from the JSON request body.
///
/// # Returns
///
/// `200 OK` with `{ "status": "updated" }` on success.
///
/// # Errors
///
/// Returns `500 Internal Server Error` with a JSON `{ "error": "..." }` body if
/// validation fails, the recipe does not exist for this user, or the query fails.
pub async fn handle_update_recipe(
    auth: AuthUser,
    State(pool): State<SqlitePool>,
    Path(id): Path<i64>,
    Json(updated_recipe): Json<Recipe>,
) -> impl IntoResponse {
    match manager::update_recipe(&pool, auth.user_id, id, updated_recipe).await {
        Ok(_) => {
            tracing::info!("Recipe {} updated", id);
            (StatusCode::OK, Json(serde_json::json!({ "status": "updated" })))
        }
        Err(err_msg) => {
            tracing::error!("Error updating recipe {id}: {err_msg}");
            (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({ "error": err_msg })))
        }
    }
}

// ---------------------------------------------------------------------------
// Calendar — shared query parameter structs
// ---------------------------------------------------------------------------

/// Query parameters for endpoints that accept a date range.
///
/// Both fields are required. Dates must be in `YYYY-MM-DD` format. It is the
/// handler's responsibility to validate that `start <= end`; the struct itself
/// does not enforce ordering.
#[derive(Deserialize)]
pub struct DateRangeParams {
    /// The first day of the range (inclusive).
    pub start: NaiveDate,
    /// The last day of the range (inclusive).
    pub end: NaiveDate,
}

/// Query parameters for deleting a planned meal entry by its primary key.
#[derive(Deserialize)]
pub struct DeleteMealParams {
    /// The primary key of the `meal_plan` row to delete.
    pub id: i64,
}

// ---------------------------------------------------------------------------
// Calendar — page
// ---------------------------------------------------------------------------

/// Serves the calendar and meal planning HTML page.
///
/// The `AuthUser` extractor automatically redirects unauthenticated requests
/// to `/login` before this handler runs.
///
/// # Parameters
///
/// - `_auth` — the authenticated user extracted from the session (used only to
///   enforce authentication).
///
/// # Returns
///
/// `200 OK` with the calendar HTML page.
///
/// # Errors
///
/// Returns a `302` redirect to `/login` (via the `AuthUser` extractor) if the
/// session is missing or expired — the handler body never executes in that case.
pub async fn handle_calendar_page(_auth: AuthUser) -> impl IntoResponse {
    Html(HTML_CALENDAR)
}

// ---------------------------------------------------------------------------
// Calendar — meal plan
// ---------------------------------------------------------------------------

/// Returns meal plan entries for the authenticated user within a date range.
///
/// Delegates range validation to [`calendar_manager::get_meals_in_range`], which
/// returns an error if `start` is after `end`.
///
/// # Parameters
///
/// - `auth` — the authenticated user; `auth.user_id` scopes the query.
/// - `pool` — the SQLite connection pool.
/// - `params` — the `?start=YYYY-MM-DD&end=YYYY-MM-DD` query parameters.
///
/// # Returns
///
/// `200 OK` with a JSON array of [`MealEntry`] objects (may be empty).
///
/// # Errors
///
/// Returns `400 Bad Request` with a JSON `{ "error": "..." }` body if the
/// date range is invalid (e.g. start after end) or the storage query fails.
pub async fn handle_get_meal_entries(
    auth: AuthUser,
    State(pool): State<SqlitePool>,
    Query(params): Query<DateRangeParams>,
) -> impl IntoResponse {
    match calendar_manager::get_meals_in_range(&pool, auth.user_id, params.start, params.end).await {
        Ok(entries) => (StatusCode::OK, Json(entries)).into_response(),
        Err(err_msg) => {
            tracing::error!("Error fetching meal entries: {err_msg}");
            (StatusCode::BAD_REQUEST, Json(serde_json::json!({ "error": err_msg }))).into_response()
        }
    }
}

/// Plans a meal by inserting a meal plan entry for the authenticated user.
///
/// Delegates quota and validation checks to [`calendar_manager::plan_meal`].
/// Multiple entries per date-slot combination are allowed.
///
/// # Parameters
///
/// - `auth` — the authenticated user; `auth.user_id` is set as the entry owner.
/// - `pool` — the SQLite connection pool.
/// - `entry` — the meal entry to create, deserialized from the JSON request body.
///
/// # Returns
///
/// `201 Created` with `{ "status": "planned" }` on success.
///
/// # Errors
///
/// Returns `400 Bad Request` with a JSON `{ "error": "..." }` body if the
/// per-user meal plan quota is exceeded, the recipe does not belong to this user,
/// or the insert fails.
pub async fn handle_plan_meal(
    auth: AuthUser,
    State(pool): State<SqlitePool>,
    Json(entry): Json<MealEntry>,
) -> impl IntoResponse {
    match calendar_manager::plan_meal(&pool, auth.user_id, entry.date, entry.slot, entry.recipe_id, entry.portions).await {
        Ok(_) => {
            tracing::info!("Meal planned: recipe {} on {}", entry.recipe_id, entry.date);
            (StatusCode::CREATED, Json(serde_json::json!({ "status": "planned" })))
        }
        Err(err_msg) => {
            tracing::error!("Error planning meal: {err_msg}");
            (StatusCode::BAD_REQUEST, Json(serde_json::json!({ "error": err_msg })))
        }
    }
}

/// Removes a planned meal entry owned by the authenticated user.
///
/// Only entries owned by the authenticated user can be deleted. Attempting to
/// delete an entry that does not exist or belongs to another user returns
/// `404 Not Found`.
///
/// # Parameters
///
/// - `auth` — the authenticated user; only entries owned by this user may be deleted.
/// - `pool` — the SQLite connection pool.
/// - `params` — the `?id=<entry_id>` query parameter identifying the entry.
///
/// # Returns
///
/// `200 OK` with `{ "status": "deleted" }` on success.
///
/// # Errors
///
/// Returns `404 Not Found` with a JSON `{ "error": "..." }` body if the entry
/// does not exist for this user or the delete fails.
pub async fn handle_delete_meal_entry(
    auth: AuthUser,
    State(pool): State<SqlitePool>,
    Query(params): Query<DeleteMealParams>,
) -> impl IntoResponse {
    match calendar_manager::remove_planned_meal(&pool, auth.user_id, params.id).await {
        Ok(_) => (StatusCode::OK, Json(serde_json::json!({ "status": "deleted" }))),
        Err(err_msg) => {
            tracing::error!("Error deleting meal entry: {err_msg}");
            (StatusCode::NOT_FOUND, Json(serde_json::json!({ "error": err_msg })))
        }
    }
}

// ---------------------------------------------------------------------------
// Calendar — cooked log
// ---------------------------------------------------------------------------

/// Records that a recipe was cooked on a given date.
///
/// The cooked log is append-only; the same recipe can be marked cooked
/// multiple times on the same day. Delegates to
/// [`calendar_manager::mark_as_cooked`].
///
/// # Parameters
///
/// - `auth` — the authenticated user; `auth.user_id` is set as the log entry owner.
/// - `pool` — the SQLite connection pool.
/// - `entry` — the cooked entry (date + recipe ID), deserialized from the JSON request body.
///
/// # Returns
///
/// `201 Created` with `{ "status": "logged" }` on success.
///
/// # Errors
///
/// Returns `400 Bad Request` with a JSON `{ "error": "..." }` body if the
/// recipe does not exist for this user or the insert fails.
pub async fn handle_mark_cooked(
    auth: AuthUser,
    State(pool): State<SqlitePool>,
    Json(entry): Json<CookedEntry>,
) -> impl IntoResponse {
    match calendar_manager::mark_as_cooked(&pool, auth.user_id, entry.date, entry.recipe_id).await {
        Ok(_) => {
            tracing::info!("Recipe {} marked cooked on {}", entry.recipe_id, entry.date);
            (StatusCode::CREATED, Json(serde_json::json!({ "status": "logged" })))
        }
        Err(err_msg) => {
            tracing::error!("Error logging cooked entry: {err_msg}");
            (StatusCode::BAD_REQUEST, Json(serde_json::json!({ "error": err_msg })))
        }
    }
}

/// Returns cooked log entries for the authenticated user within a date range.
///
/// Delegates range validation to [`calendar_manager::get_cooked_in_range`],
/// which returns an error if `start` is after `end`.
///
/// # Parameters
///
/// - `auth` — the authenticated user; `auth.user_id` scopes the query.
/// - `pool` — the SQLite connection pool.
/// - `params` — the `?start=YYYY-MM-DD&end=YYYY-MM-DD` query parameters.
///
/// # Returns
///
/// `200 OK` with a JSON array of [`CookedEntry`] objects (may be empty).
///
/// # Errors
///
/// Returns `400 Bad Request` with a JSON `{ "error": "..." }` body if the
/// date range is invalid or the storage query fails.
pub async fn handle_get_cooked_entries(
    auth: AuthUser,
    State(pool): State<SqlitePool>,
    Query(params): Query<DateRangeParams>,
) -> impl IntoResponse {
    match calendar_manager::get_cooked_in_range(&pool, auth.user_id, params.start, params.end).await {
        Ok(entries) => (StatusCode::OK, Json(entries)).into_response(),
        Err(err_msg) => {
            tracing::error!("Error fetching cooked entries: {err_msg}");
            (StatusCode::BAD_REQUEST, Json(serde_json::json!({ "error": err_msg }))).into_response()
        }
    }
}

// ---------------------------------------------------------------------------
// Calendar — shopping list
// ---------------------------------------------------------------------------

/// Returns an aggregated shopping list for the authenticated user's meal plan.
///
/// Combines all ingredient quantities across all planned meals in the date range,
/// normalising units (e.g. `"g"` and `"grams"` are merged). Delegates to
/// [`calendar_manager::get_shopping_list`].
///
/// # Parameters
///
/// - `auth` — the authenticated user; `auth.user_id` scopes the query.
/// - `pool` — the SQLite connection pool.
/// - `params` — the `?start=YYYY-MM-DD&end=YYYY-MM-DD` query parameters.
///
/// # Returns
///
/// `200 OK` with a JSON array of aggregated [`crate::model::Ingredient`] objects
/// (may be empty if no meals are planned in the range).
///
/// # Errors
///
/// Returns `400 Bad Request` with a JSON `{ "error": "..." }` body if the
/// date range is invalid or the storage query fails.
pub async fn handle_shopping_list(
    auth: AuthUser,
    State(pool): State<SqlitePool>,
    Query(params): Query<DateRangeParams>,
) -> impl IntoResponse {
    match calendar_manager::get_shopping_list(&pool, auth.user_id, params.start, params.end).await {
        Ok(items) => (StatusCode::OK, Json(items)).into_response(),
        Err(err_msg) => {
            tracing::error!("Error generating shopping list: {err_msg}");
            (StatusCode::BAD_REQUEST, Json(serde_json::json!({ "error": err_msg }))).into_response()
        }
    }
}

// ---------------------------------------------------------------------------
// Admin — user management
// ---------------------------------------------------------------------------

/// Serves the admin user management page.
///
/// The `AuthAdmin` extractor returns `403 Forbidden` for non-admin users and
/// for unauthenticated requests — the handler body never executes in those cases.
///
/// # Parameters
///
/// - `_auth` — the authenticated admin user (used only to enforce admin access).
///
/// # Returns
///
/// `200 OK` with the admin HTML page.
///
/// # Errors
///
/// Returns `403 Forbidden` (via the `AuthAdmin` extractor) if the session is
/// missing, expired, or the user does not have `is_admin = true`.
pub async fn handle_admin_page(_auth: AuthAdmin) -> impl IntoResponse {
    Html(HTML_ADMIN)
}

/// Returns all registered users as a JSON array (no password hashes).
///
/// Only admin users may call this endpoint. Delegates to
/// [`manager::admin_list_users`].
///
/// # Parameters
///
/// - `_auth` — the authenticated admin user (used only to enforce admin access).
/// - `pool` — the SQLite connection pool.
///
/// # Returns
///
/// `200 OK` with a JSON array of [`crate::model::UserInfo`] objects.
///
/// # Errors
///
/// Returns `403 Forbidden` (via `AuthAdmin`) if the caller is not an admin.
/// Returns `500 Internal Server Error` with a JSON `{ "error": "..." }` body
/// if the database query fails.
pub async fn handle_admin_list_users(
    _auth: AuthAdmin,
    State(pool): State<SqlitePool>,
) -> impl IntoResponse {
    match manager::admin_list_users(&pool).await {
        Ok(users) => (StatusCode::OK, Json(users)).into_response(),
        Err(err_msg) => {
            tracing::error!("Error listing users: {err_msg}");
            (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({ "error": err_msg }))).into_response()
        }
    }
}

/// Creates a new non-admin user account.
///
/// Only admin users may call this endpoint. Delegates to
/// [`manager::admin_create_user`], which hashes the password and enforces
/// username uniqueness.
///
/// # Parameters
///
/// - `_auth` — the authenticated admin user (used only to enforce admin access).
/// - `pool` — the SQLite connection pool.
/// - `form` — the new user's `username` and `password`, deserialized from the JSON request body.
///
/// # Returns
///
/// `201 Created` with `{ "status": "created" }` on success.
///
/// # Errors
///
/// Returns `403 Forbidden` (via `AuthAdmin`) if the caller is not an admin.
/// Returns `400 Bad Request` with a JSON `{ "error": "..." }` body if the
/// username is already taken, the password fails validation, or the insert fails.
pub async fn handle_admin_create_user(
    _auth: AuthAdmin,
    State(pool): State<SqlitePool>,
    Json(form): Json<CreateUserForm>,
) -> impl IntoResponse {
    match manager::admin_create_user(&pool, &form.username, &form.password).await {
        Ok(_) => {
            tracing::info!("Admin created user: {}", form.username);
            (StatusCode::CREATED, Json(serde_json::json!({ "status": "created" })))
        }
        Err(err_msg) => {
            tracing::error!("Error creating user: {err_msg}");
            (StatusCode::BAD_REQUEST, Json(serde_json::json!({ "error": err_msg })))
        }
    }
}

/// Changes a target user's password (admin operation).
///
/// Only admin users may call this endpoint. The admin provides the target
/// user's ID and the new plaintext password. The new password is hashed via
/// argon2id before storage. Delegates to [`manager::admin_change_password`].
///
/// # Parameters
///
/// - `_auth` — the authenticated admin user (used only to enforce admin access).
/// - `pool` — the SQLite connection pool.
/// - `form` — contains `target_user_id` (the user whose password changes) and
///   `new_password`, deserialized from the JSON request body.
///
/// # Returns
///
/// `200 OK` with `{ "status": "updated" }` on success.
///
/// # Errors
///
/// Returns `403 Forbidden` (via `AuthAdmin`) if the caller is not an admin.
/// Returns `400 Bad Request` with a JSON `{ "error": "..." }` body if the
/// target user does not exist, the password fails validation, or the update fails.
pub async fn handle_admin_change_password(
    _auth: AuthAdmin,
    State(pool): State<SqlitePool>,
    Json(form): Json<ChangePasswordForm>,
) -> impl IntoResponse {
    match manager::admin_change_password(&pool, form.target_user_id, &form.new_password).await {
        Ok(_) => {
            tracing::info!("Admin changed password for user_id: {}", form.target_user_id);
            (StatusCode::OK, Json(serde_json::json!({ "status": "updated" })))
        }
        Err(err_msg) => {
            tracing::error!("Error changing password: {err_msg}");
            (StatusCode::BAD_REQUEST, Json(serde_json::json!({ "error": err_msg })))
        }
    }
}

/// Deletes a user account and all their associated data.
///
/// Only admin users may call this endpoint. Self-deletion (deleting one's own
/// account) is blocked by the manager layer. All of the target user's recipes,
/// meal plan entries, and cooked log entries are removed automatically via
/// `ON DELETE CASCADE`. Deleting a non-existent user is a no-op.
///
/// # Parameters
///
/// - `auth` — the authenticated admin user; `auth.user_id` is checked against
///   `id` to prevent self-deletion.
/// - `pool` — the SQLite connection pool.
/// - `id` — the primary key of the user to delete, extracted from the URL path.
///
/// # Returns
///
/// `200 OK` with `{ "status": "deleted" }` on success.
///
/// # Errors
///
/// Returns `403 Forbidden` (via `AuthAdmin`) if the caller is not an admin.
/// Returns `400 Bad Request` with a JSON `{ "error": "..." }` body if the
/// caller attempts to delete their own account or the delete query fails.
pub async fn handle_admin_delete_user(
    auth: AuthAdmin,
    State(pool): State<SqlitePool>,
    Path(id): Path<i64>,
) -> impl IntoResponse {
    match manager::admin_delete_user(&pool, auth.user_id, id).await {
        Ok(_) => {
            tracing::info!("Admin deleted user_id: {}", id);
            (StatusCode::OK, Json(serde_json::json!({ "status": "deleted" }))).into_response()
        }
        Err(err_msg) => {
            tracing::error!("Error deleting user {id}: {err_msg}");
            (StatusCode::BAD_REQUEST, Json(serde_json::json!({ "error": err_msg }))).into_response()
        }
    }
}

// ---------------------------------------------------------------------------
// Profile — self-service
// ---------------------------------------------------------------------------

/// Returns the authenticated user's own public profile as JSON.
///
/// The returned [`crate::model::UserInfo`] never includes the password hash.
/// Returns `404 Not Found` if the session references a user ID that no longer
/// exists in the database (e.g. an admin deleted the account mid-session).
///
/// # Parameters
///
/// - `auth` — the authenticated user; `auth.user_id` is used to look up the profile.
/// - `pool` — the SQLite connection pool.
///
/// # Returns
///
/// `200 OK` with the [`crate::model::UserInfo`] JSON on success;
/// `404 Not Found` if the user no longer exists.
///
/// # Errors
///
/// Returns `500 Internal Server Error` with a JSON `{ "error": "..." }` body
/// if the database query fails.
pub async fn handle_profile_me(
    auth: AuthUser,
    State(pool): State<SqlitePool>,
) -> impl IntoResponse {
    match manager::get_user_info_by_id(&pool, auth.user_id).await {
        Ok(Some(me)) => (StatusCode::OK, Json(me)).into_response(),
        Ok(None) => (StatusCode::NOT_FOUND, Json(serde_json::json!({ "error": "User not found" }))).into_response(),
        Err(err_msg) => {
            tracing::error!("Error loading profile: {err_msg}");
            (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({ "error": err_msg }))).into_response()
        }
    }
}

/// Serves the self-service password change page.
///
/// # Parameters
///
/// - `_auth` — the authenticated user extracted from the session (used only to
///   enforce authentication).
///
/// # Returns
///
/// `200 OK` with the profile HTML page.
///
/// # Errors
///
/// Returns a `302` redirect to `/login` (via the `AuthUser` extractor) if the
/// session is missing or expired.
pub async fn handle_profile_page(_auth: AuthUser) -> impl IntoResponse {
    Html(HTML_PROFILE)
}

/// Changes the authenticated user's own password.
///
/// Requires the user's current password for verification before applying the
/// change. Delegates to [`manager::change_own_password`].
///
/// # Parameters
///
/// - `auth` — the authenticated user; `auth.user_id` scopes the update.
/// - `pool` — the SQLite connection pool.
/// - `form` — contains `current_password` (for verification) and `new_password`,
///   deserialized from the JSON request body.
///
/// # Returns
///
/// `200 OK` with `{ "status": "updated" }` on success.
///
/// # Errors
///
/// Returns `400 Bad Request` with a JSON `{ "error": "..." }` body if the
/// current password is wrong, the new password fails validation, or the update fails.
pub async fn handle_change_own_password(
    auth: AuthUser,
    State(pool): State<SqlitePool>,
    Json(form): Json<SelfChangePasswordForm>,
) -> impl IntoResponse {
    match manager::change_own_password(&pool, auth.user_id, &form.current_password, &form.new_password).await {
        Ok(_) => {
            tracing::info!("User {} changed their password", auth.user_id);
            (StatusCode::OK, Json(serde_json::json!({ "status": "updated" }))).into_response()
        }
        Err(err_msg) => {
            tracing::error!("Password change error for user {}: {err_msg}", auth.user_id);
            (StatusCode::BAD_REQUEST, Json(serde_json::json!({ "error": err_msg }))).into_response()
        }
    }
}

// ---------------------------------------------------------------------------
// Error Handling
// ---------------------------------------------------------------------------

/// Fallback handler for unmatched routes — returns `404 Not Found`.
///
/// Registered via `Router::fallback` in `main.rs` and triggered for any
/// request that does not match a defined route.
///
/// # Parameters
///
/// None — this handler takes no parameters.
///
/// # Returns
///
/// `404 Not Found` with the 404 HTML page.
///
/// # Errors
///
/// This handler never fails.
pub async fn handle_404() -> Response {
    (StatusCode::NOT_FOUND, Html(HTML_404)).into_response()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use axum::extract::State;
    use http_body_util::BodyExt;
    use serde_json::from_slice;

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
    async fn test_handle_new_recipe_page() {
        let expected_html = HTML_ADD_RECIPE;
        let response = handle_new_recipe_page(AuthUser { user_id: 1 })
            .await
            .into_response();
        let body_bytes = response.into_body().collect().await.unwrap().to_bytes();
        let body_str = String::from_utf8(body_bytes.to_vec()).unwrap();
        assert_eq!(body_str, expected_html);
    }

    #[tokio::test]
    async fn test_handle_all_recipes_empty() {
        let pool = setup().await;
        let response = handle_all_recipes(AuthUser { user_id: 1 }, State(pool))
            .await
            .into_response();
        assert_eq!(response.status(), StatusCode::OK);
        let body_bytes = response.into_body().collect().await.unwrap().to_bytes();
        let recipes: Vec<Recipe> = from_slice(&body_bytes).unwrap();
        assert!(recipes.is_empty());
    }

    #[tokio::test]
    async fn test_handle_recipe_with_invalid_id() {
        let pool = setup().await;
        let response = handle_recipe(AuthUser { user_id: 1 }, State(pool.clone()), Path(999_999))
            .await
            .into_response();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn test_handle_get_meal_entries_valid_range() {
        let pool = setup().await;
        let params = DateRangeParams {
            start: NaiveDate::from_ymd_opt(2026, 1, 1).unwrap(),
            end: NaiveDate::from_ymd_opt(2026, 1, 7).unwrap(),
        };
        let response = handle_get_meal_entries(AuthUser { user_id: 1 }, State(pool), Query(params))
            .await
            .into_response();
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_handle_get_meal_entries_invalid_range() {
        let pool = setup().await;
        let params = DateRangeParams {
            start: NaiveDate::from_ymd_opt(2026, 1, 7).unwrap(),
            end: NaiveDate::from_ymd_opt(2026, 1, 1).unwrap(),
        };
        let response = handle_get_meal_entries(AuthUser { user_id: 1 }, State(pool), Query(params))
            .await
            .into_response();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn test_handle_shopping_list_empty_range() {
        let pool = setup().await;
        let params = DateRangeParams {
            start: NaiveDate::from_ymd_opt(2099, 1, 1).unwrap(),
            end: NaiveDate::from_ymd_opt(2099, 1, 7).unwrap(),
        };
        let response = handle_shopping_list(AuthUser { user_id: 1 }, State(pool), Query(params))
            .await
            .into_response();
        assert_eq!(response.status(), StatusCode::OK);
        let body_bytes = response.into_body().collect().await.unwrap().to_bytes();
        let ingredients: Vec<serde_json::Value> = from_slice(&body_bytes).unwrap();
        assert!(ingredients.is_empty());
    }

    #[tokio::test]
    async fn test_handle_profile_me_returns_current_user() {
        let pool = setup().await;
        let response = handle_profile_me(AuthUser { user_id: 1 }, State(pool))
            .await
            .into_response();
        assert_eq!(response.status(), StatusCode::OK);
        let body_bytes = response.into_body().collect().await.unwrap().to_bytes();
        let me: serde_json::Value = from_slice(&body_bytes).unwrap();
        assert_eq!(me["username"], "test");
        assert_eq!(me["id"], 1);
    }

    #[tokio::test]
    async fn test_handle_profile_me_user_not_found() {
        let pool = setup().await;
        let response = handle_profile_me(AuthUser { user_id: 999_999 }, State(pool))
            .await
            .into_response();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }
}
