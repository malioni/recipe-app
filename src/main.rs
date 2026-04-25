use recipe_app::{auth, network, storage};
use axum::{
    extract::DefaultBodyLimit,
    middleware::{self, Next},
    response::Response,
    http::Request,
    routing::{get, post},
    Router,
};
use tower_http::services::ServeDir;
use std::sync::Arc;
use tower_governor::{
    governor::GovernorConfigBuilder, 
    GovernorLayer,
    key_extractor::PeerIpKeyExtractor,
};
use sqlx::sqlite::SqlitePoolOptions;
use std::net::SocketAddr;
use time::Duration as TimeDuration;
use tokio::net::TcpListener;
use tower_sessions::{Expiry, SessionManagerLayer};
use tower_sessions_sqlx_store::SqliteStore;

#[tokio::main]
async fn main() {
    // Initialise structured logging. The RUST_LOG environment variable
    // controls the log level (e.g. RUST_LOG=info cargo run).
    // Defaults to "info" if RUST_LOG is not set.
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    // Load .env file into the process environment if it exists.
    // This is a no-op if the file is absent, so it is safe in all environments.
    dotenvy::dotenv().ok();

    // Create the db/ directory if it doesn't exist.
    std::fs::create_dir_all("db").expect("Failed to create db/ directory");

    // Open (or create) the SQLite database and configure the connection pool.
    let pool = SqlitePoolOptions::new()
        .max_connections(5)
        .connect("sqlite://db/recipe_app.db?mode=rwc")
        .await
        .expect("Failed to connect to database");

    tracing::info!("Connected to database");

    // Run migrations. include_str! embeds the SQL at compile time so the
    // migrations file must be present when building but not at runtime.
    sqlx::query(include_str!("../migrations/001_initial.sql"))
        .execute(&pool)
        .await
        .expect("Failed to run migration 001");
    sqlx::query(include_str!("../migrations/002_multiple_entries_per_slot.sql"))
        .execute(&pool)
        .await
        .expect("Failed to run migration 002");

    // Seed the initial user from environment variables if no users exist yet.
    // Set INITIAL_USERNAME and INITIAL_PASSWORD in your .env file before
    // first boot. After the user is created these variables are no longer read.
    if !storage::any_users_exist(&pool).await.expect("Failed to check users") {
        let username = std::env::var("INITIAL_USERNAME")
            .expect("No users exist. Set INITIAL_USERNAME in your .env file.");
        let password = std::env::var("INITIAL_PASSWORD")
            .expect("No users exist. Set INITIAL_PASSWORD in your .env file.");
        let hash = auth::hash_password(&password)
            .expect("Failed to hash initial password");
        storage::create_user(&pool, &username, &hash)
            .await
            .expect("Failed to create initial user");
        tracing::info!("Created initial user: {}", username);
    }

    // Configure the session store backed by SQLite so sessions survive restarts.
    let session_store = SqliteStore::new(pool.clone());
    session_store
        .migrate()
        .await
        .expect("Failed to migrate session store");

    // Sessions expire after 7 days of inactivity.
    let session_layer = SessionManagerLayer::new(session_store)
        .with_secure(false)          // set to true when serving over HTTPS
        .with_expiry(Expiry::OnInactivity(TimeDuration::days(7)));

    // Rate limiting: 60 requests per minute per IP address.
    // replenish_rate is tokens added per second; burst_size is the maximum
    // number of requests that can be made in a burst before throttling kicks in.
    let governor_conf = Arc::new(
        GovernorConfigBuilder::default()
            .key_extractor(PeerIpKeyExtractor)
            .per_second(5)
            .burst_size(60)
            .finish()
            .expect("Failed to build rate limiter config"),
    );
    let governor_layer = GovernorLayer {
        config: governor_conf,
    };

    let app = Router::new()
        // Auth
        .route("/login",  get(network::handle_login_page).post(network::handle_login))
        .route("/logout", post(network::handle_logout))
        // Recipes
        .route("/", get(network::handle_index))
        .route("/recipes", get(network::handle_all_recipes).post(network::handle_add_recipe))
        .route("/recipes/new", get(network::handle_new_recipe_page))
        .route("/recipes/edit", get(network::handle_new_recipe_page))
        .route("/recipes/:id", get(network::handle_recipe).put(network::handle_update_recipe).delete(network::handle_delete_recipe))
        // Calendar
        .route("/calendar", get(network::handle_calendar_page))
        .route("/calendar/entries", get(network::handle_get_meal_entries)
            .post(network::handle_plan_meal)
            .delete(network::handle_delete_meal_entry))
        .route("/calendar/cooked", get(network::handle_get_cooked_entries)
            .post(network::handle_mark_cooked))
        .route("/calendar/shopping-list", get(network::handle_shopping_list))
        .fallback(network::handle_404)
        .nest_service("/static", ServeDir::new("static"))
        .layer(middleware::from_fn(add_csp_header))
        // Middleware — order matters: session wraps all routes, body limit
        // is applied first so oversized requests are rejected before any
        // handler or session logic runs.
        .layer(session_layer)
        .layer(DefaultBodyLimit::max(64 * 1024)) // 64KB max request body
        .layer(governor_layer)
        .with_state(pool);

    let addr = SocketAddr::from(([127, 0, 0, 1], 7878));
    let listener = TcpListener::bind(addr).await.unwrap();
    tracing::info!("Listening on {addr}");
    axum::serve(listener, app.into_make_service_with_connect_info::<SocketAddr>())
        .await
        .unwrap();
}

async fn add_csp_header(request: Request<axum::body::Body>, next: Next) -> Response {
    let mut response = next.run(request).await;
    response.headers_mut().insert(
        axum::http::header::CONTENT_SECURITY_POLICY,
        // Allows Bootstrap and Bootstrap Icons from jsdelivr.net.
        // 'self' covers all app-served HTML, JS, and API responses.
        "default-src 'self'; \
         script-src 'self' https://cdn.jsdelivr.net; \
         style-src 'self' 'unsafe-inline' https://cdn.jsdelivr.net; \
         font-src 'self' https://cdn.jsdelivr.net; \
         img-src 'self' data:; \
         connect-src 'self'"
            .parse()
            .unwrap(),
    );
    response
}