use axum::{
    response::{IntoResponse, Html},
    extract::{Path, Json, Query, State},
    http::StatusCode,
};
use chrono::NaiveDate;
use serde::Deserialize;
use sqlx::SqlitePool;
use crate::manager;
use crate::calendar_manager;
use crate::model::{MealEntry, CookedEntry, MealSlot, Recipe};

// ---------------------------------------------------------------------------
// Recipes
// ---------------------------------------------------------------------------

pub async fn handle_index() -> impl IntoResponse {
    Html(std::fs::read_to_string("html/index.html").unwrap_or_else(|_| "<h1>Error</h1>".to_string()))
}

pub async fn handle_all_recipes(
    State(pool): State<SqlitePool>,
) -> Json<Vec<Recipe>> {
    Json(manager::get_all_recipes(&pool).await)
}

pub async fn handle_recipe(
    State(pool): State<SqlitePool>,
    Path(id): Path<i64>,
) -> impl IntoResponse {
    match manager::get_recipe_by_id(&pool, id).await {
        Some(recipe) => (StatusCode::OK, Json(recipe)).into_response(),
        None => (StatusCode::NOT_FOUND, Json(serde_json::json!({ "error": format!("Recipe with ID {} not found", id) }))).into_response(),
    }
}

pub async fn handle_new_recipe_page() -> impl IntoResponse {
    Html(std::fs::read_to_string("html/add-recipe.html").unwrap_or_else(|_| "<h1>Error</h1>".to_string()))
}

pub async fn handle_add_recipe(
    State(pool): State<SqlitePool>,
    Json(new_recipe): Json<Recipe>,
) -> impl IntoResponse {
    match manager::add_recipe(&pool, new_recipe).await {
        Ok(_) => (StatusCode::CREATED, Json(serde_json::json!({ "status": "created" }))),
        Err(err_msg) => {
            eprintln!("Error saving recipe: {err_msg}");
            (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({ "error": err_msg })))
        }
    }
}

pub async fn handle_delete_recipe(
    State(pool): State<SqlitePool>,
    Path(id): Path<i64>,
) -> impl IntoResponse {
    match manager::delete_recipe(&pool, id).await {
        Ok(_) => (StatusCode::OK, Json(serde_json::json!({ "status": "deleted" }))),
        Err(err_msg) => {
            eprintln!("Error deleting recipe: {err_msg}");
            (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({ "error": err_msg })))
        }
    }
}

pub async fn handle_update_recipe(
    State(pool): State<SqlitePool>,
    Path(id): Path<i64>,
    Json(updated_recipe): Json<Recipe>,
) -> impl IntoResponse {
    match manager::update_recipe(&pool, id, updated_recipe).await {
        Ok(_) => (StatusCode::OK, Json(serde_json::json!({ "status": "updated" }))),
        Err(err_msg) => {
            eprintln!("Error updating recipe: {err_msg}");
            (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({ "error": err_msg })))
        }
    }
}

// ---------------------------------------------------------------------------
// Calendar — shared query parameter structs
// ---------------------------------------------------------------------------

/// Query params for endpoints that accept a date range: `?start=YYYY-MM-DD&end=YYYY-MM-DD`
#[derive(Deserialize)]
pub struct DateRangeParams {
    pub start: NaiveDate,
    pub end: NaiveDate,
}

/// Query params for deleting a planned meal: `?date=YYYY-MM-DD&slot=breakfast`
#[derive(Deserialize)]
pub struct DeleteMealParams {
    pub date: NaiveDate,
    pub slot: MealSlot,
}

// ---------------------------------------------------------------------------
// Calendar — page
// ---------------------------------------------------------------------------

/// GET /calendar
pub async fn handle_calendar_page() -> impl IntoResponse {
    Html(std::fs::read_to_string("html/calendar.html").unwrap_or_else(|_| "<h1>Error</h1>".to_string()))
}

// ---------------------------------------------------------------------------
// Calendar — meal plan
// ---------------------------------------------------------------------------

/// GET /calendar/entries?start=YYYY-MM-DD&end=YYYY-MM-DD
pub async fn handle_get_meal_entries(
    State(pool): State<SqlitePool>,
    Query(params): Query<DateRangeParams>,
) -> impl IntoResponse {
    match calendar_manager::get_meals_in_range(&pool, params.start, params.end).await {
        Ok(entries) => (StatusCode::OK, Json(entries)).into_response(),
        Err(err_msg) => {
            eprintln!("Error fetching meal entries: {err_msg}");
            (StatusCode::BAD_REQUEST, Json(serde_json::json!({ "error": err_msg }))).into_response()
        }
    }
}

/// POST /calendar/entries
/// Body: `{ "date": "YYYY-MM-DD", "slot": "breakfast", "recipe_id": 1 }`
pub async fn handle_plan_meal(
    State(pool): State<SqlitePool>,
    Json(entry): Json<MealEntry>,
) -> impl IntoResponse {
    match calendar_manager::plan_meal(&pool, entry.date, entry.slot, entry.recipe_id).await {
        Ok(_) => (StatusCode::CREATED, Json(serde_json::json!({ "status": "planned" }))),
        Err(err_msg) => {
            eprintln!("Error planning meal: {err_msg}");
            (StatusCode::BAD_REQUEST, Json(serde_json::json!({ "error": err_msg })))
        }
    }
}

