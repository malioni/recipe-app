use recipe_app::network;
use axum::{
    routing::{delete, get, post},
    Router,
};
use sqlx::sqlite::SqlitePoolOptions;
use std::net::SocketAddr;
use tokio::net::TcpListener;

/// The ID of the single placeholder user.
/// All data is owned by this user until real authentication is implemented.
/// When auth is added, replace this constant with the session user's ID
/// in each manager function — the schema requires no changes.
pub const SINGLE_USER_ID: i64 = 1;

#[tokio::main]
async fn main() {
    // Create the db/ directory if it doesn't exist.
    std::fs::create_dir_all("db").expect("Failed to create db/ directory");

    // Open (or create) the SQLite database and configure the connection pool.
    let pool = SqlitePoolOptions::new()
        .max_connections(5)
        .connect("sqlite:db/recipe_app.db")
        .await
        .expect("Failed to connect to database");

    // Run migrations. include_str! embeds the SQL at compile time so the
    // migrations file must be present when building but not at runtime.
    sqlx::query(include_str!("../migrations/001_initial.sql"))
        .execute(&pool)
        .await
        .expect("Failed to run migrations");

    // Insert the placeholder user if they don't already exist.
    // password_hash is a placeholder — it is never used for authentication
    // until real auth is implemented.
    sqlx::query(
        "INSERT OR IGNORE INTO users (id, username, password_hash) VALUES (?, 'default', 'placeholder')"
    )
    .bind(SINGLE_USER_ID)
    .execute(&pool)
    .await
    .expect("Failed to insert placeholder user");

    let app = Router::new()
        // Recipes
        .route("/", get(network::handle_index))
        .route("/recipes", get(network::handle_all_recipes).post(network::handle_add_recipe))
        .route("/recipes/new", get(network::handle_new_recipe_page))
        .route("/recipes/edit", get(network::handle_new_recipe_page))
        .route("/recipes/:id", get(network::handle_recipe).put(network::handle_update_recipe))
        .route("/recipes/:id/delete", post(network::handle_delete_recipe))
        // Calendar
        .route("/calendar", get(network::handle_calendar_page))
        .route("/calendar/entries", get(network::handle_get_meal_entries)
            .post(network::handle_plan_meal)
            .delete(network::handle_delete_meal_entry))
        .route("/calendar/cooked", get(network::handle_get_cooked_entries)
            .post(network::handle_mark_cooked))
        .route("/calendar/shopping-list", get(network::handle_shopping_list))
        .with_state(pool);

    let addr = SocketAddr::from(([127, 0, 0, 1], 7878));
    let listener = TcpListener::bind(addr).await.unwrap();
    println!("Listening on {addr}");
    axum::serve(listener, app).await.unwrap();
}