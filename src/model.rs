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
    /// SQLite AUTOINCREMENT returns i64; using i64 throughout avoids casting.
    #[serde(default)]
    pub id: i64,
    pub name: String,
    /// Link to the website where this recipe was originally found.
    /// Not all recipes have a source, so this field is optional.
    pub source_url: Option<String>,
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

impl std::fmt::Display for MealSlot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MealSlot::Breakfast => write!(f, "breakfast"),
            MealSlot::Lunch    => write!(f, "lunch"),
            MealSlot::Dinner   => write!(f, "dinner"),
        }
    }
}

/// A planned meal: a recipe assigned to a specific date and slot.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct MealEntry {
    /// Date serialized as "YYYY-MM-DD" in JSON.
    pub date: NaiveDate,
    pub slot: MealSlot,
    pub recipe_id: i64,
}

/// A record of a recipe that was actually cooked on a given date.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct CookedEntry {
    /// Date serialized as "YYYY-MM-DD" in JSON.
    pub date: NaiveDate,
    pub recipe_id: i64,
}

/// A registered user.
///
/// `password_hash` is never serialized — it must never appear in an API
/// response. The field is intentionally excluded from `Serialize`.
#[derive(Deserialize, Debug, Clone)]
pub struct User {
    pub id: i64,
    pub username: String,
    pub password_hash: String,
}

/// The form payload submitted on the login page.
#[derive(Deserialize, Debug)]
pub struct LoginForm {
    pub username: String,
    pub password: String,
}