use axum::{
    response::{IntoResponse, Html},
    extract::{Path, Json},
    http::StatusCode,
};
use crate::manager;
use crate::model::Recipe;

pub async fn handle_index() -> impl IntoResponse {
    Html(std::fs::read_to_string("html/index.html").unwrap_or_else(|_| "<h1>Error</h1>".to_string()))
}

pub async fn handle_all_recipes() -> Json<Vec<Recipe>> {
    Json(manager::get_all_recipes())
}

pub async fn handle_recipe(Path(id): Path<u32>) -> impl IntoResponse {
    match manager::get_recipe_by_id(id) {
        Some(recipe) => (StatusCode::OK, Json(recipe)).into_response(),
        None => (StatusCode::NOT_FOUND, Json(serde_json::json!({ "error": format!("Recipe with ID {} not found", id) }))).into_response(),
    }
}

pub async fn handle_new_recipe_page() -> impl IntoResponse {
    Html(std::fs::read_to_string("html/add-recipe.html").unwrap_or_else(|_| "<h1>Error</h1>".to_string()))
}

pub async fn handle_add_recipe(Json(new_recipe): Json<Recipe>) -> impl IntoResponse {
    match manager::add_recipe(new_recipe) {
        Ok(_) => (StatusCode::CREATED, Json(serde_json::json!({ "status": "created" }))),
        Err(err_msg) => {
            eprintln!("Error saving recipe: {err_msg}");
            (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({ "error": err_msg })))
        }
    }
}

pub async fn handle_delete_recipe(Path(id): Path<u32>) -> impl IntoResponse {
    match manager::delete_recipe(id) {
        Ok(_) => (StatusCode::OK, Json(serde_json::json!({ "status": "deleted" }))),
        Err(err_msg) => {
            eprintln!("Error deleting recipe: {err_msg}");
            (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({ "error": err_msg })))
        }
    }
}

pub async fn handle_update_recipe(
    Path(id): Path<u32>,
    Json(updated_recipe): Json<Recipe>
) -> impl IntoResponse {
    match manager::update_recipe(id, updated_recipe) {
        Ok(_) => (StatusCode::OK, Json(serde_json::json!({ "status": "updated" }))),
        Err(err_msg) => {
            eprintln!("Error updating recipe: {err_msg}");
            (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({ "error": err_msg })))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use http_body_util::BodyExt;
    use std::fs;
    use serde_json::from_slice;

    #[tokio::test]
    async fn test_handle_index() {
        let expected_html = fs::read_to_string("html/index.html").unwrap();
        let response = handle_index().await.into_response();
        let body_bytes = response.into_body().collect().await.unwrap().to_bytes();
        let body_str = String::from_utf8(body_bytes.to_vec()).unwrap();
        assert_eq!(body_str, expected_html);
    }

    #[tokio::test]
    async fn test_handle_new_recipe_page() {
        let expected_html = fs::read_to_string("html/add-recipe.html").unwrap();
        let response = handle_new_recipe_page().await.into_response();
        let body_bytes = response.into_body().collect().await.unwrap().to_bytes();
        let body_str = String::from_utf8(body_bytes.to_vec()).unwrap();
        assert_eq!(body_str, expected_html);
    }

    #[tokio::test]
    async fn test_handle_all_recipes() {
        let response = handle_all_recipes().await.into_response();
        let body_bytes = response.into_body().collect().await.unwrap().to_bytes();
        let recipes: Vec<Recipe> = from_slice(&body_bytes).unwrap();
        assert!(!recipes.is_empty());
    }

    #[tokio::test]
    async fn test_handle_recipe_with_id_0() {
        let response = handle_recipe(Path(0)).await.into_response();
        assert_eq!(response.status(), StatusCode::OK);
        let body_bytes = response.into_body().collect().await.unwrap().to_bytes();
        let recipe: Recipe = from_slice(&body_bytes).unwrap();
        assert_ne!(recipe, Recipe::default());
    }
    
    #[tokio::test]
    async fn test_handle_recipe_with_invalid_id() {
        let response = handle_recipe(Path(999_999)).await.into_response();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }
}