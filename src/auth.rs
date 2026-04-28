/// Authentication module.
///
/// Provides:
///   - `hash_password` / `verify_password` — argon2id wrappers
///   - `AuthUser` — an Axum extractor that validates the session on every
///     protected request and returns the authenticated user's ID.
///   - `AuthAdmin` — an Axum extractor that additionally requires the
///     authenticated user to have `is_admin = true`.
///
/// Multi-user support is fully implemented. All domain tables (`recipes`,
/// `meal_plan`, `cooked_log`) carry a `user_id` column and every query is
/// scoped to the authenticated user. To protect a handler, add `auth: AuthUser`
/// (or `auth: AuthAdmin` for admin-only routes) as a parameter — the extractor
/// handles the redirect or 403 response automatically; individual handlers
/// never check authentication themselves.
use argon2::{
    password_hash::{rand_core::OsRng, PasswordHash, PasswordHasher, PasswordVerifier, SaltString},
    Argon2,
};
use axum::{
    async_trait,
    extract::FromRequestParts,
    http::{request::Parts, StatusCode},
    response::{IntoResponse, Redirect, Response},
};
use tower_sessions::Session;

/// The session key under which the authenticated user's ID is stored.
pub const SESSION_USER_ID_KEY: &str = "user_id";

/// The session key under which the authenticated user's admin flag is stored.
pub const SESSION_IS_ADMIN_KEY: &str = "is_admin";

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
/// Add this as a parameter to any handler that requires authentication.
/// The extractor reads the session and returns the authenticated user's ID
/// via `auth.user_id`. If the session is missing or expired, the extractor
/// returns a redirect to `/login` automatically — the handler code never runs.
pub struct AuthUser {
    pub user_id: i64,
}

/// The error type returned when the extractor cannot authenticate the request.
/// Always redirects to the login page rather than returning a 401, since this
/// is a browser-facing app.
#[derive(Debug)]
pub struct AuthRedirect;

/// The error type returned when an authenticated user lacks admin privileges.
/// Returns 403 Forbidden rather than redirecting to login.
#[derive(Debug)]
pub struct AuthForbidden;

impl IntoResponse for AuthRedirect {
    fn into_response(self) -> Response {
        Redirect::to("/login").into_response()
    }
}

impl IntoResponse for AuthForbidden {
    fn into_response(self) -> Response {
        (StatusCode::FORBIDDEN, "Forbidden").into_response()
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

/// Represents an authenticated admin user extracted from the session.
///
/// Add this as a parameter to any handler that requires admin privileges.
/// Returns 403 Forbidden if the session is missing, expired, or the user
/// does not have `is_admin = true`. The handler code never runs in that case.
pub struct AuthAdmin {
    pub user_id: i64,
}

#[async_trait]
impl<S> FromRequestParts<S> for AuthAdmin
where
    S: Send + Sync,
{
    type Rejection = AuthForbidden;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let session = Session::from_request_parts(parts, state)
            .await
            .map_err(|_| AuthForbidden)?;

        let user_id: i64 = session
            .get(SESSION_USER_ID_KEY)
            .await
            .map_err(|_| AuthForbidden)?
            .ok_or(AuthForbidden)?;

        let is_admin: bool = session
            .get(SESSION_IS_ADMIN_KEY)
            .await
            .map_err(|_| AuthForbidden)?
            .unwrap_or(false);

        if !is_admin {
            return Err(AuthForbidden);
        }

        Ok(AuthAdmin { user_id })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::Request;
    use std::sync::Arc;
    use tower_sessions::{MemoryStore, Session};

    /// Build a minimal `Parts` with the given session injected into extensions,
    /// mirroring what `SessionManagerLayer` does at runtime.
    fn parts_with_session(session: Session) -> axum::http::request::Parts {
        let (mut parts, _) = Request::builder().body(()).unwrap().into_parts();
        parts.extensions.insert(session);
        parts
    }

    fn make_session() -> Session {
        Session::new(None, Arc::new(MemoryStore::default()), None)
    }

    // -- AuthUser -------------------------------------------------------------

    /// No session in extensions → redirect to /login.
    #[tokio::test]
    async fn test_auth_user_no_session_redirects() {
        let (mut parts, _) = Request::builder().body(()).unwrap().into_parts();
        let result = AuthUser::from_request_parts(&mut parts, &()).await;
        assert!(result.is_err());
    }

    /// Session present but `user_id` key missing → redirect to /login.
    #[tokio::test]
    async fn test_auth_user_missing_user_id_redirects() {
        let session = make_session();
        let mut parts = parts_with_session(session);
        let result = AuthUser::from_request_parts(&mut parts, &()).await;
        assert!(result.is_err());
    }

    /// Session present and `user_id` set → returns the correct `AuthUser`.
    #[tokio::test]
    async fn test_auth_user_valid_session_returns_user_id() {
        let session = make_session();
        session.insert(SESSION_USER_ID_KEY, 42i64).await.unwrap();
        let mut parts = parts_with_session(session);
        let auth = AuthUser::from_request_parts(&mut parts, &()).await.unwrap();
        assert_eq!(auth.user_id, 42);
    }

    // -- AuthAdmin ------------------------------------------------------------

    /// Session present but `user_id` key missing → 403 Forbidden.
    #[tokio::test]
    async fn test_auth_admin_missing_user_id_forbidden() {
        let session = make_session();
        let mut parts = parts_with_session(session);
        let result = AuthAdmin::from_request_parts(&mut parts, &()).await;
        assert!(result.is_err());
    }

    /// Session has `user_id` but `is_admin` is false → 403 Forbidden.
    #[tokio::test]
    async fn test_auth_admin_not_admin_forbidden() {
        let session = make_session();
        session.insert(SESSION_USER_ID_KEY, 1i64).await.unwrap();
        session.insert(SESSION_IS_ADMIN_KEY, false).await.unwrap();
        let mut parts = parts_with_session(session);
        let result = AuthAdmin::from_request_parts(&mut parts, &()).await;
        assert!(result.is_err());
    }

    /// Session has `user_id` but `is_admin` key is absent → defaults to false → 403.
    #[tokio::test]
    async fn test_auth_admin_missing_is_admin_key_forbidden() {
        let session = make_session();
        session.insert(SESSION_USER_ID_KEY, 1i64).await.unwrap();
        // is_admin key intentionally not set; unwrap_or(false) should deny
        let mut parts = parts_with_session(session);
        let result = AuthAdmin::from_request_parts(&mut parts, &()).await;
        assert!(result.is_err());
    }

    /// Session has `user_id` and `is_admin = true` → returns the correct `AuthAdmin`.
    #[tokio::test]
    async fn test_auth_admin_valid_session_returns_user_id() {
        let session = make_session();
        session.insert(SESSION_USER_ID_KEY, 7i64).await.unwrap();
        session.insert(SESSION_IS_ADMIN_KEY, true).await.unwrap();
        let mut parts = parts_with_session(session);
        let auth = AuthAdmin::from_request_parts(&mut parts, &()).await.unwrap();
        assert_eq!(auth.user_id, 7);
    }
}