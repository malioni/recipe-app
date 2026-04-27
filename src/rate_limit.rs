/// Rate limiting utilities.
///
/// Provides:
///   - `SessionUserId` — a typed request extension that carries the authenticated
///     user's ID from `inject_user_id` to `UserIdKeyExtractor`.
///   - `inject_user_id` — an Axum `from_fn` middleware that reads the session and
///     inserts `SessionUserId` when the request belongs to an authenticated user.
///   - `UserIdKeyExtractor` — a `tower_governor::KeyExtractor` that keys rate
///     limits by user ID, falling back to peer IP for unauthenticated requests.
use std::net::SocketAddr;

use axum::{
    body::Body,
    extract::ConnectInfo,
    http::Request,
    middleware::Next,
    response::Response,
};
use tower_governor::{errors::GovernorError, key_extractor::KeyExtractor};
use tower_sessions::Session;

use crate::auth::SESSION_USER_ID_KEY;

// ---------------------------------------------------------------------------
// Typed extension
// ---------------------------------------------------------------------------

/// Typed request extension that holds the authenticated user's ID.
///
/// Inserted by `inject_user_id` and consumed by `UserIdKeyExtractor`.
#[derive(Clone, Debug)]
pub struct SessionUserId(pub i64);

// ---------------------------------------------------------------------------
// Middleware
// ---------------------------------------------------------------------------

/// Reads the session's `user_id` and inserts it as a `SessionUserId` extension.
///
/// Must run inside the `SessionManagerLayer` so the `Session` extension is
/// already populated. When there is no authenticated user in the session the
/// function is a no-op — the extension is simply not inserted.
pub async fn inject_user_id(mut req: Request<Body>, next: Next) -> Response {
    if let Some(session) = req.extensions().get::<Session>().cloned() {
        let user_id: Option<i64> = session.get(SESSION_USER_ID_KEY).await.ok().flatten();
        if let Some(uid) = user_id {
            req.extensions_mut().insert(SessionUserId(uid));
        }
    }
    next.run(req).await
}

// ---------------------------------------------------------------------------
// KeyExtractor
// ---------------------------------------------------------------------------

/// Rate-limit key extractor that identifies requests by authenticated user ID.
///
/// Key priority:
/// 1. `SessionUserId` extension present (set by `inject_user_id`) → `"u:{id}"`
/// 2. `ConnectInfo<SocketAddr>` extension present (unauthenticated request) →
///    `"ip:{addr}"` — falls back to IP so unauthenticated requests are still
///    limited without returning a 500 before `AuthUser` can redirect them.
/// 3. Neither present → `Err(UnableToExtractKey)`.
#[derive(Clone, Debug)]
pub struct UserIdKeyExtractor;

impl KeyExtractor for UserIdKeyExtractor {
    type Key = String;

    fn extract<T>(&self, req: &Request<T>) -> Result<Self::Key, GovernorError> {
        if let Some(uid) = req.extensions().get::<SessionUserId>() {
            return Ok(format!("u:{}", uid.0));
        }
        if let Some(addr) = req
            .extensions()
            .get::<ConnectInfo<SocketAddr>>()
            .map(|ci| ci.0.ip())
        {
            return Ok(format!("ip:{addr}"));
        }
        Err(GovernorError::UnableToExtractKey)
    }
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use axum::extract::ConnectInfo;
    use http::Request;
    use std::net::SocketAddr;
    use tower_governor::errors::GovernorError;

    fn make_request_with_user(user_id: i64) -> Request<()> {
        let mut req = Request::new(());
        req.extensions_mut().insert(SessionUserId(user_id));
        req
    }

    fn make_request_with_ip(ip: &str, port: u16) -> Request<()> {
        let addr: SocketAddr = format!("{ip}:{port}").parse().unwrap();
        let mut req = Request::new(());
        req.extensions_mut().insert(ConnectInfo(addr));
        req
    }

    #[test]
    fn test_user_key_extractor_prefers_session_user_id() {
        let req = make_request_with_user(42);
        let result = UserIdKeyExtractor.extract(&req);
        assert_eq!(result, Ok("u:42".to_string()));
    }

    #[test]
    fn test_user_key_extractor_falls_back_to_ip() {
        let req = make_request_with_ip("127.0.0.1", 1234);
        let result = UserIdKeyExtractor.extract(&req);
        assert_eq!(result, Ok("ip:127.0.0.1".to_string()));
    }

    #[test]
    fn test_user_key_extractor_no_info_returns_error() {
        let req = Request::new(());
        let result = UserIdKeyExtractor.extract(&req);
        assert!(matches!(result, Err(GovernorError::UnableToExtractKey)));
    }
}
