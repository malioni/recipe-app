use recipe_app::network;
use axum::{
    routing::get,
    Router,
};
use std::net::SocketAddr;
use tokio::net::TcpListener;

#[tokio::main]
async fn main() {
    let app = Router::new()
        .route("/", get(network::handle_index))
        .route("/recipes/:id", get(network::handle_recipe));

    let addr = SocketAddr::from(([127, 0, 0, 1], 7878));
    let listener = TcpListener::bind(addr).await.unwrap();
    println!("Listening on {addr}");
    axum::serve(listener, app)
        .await
        .unwrap();
}
