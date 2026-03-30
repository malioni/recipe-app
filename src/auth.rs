/// Authentication module.
///
/// Provides:
///   - `hash_password` / `verify_password` — argon2 wrappers
///   - `AuthUser` — an Axum extractor that validates the session on every
///     protected request and returns the authenticated user's ID.
///
/// To protect a handler, add `auth: AuthUser` as a parameter. If no valid
/// session exists, the extractor automatically redirects to `/login` so
/// individual handlers never need to check authentication themselves.
///
/// When multi-user support is fully implemented, replace `SINGLE_USER_ID`
/// in `manager.rs` and `calendar_manager.rs` with `auth.user_id`.
use argon2::{
    password_hash::{rand_core::OsRng, PasswordHash, PasswordHasher, PasswordVerifier, SaltString},
    Argon2,
};
use axum::{
    async_trait,
    extract::FromRequestParts,
    http::request::Parts,
    response::{IntoResponse, Redirect, Response},
};
use tower_sessions::Session;

/// The session key under which the authenticated user's ID is stored.
pub const SESSION_USER_ID_KEY: &str = "user_id";

// ---------------------------------------------------------------------------
// Password hashing
// ---------------------------------------------------------------------------

/// Hashes a plaintext password using argon2id with a random salt.
///
/// The returned string is self-contained (algorithm + salt + hash) and
/// can be stored directly in the `users.password_hash` column.
///
/// # Errors
///
/// Returns `Err` if hashing fails (this should not happen in practice).
pub fn hash_password(password: &str) -> Result<String, String> {
    let salt = SaltString::generate(&mut OsRng);
    let argon2 = Argon2::default();
    argon2
        .hash_password(password.as_bytes(), &salt)
        .map(|h| h.to_string())
        .map_err(|e| format!("Failed to hash password: {e}"))
}

/// Verifies a plaintext password against a stored argon2 hash string.
///
/// Returns `true` if the password matches, `false` otherwise.
/// Returns `Err` only if the stored hash string is malformed.
pub fn verify_password(password: &str, hash: &str) -> Result<bool, String> {
    let parsed = PasswordHash::new(hash)
        .map_err(|e| format!("Malformed password hash: {e}"))?;
    Ok(Argon2::default()
        .verify_password(password.as_bytes(), &parsed)
        .is_ok())
}

// ---------------------------------------------------------------------------
// AuthUser extractor
// ---------------------------------------------------------------------------

/// Represents an authenticated user extracted from the session.
///
/// Add this as a parameter to any handler that requires authentication:
///
/// ```rust
/// pub async fn handle_something(
///     auth: AuthUser,
///     State(pool): State<SqlitePool>,
/// ) -> impl IntoResponse {
///     // auth.user_id is the authenticated user's ID
/// }
/// ```
///
/// If the session is missing or expired, the extractor returns a redirect
/// to `/login` automatically — the handler code never runs.
pub struct AuthUser {
    pub user_id: i64,
}

/// The error type returned when the extractor cannot authenticate the request.
/// Always redirects to the login page rather than returning a 401, since this
/// is a browser-facing app.
pub struct AuthRedirect;

impl IntoResponse for AuthRedirect {
    fn into_response(self) -> Response {
        Redirect::to("/login").into_response()
    }
}

#[async_trait]
impl<S> FromRequestParts<S> for AuthUser
where
    S: Send + Sync,
{
    type Rejection = AuthRedirect;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        // Extract the session from the request. tower-sessions injects this
        // via middleware so it is always present — if it isn't, the middleware
        // is misconfigured and we treat it as unauthenticated.
        let session = Session::from_request_parts(parts, state)
            .await
            .map_err(|_| AuthRedirect)?;

        // Look up the user_id stored in the session.
        let user_id: i64 = session
            .get(SESSION_USER_ID_KEY)
            .await
            .map_err(|_| AuthRedirect)?
            .ok_or(AuthRedirect)?;

        Ok(AuthUser { user_id })
    }
}