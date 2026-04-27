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
/// The inner value is private to prevent callers from constructing a
/// spoofed extension outside this module.
#[derive(Clone, Debug)]
pub struct SessionUserId(i64);

impl SessionUserId {
    /// Returns the authenticated user's ID.
    pub fn user_id(&self) -> i64 {
        self.0
    }
}

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
        match session.get::<i64>(SESSION_USER_ID_KEY).await {
            Ok(Some(uid)) => {
                req.extensions_mut().insert(SessionUserId(uid));
            }
            Ok(None) => {}
            Err(e) => {
                tracing::warn!("session read error in rate-limit middleware: {e}");
            }
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
            return Ok(format!("u:{}", uid.user_id()));
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
    use axum::{extract::ConnectInfo, http::Request};
    use std::net::SocketAddr;
    use tower_governor::errors::GovernorError;

    fn make_request_with_user(user_id: i64) -> Request<()> {
        let mut req = Request::new(());
        req.extensions_mut().insert(SessionUserId(user_id)); // allowed: same module
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
        let key = UserIdKeyExtractor.extract(&req).unwrap();
        assert_eq!(key, "u:42");
    }

    #[test]
    fn test_user_key_extractor_falls_back_to_ip() {
        let req = make_request_with_ip("127.0.0.1", 1234);
        let key = UserIdKeyExtractor.extract(&req).unwrap();
        assert_eq!(key, "ip:127.0.0.1");
    }

    #[test]
    fn test_user_key_extractor_user_id_beats_ip_when_both_present() {
        // When both extensions are present user ID takes priority.
        let addr: SocketAddr = "10.0.0.1:80".parse().unwrap();
        let mut req = Request::new(());
        req.extensions_mut().insert(SessionUserId(7));
        req.extensions_mut().insert(ConnectInfo(addr));
        let key = UserIdKeyExtractor.extract(&req).unwrap();
        assert_eq!(key, "u:7");
    }

    #[test]
    fn test_user_key_extractor_no_info_returns_error() {
        let req = Request::new(());
        let result = UserIdKeyExtractor.extract(&req);
        assert!(matches!(result, Err(GovernorError::UnableToExtractKey)));
    }

    #[test]
    fn test_user_key_extractor_different_user_ids_produce_distinct_keys() {
        let req_a = make_request_with_user(1);
        let req_b = make_request_with_user(2);
        let key_a = UserIdKeyExtractor.extract(&req_a).unwrap();
        let key_b = UserIdKeyExtractor.extract(&req_b).unwrap();
        assert_ne!(key_a, key_b);
    }

    #[test]
    fn test_user_key_extractor_different_ips_produce_distinct_keys() {
        let req_a = make_request_with_ip("192.168.1.1", 80);
        let req_b = make_request_with_ip("192.168.1.2", 80);
        let key_a = UserIdKeyExtractor.extract(&req_a).unwrap();
        let key_b = UserIdKeyExtractor.extract(&req_b).unwrap();
        assert_ne!(key_a, key_b);
    }

    #[test]
    fn test_user_key_extractor_ip_key_contains_address() {
        let req = make_request_with_ip("10.20.30.40", 9000);
        let key = UserIdKeyExtractor.extract(&req).unwrap();
        assert!(key.contains("10.20.30.40"), "key should contain the IP: {key}");
    }
}
