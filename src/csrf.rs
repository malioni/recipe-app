/// CSRF protection middleware.
///
/// Applies an `Origin`-check to all state-mutating requests (POST, PUT,
/// DELETE, PATCH) as defence-in-depth alongside the `SameSite=Strict` session
/// cookie. If an `Origin` header is present and its host does not match the
/// request's `Host` header, the middleware returns 403 immediately.
///
/// Absent `Origin` is allowed: browsers always send `Origin` for cross-site
/// fetches, so its absence means the request is same-origin or from a
/// non-browser client (e.g. curl, Apple Shortcuts).
use axum::{
    body::Body,
    http::{Method, Request, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
};

/// Middleware that rejects cross-origin mutating requests.
///
/// Apply to the authenticated sub-router via `middleware::from_fn(csrf::check_csrf)`.
pub async fn check_csrf(req: Request<Body>, next: Next) -> Response {
    // Read-only methods are never a CSRF risk.
    if matches!(
        req.method(),
        &Method::GET | &Method::HEAD | &Method::OPTIONS
    ) {
        return next.run(req).await;
    }

    // Extract the Host header value. Without a known host we cannot validate,
    // so we pass through rather than producing a false positive.
    let host = match req.headers().get("host").and_then(|v| v.to_str().ok()) {
        Some(h) => h.to_owned(),
        None => return next.run(req).await,
    };

    // Extract the Origin header. Absent means same-origin or non-browser — allow.
    let origin = match req.headers().get("origin").and_then(|v| v.to_str().ok()) {
        Some(o) => o.to_owned(),
        None => return next.run(req).await,
    };

    // Parse the origin URL and extract its host+port for comparison.
    // Expected format: "scheme://host" or "scheme://host:port".
    let origin_host = origin
        .trim_start_matches("http://")
        .trim_start_matches("https://")
        .split('/')
        .next()
        .unwrap_or("");

    if origin_host != host {
        return (StatusCode::FORBIDDEN, "CSRF check failed").into_response();
    }

    next.run(req).await
}