/// DELETE /calendar/entries?date=YYYY-MM-DD&slot=breakfast
pub async fn handle_delete_meal_entry(
    State(pool): State<SqlitePool>,
    Query(params): Query<DeleteMealParams>,
) -> impl IntoResponse {
    match calendar_manager::remove_planned_meal(&pool, params.date, params.slot).await {
        Ok(_) => (StatusCode::OK, Json(serde_json::json!({ "status": "deleted" }))),
        Err(err_msg) => {
            eprintln!("Error deleting meal entry: {err_msg}");
            (StatusCode::NOT_FOUND, Json(serde_json::json!({ "error": err_msg })))
        }
    }
}

// ---------------------------------------------------------------------------
// Calendar — cooked log
// ---------------------------------------------------------------------------

/// POST /calendar/cooked
/// Body: `{ "date": "YYYY-MM-DD", "recipe_id": 1 }`
pub async fn handle_mark_cooked(
    State(pool): State<SqlitePool>,
    Json(entry): Json<CookedEntry>,
) -> impl IntoResponse {
    match calendar_manager::mark_as_cooked(&pool, entry.date, entry.recipe_id).await {
        Ok(_) => (StatusCode::CREATED, Json(serde_json::json!({ "status": "logged" }))),
        Err(err_msg) => {
            eprintln!("Error logging cooked entry: {err_msg}");
            (StatusCode::BAD_REQUEST, Json(serde_json::json!({ "error": err_msg })))
        }
    }
}

/// GET /calendar/cooked?start=YYYY-MM-DD&end=YYYY-MM-DD
pub async fn handle_get_cooked_entries(
    State(pool): State<SqlitePool>,
    Query(params): Query<DateRangeParams>,
) -> impl IntoResponse {
    match calendar_manager::get_cooked_in_range(&pool, params.start, params.end).await {
        Ok(entries) => (StatusCode::OK, Json(entries)).into_response(),
        Err(err_msg) => {
            eprintln!("Error fetching cooked entries: {err_msg}");
            (StatusCode::BAD_REQUEST, Json(serde_json::json!({ "error": err_msg }))).into_response()
        }
    }
}

// ---------------------------------------------------------------------------
// Calendar — shopping list
// ---------------------------------------------------------------------------

/// GET /calendar/shopping-list?start=YYYY-MM-DD&end=YYYY-MM-DD
pub async fn handle_shopping_list(
    State(pool): State<SqlitePool>,
    Query(params): Query<DateRangeParams>,
) -> impl IntoResponse {
    match calendar_manager::get_shopping_list(&pool, params.start, params.end).await {
        Ok(ingredients) => (StatusCode::OK, Json(ingredients)).into_response(),
        Err(err_msg) => {
            eprintln!("Error generating shopping list: {err_msg}");
            (StatusCode::BAD_REQUEST, Json(serde_json::json!({ "error": err_msg }))).into_response()
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use axum::extract::State;
    use http_body_util::BodyExt;
    use std::fs;
    use serde_json::from_slice;

    async fn setup() -> SqlitePool {
        let pool = SqlitePool::connect(":memory:")
            .await
            .expect("Failed to create in-memory database");
        sqlx::query(include_str!("../migrations/001_initial.sql"))
            .execute(&pool)
            .await
            .expect("Failed to run migrations");
        sqlx::query("INSERT INTO users (id, username, password_hash) VALUES (1, 'test', 'placeholder')")
            .execute(&pool)
            .await
            .expect("Failed to insert test user");
        pool
    }

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
    async fn test_handle_all_recipes_empty() {
        let pool = setup().await;
        let response = handle_all_recipes(State(pool)).await.into_response();
        let body_bytes = response.into_body().collect().await.unwrap().to_bytes();
        let recipes: Vec<Recipe> = from_slice(&body_bytes).unwrap();
        assert!(recipes.is_empty());
    }

    #[tokio::test]
    async fn test_handle_recipe_with_invalid_id() {
        let pool = setup().await;
        let response = handle_recipe(State(pool), Path(999_999)).await.into_response();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn test_handle_get_meal_entries_valid_range() {
        let pool = setup().await;
        let params = DateRangeParams {
            start: NaiveDate::from_ymd_opt(2026, 1, 1).unwrap(),
            end: NaiveDate::from_ymd_opt(2026, 1, 7).unwrap(),
        };
        let response = handle_get_meal_entries(State(pool), Query(params)).await.into_response();
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_handle_get_meal_entries_invalid_range() {
        let pool = setup().await;
        let params = DateRangeParams {
            start: NaiveDate::from_ymd_opt(2026, 1, 7).unwrap(),
            end: NaiveDate::from_ymd_opt(2026, 1, 1).unwrap(),
        };
        let response = handle_get_meal_entries(State(pool), Query(params)).await.into_response();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn test_handle_shopping_list_empty_range() {
        let pool = setup().await;
        let params = DateRangeParams {
            start: NaiveDate::from_ymd_opt(2099, 1, 1).unwrap(),
            end: NaiveDate::from_ymd_opt(2099, 1, 7).unwrap(),
        };
        let response = handle_shopping_list(State(pool), Query(params)).await.into_response();
        assert_eq!(response.status(), StatusCode::OK);
        let body_bytes = response.into_body().collect().await.unwrap().to_bytes();
        let ingredients: Vec<serde_json::Value> = from_slice(&body_bytes).unwrap();
        assert!(ingredients.is_empty());
    }
}