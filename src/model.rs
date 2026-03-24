use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct Ingredient {
    pub name: String,
    pub quantity: f32,
    pub unit: String,
}

#[derive(Serialize, Deserialize, Debug, Clone, Default, PartialEq)]
pub struct Recipe {
    #[serde(default)]
    pub id: u32,
    pub name: String,
    pub picture: String,
    pub ingredients: Vec<Ingredient>,
    pub instructions: Vec<String>,
}