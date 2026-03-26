use serde::{Deserialize, Serialize};
use chrono::NaiveDate;

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

/// Represents a meal slot within a day.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum MealSlot {
    Breakfast,
    Lunch,
    Dinner,
}

/// A planned meal: a recipe assigned to a specific date and slot.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct MealEntry {
    /// Date serialized as "YYYY-MM-DD" in JSON.
    pub date: NaiveDate,
    pub slot: MealSlot,
    pub recipe_id: u32,
}

/// A record of a recipe that was actually cooked on a given date.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct CookedEntry {
    /// Date serialized as "YYYY-MM-DD" in JSON.
    pub date: NaiveDate,
    pub recipe_id: u32,
}