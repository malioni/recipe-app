use recipe_app::network;
use axum::{
    routing::{get, post},
    Router,
};
use std::net::SocketAddr;
use tokio::net::TcpListener;

#[tokio::main]
async fn main() {
    let app = Router::new()
        .route("/", get(network::handle_index))
        .route("/recipes", get(network::handle_all_recipes).post(network::handle_add_recipe))
        .route("/recipes/new", get(network::handle_new_recipe_page))
        .route("/recipes/edit", get(network::handle_new_recipe_page))
        .route("/recipes/:id", get(network::handle_recipe).put(network::handle_update_recipe))
        .route("/recipes/:id/delete", post(network::handle_delete_recipe));

    let addr = SocketAddr::from(([127, 0, 0, 1], 7878));
    let listener = TcpListener::bind(addr).await.unwrap();
    println!("Listening on {addr}");
    axum::serve(listener, app).await.unwrap();
}