use crate::model::Recipe;
use crate::storage;

/// Retrieves a recipe by its ID.
///
/// # Returns
///
/// Returns `Some(Recipe)` if found, `None` if the ID does not exist or
/// storage cannot be read.
pub fn get_recipe_by_id(id: u32) -> Option<Recipe> {
    storage::load_recipe(id).ok()
}

/// Adds a new recipe to storage.
///
/// # Returns
///
/// Returns `Ok(())` on success. The ID assigned to the recipe is managed
/// by the storage layer.
///
/// # Errors
///
/// Returns `Err` if the recipe could not be persisted.
pub fn add_recipe(recipe: Recipe) -> Result<(), String> {
    storage::add_recipe(&recipe)?;
    Ok(())
}

/// Deletes the recipe at the given ID.
///
/// # Errors
///
/// Returns `Err` if no recipe exists at `id` or storage cannot be written.
pub fn delete_recipe(id: u32) -> Result<(), String> {
    storage::delete_recipe(id)
}

/// Returns all recipes from storage.
///
/// Returns an empty Vec if storage cannot be read.
pub fn get_all_recipes() -> Vec<Recipe> {
    storage::load_all_recipes().unwrap_or_default()
}

/// Updates an existing recipe by ID.
///
/// # Errors
///
/// Returns `Err` if no recipe exists at `id` or storage cannot be written.
pub fn update_recipe(id: u32, recipe: Recipe) -> Result<(), String> {
    storage::save_recipe(id, &recipe)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_get_recipe_by_id_valid() {
        let recipe = get_recipe_by_id(0);
        assert!(recipe.is_some(), "Recipe should be found for ID 0");
    }

    #[test]
    fn test_get_recipe_by_id_invalid() {
        let recipe = get_recipe_by_id(999_999);
        assert!(recipe.is_none(), "Recipe should not be found for a missing ID");
    }

    #[test]
    fn test_get_all_recipes_returns_vec() {
        let recipes = get_all_recipes();
        assert!(!recipes.is_empty(), "Expected at least one recipe");
    }
}