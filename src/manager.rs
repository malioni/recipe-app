use crate::storage::read_from_file;
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Serialize, Deserialize, Debug, Clone)]
struct Ingredient {
    name: String,
    quantity: u32,
    unit: String,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
struct Recipe {
    name: String,
    picture: String,
    ingredients: Vec<Ingredient>,
    instructions: Vec<String>,
}

/// Retrieves a recipe by its ID from the database.
///
/// # Arguments
///
/// * `id` - The ID of the recipe as stored in the database. Used to retrieve the next or the
/// previous recipe.
///
/// # Returns
/// 
/// Returns `Some(String)` if the recipe is found and serialized to a JSON string.
/// Returns `None` if the recipe was not found or if there was an error during the process.
///
pub fn get_recipe_by_id(id: u32) -> Option<String> {
    // Read the recipes from a file
    let directory = "db";
    let filename = "recipes.json";
    let path = Path::new(directory).join(filename);
    read_from_file(&path)
        .ok()
        .and_then(|data| {
            // Deserialize the JSON data into a vector of Recipe
            serde_json::from_str::<Vec<Recipe>>(&data).ok()
        })
        .and_then(|recipes| {
            // Find the recipe with the given ID
            recipes.get(id as usize).cloned()
        }).and_then(|recipe| {
            // Serialize the found recipe back to a JSON string
            serde_json::to_string(&recipe).ok()
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_get_recipe_by_id() {
        // Test with a valid ID
        let recipe = get_recipe_by_id(0);
        assert!(recipe.is_some(), "Recipe should be found for ID 0");

        // Test with an invalid ID
        let recipe = get_recipe_by_id(999999);
        assert!(recipe.is_none(), "Recipe should not be found for ID 999999");
    }
}


