use axum::{
   response::{IntoResponse, Html},
   extract::Path,
   Json
};
use crate::manager;

pub async fn handle_index() -> impl IntoResponse {
    // Serve the html page
    Html(std::fs::read_to_string("html/recipe-page.html").unwrap_or_else(|_| "<h1>Error</h1>".to_string()))
}

pub async fn handle_recipe(Path(id): Path<u32>) -> Json<manager::Recipe> {
    Json(manager::get_recipe_by_id(id).unwrap_or(manager::Recipe::default()))
}


#[cfg(test)]
mod tests {
    use super::*;
    use http_body_util::BodyExt;
    use std::fs;
    use serde_json::from_slice;

    #[tokio::test]
    async fn test_handle_index() {
        // Arrange: read the expected file contents
        let expected_html = fs::read_to_string("html/recipe-page.html").unwrap();

        // Act: call your handler
        let response = handle_index().await.into_response();

        // Assert: extract body into a string
        let body_bytes = response
            .into_body()
            .collect()
            .await
            .unwrap()
            .to_bytes();
        let body_str = String::from_utf8(body_bytes.to_vec()).unwrap();

        assert_eq!(body_str, expected_html);
    }

    #[tokio::test]
    async fn test_handle_recipe_with_id_0() {
        // Act: call the handler directly
        let response = handle_recipe(Path(0)).await.into_response();

        // Extract body
        let body_bytes = response.into_body().collect().await.unwrap().to_bytes();

        let recipe: manager::Recipe = from_slice(&body_bytes).unwrap();

        // Assert: a recipe is returned
        assert_ne!(recipe, manager::Recipe::default());
    }
}
