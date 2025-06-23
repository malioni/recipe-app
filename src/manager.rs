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
