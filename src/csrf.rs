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
    // splitn(3, '/') on "http://host:port" yields ["http:", "", "host:port"];
    // nth(2) then gives the authority component without a repeated-strip risk.
    let origin_host = origin
        .splitn(3, '/')
        .nth(2)
        .and_then(|s| s.split('/').next())
        .unwrap_or("");

    if origin_host != host {
        return (StatusCode::FORBIDDEN, "CSRF check failed").into_response();
    }

    next.run(req).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        middleware,
        routing::get,
        Router,
    };
    use tower::ServiceExt;

    async fn ok() -> StatusCode {
        StatusCode::OK
    }

    fn app() -> Router {
        Router::new()
            .route("/", get(ok).post(ok).put(ok).delete(ok))
            .layer(middleware::from_fn(check_csrf))
    }

    fn make_req(method: &str, origin: Option<&str>, host: Option<&str>) -> Request<Body> {
        let mut builder = Request::builder().method(method).uri("/");
        if let Some(o) = origin {
            builder = builder.header("origin", o);
        }
        if let Some(h) = host {
            builder = builder.header("host", h);
        }
        builder.body(Body::empty()).unwrap()
    }

    #[tokio::test]
    async fn test_csrf_allows_get_with_cross_origin() {
        // GET is never a CSRF risk — allowed regardless of Origin.
        let res = app()
            .oneshot(make_req("GET", Some("http://evil.com"), Some("127.0.0.1:7878")))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_csrf_allows_post_no_origin() {
        // No Origin header → same-origin or non-browser client → allow.
        let res = app()
            .oneshot(make_req("POST", None, Some("127.0.0.1:7878")))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_csrf_allows_post_same_origin() {
        // Origin matches Host → same-origin fetch → allow.
        let res = app()
            .oneshot(make_req(
                "POST",
                Some("http://127.0.0.1:7878"),
                Some("127.0.0.1:7878"),
            ))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_csrf_blocks_post_cross_origin() {
        // Origin differs from Host → cross-origin POST → 403.
        let res = app()
            .oneshot(make_req(
                "POST",
                Some("http://evil.com"),
                Some("127.0.0.1:7878"),
            ))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn test_csrf_blocks_put_cross_origin() {
        // PUT is a mutating method — also blocked when Origin mismatches.
        let res = app()
            .oneshot(make_req(
                "PUT",
                Some("http://evil.com"),
                Some("127.0.0.1:7878"),
            ))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn test_csrf_blocks_delete_cross_origin() {
        // DELETE is a mutating method — also blocked when Origin mismatches.
        let res = app()
            .oneshot(make_req(
                "DELETE",
                Some("http://evil.com"),
                Some("127.0.0.1:7878"),
            ))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn test_csrf_allows_post_no_host() {
        // No Host header → cannot validate → pass through rather than false-positive.
        let res = app()
            .oneshot(make_req("POST", Some("http://evil.com"), None))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
    }
}
