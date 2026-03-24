/// Storage module responsible for all recipe persistence.
///
/// This module owns the storage backend entirely. No other module should know
/// about file paths, serialization format, or how recipes are physically stored.
/// When the backend changes (e.g. file → SQLite → Postgres), only this file changes.
use std::fs;
use std::path::Path;
use crate::model::Recipe;

const DB_PATH: &str = "db/recipes.json";

/// Loads a single recipe by its ID.
///
/// Prefer this over `load_all_recipes` when only one recipe is needed,
/// as future backends (e.g. a database) can satisfy this with a single
/// indexed lookup rather than a full scan.
///
/// # Errors
///
/// Returns `Err` if the storage cannot be read, the contents cannot be
/// parsed, or no recipe exists at the given ID.
pub fn load_recipe(id: u32) -> Result<Recipe, String> {
    // File backend: still loads the full file for now.
    // A database backend would issue: SELECT * FROM recipes WHERE id = ?
    load_all_recipes()?
        .into_iter()
        .find(|r| r.id == id)
        .ok_or_else(|| format!("Recipe with ID {} not found", id))
}

/// Persists a single recipe, replacing the existing entry at `id`.
///
/// # Errors
///
/// Returns `Err` if the storage cannot be read or written, the contents
/// cannot be parsed, or no recipe exists at the given ID.
pub fn save_recipe(id: u32, recipe: &Recipe) -> Result<(), String> {
    let mut recipes = load_all_recipes()?;
    let slot = recipes.iter_mut()
        .find(|r| r.id == id)
        .ok_or_else(|| format!("Recipe with ID {} not found", id))?;
    *slot = recipe.clone();
    write_all(&recipes)
}

/// Loads every recipe from storage.
///
/// Use this only when the full list is genuinely needed (e.g. listing all
/// recipes in the UI). Prefer `load_recipe` for single-item access.
///
/// # Errors
///
/// Returns `Err` if the storage cannot be read or the contents cannot be
/// parsed.
pub fn load_all_recipes() -> Result<Vec<Recipe>, String> {
    let path = Path::new(DB_PATH);
    let data = fs::read_to_string(path)
        .map_err(|e| format!("Failed to read storage: {e}"))?;
    serde_json::from_str(&data)
        .map_err(|e| format!("Failed to parse storage: {e}"))
}

/// Appends a new recipe to storage, assigning it the next available ID.
///
/// Returns the ID assigned to the new recipe.
///
/// # Errors
///
/// Returns `Err` if the storage cannot be read or written.
pub fn add_recipe(recipe: &Recipe) -> Result<u32, String> {
    // File backend: load → push → write.
    // A database backend would issue: INSERT INTO recipes (...) RETURNING id
    let mut recipes = load_all_recipes().unwrap_or_default();
    let new_id = recipes.iter().map(|r| r.id).max().map_or(0, |m| m + 1);
    let mut recipe = recipe.clone();
    recipe.id = new_id;
    recipes.push(recipe);
    write_all(&recipes)?;
    Ok(new_id)
}

/// Deletes the recipe at `id`, shifting subsequent IDs down by one.
///
/// Note: ID shifting is a property of the current file backend. A database
/// backend would use stable surrogate keys and this side-effect would
/// disappear.
///
/// # Errors
///
/// Returns `Err` if the storage cannot be read or written, or no recipe
/// exists at the given ID.
pub fn delete_recipe(id: u32) -> Result<(), String> {
    // File backend: load → remove → write.
    // A database backend would issue: DELETE FROM recipes WHERE id = ?
    let mut recipes = load_all_recipes()?;
    let pos = recipes.iter().position(|r| r.id == id)
        .ok_or_else(|| format!("Recipe with ID {} not found", id))?;
    recipes.remove(pos);
    write_all(&recipes)
}

// ---------------------------------------------------------------------------
// Private helpers — nothing above this line should know these exist.
// ---------------------------------------------------------------------------

/// Serializes `recipes` and writes the result to the backing file.
/// Extracted so every mutating function shares one write path.
fn write_all(recipes: &[Recipe]) -> Result<(), String> {
    let json = serde_json::to_string_pretty(recipes)
        .map_err(|e| format!("Serialization failed: {e}"))?;
    fs::write(DB_PATH, json)
        .map_err(|e| format!("Failed to write storage: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_load_recipe_valid_id() {
        let recipe = load_recipe(0);
        assert!(recipe.is_ok(), "Expected a recipe at ID 0");
    }

    #[test]
    fn test_load_recipe_invalid_id() {
        let recipe = load_recipe(999_999);
        assert!(recipe.is_err(), "Expected an error for a missing ID");
    }

    #[test]
    fn test_load_all_recipes_returns_vec() {
        let recipes = load_all_recipes();
        assert!(recipes.is_ok(), "Expected storage to be readable");
    }
}