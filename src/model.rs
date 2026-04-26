use serde::{Deserialize, Serialize};
use chrono::NaiveDate;
use validator::Validate;

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[derive(Validate)]
pub struct Ingredient {
    #[validate(length(min = 1, max = 100))]
    pub name: String,
    #[validate(custom(function = "is_finite_positive"))]
    pub quantity: f32,
    #[validate(length(max = 32))]
    pub unit: String,
}

#[derive(Serialize, Deserialize, Debug, Clone, Default, PartialEq)]
#[derive(Validate)]
pub struct Recipe {
    /// SQLite AUTOINCREMENT returns i64; using i64 throughout avoids casting.
    #[serde(default)]
    pub id: i64,
    #[validate(length(min = 1, max = 200))]
    pub name: String,
    /// Link to the website where this recipe was originally found.
    /// Not all recipes have a source, so this field is optional.
    #[validate(url, length(max = 500))]
    pub source_url: Option<String>,
    #[validate(length(max = 50), nested)]
    pub ingredients: Vec<Ingredient>,
    #[validate(length(max = 100))]
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
    /// Database primary key. `None` when constructing a new entry for
    /// insertion; `Some(id)` on all entries returned from the database.
    /// Omitted from serialized output when `None` so POST bodies need not
    /// include it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<i64>,
    /// Date serialized as "YYYY-MM-DD" in JSON.
    pub date: NaiveDate,
    pub slot: MealSlot,
    pub recipe_id: i64,
    /// Number of times to multiply ingredient quantities for this entry.
    /// Defaults to 1 when omitted from the request body.
    #[serde(default = "default_one")]
    pub portions: i64,
}

fn default_one() -> i64 { 1 }

/// A shopping list entry with metric and optional imperial display quantities.
///
/// Aggregation happens internally in `g`/`ml`; this struct carries the
/// already-rounded display values so the caller never needs to know the
/// internal canonical unit.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct ShoppingListItem {
    pub name: String,
    /// Quantity in `metric_unit` (already ceiled to the display step).
    pub metric_quantity: f32,
    /// `"g"` or `"kg"` for weight; `"ml"` or `"l"` for volume; original unit
    /// string for count-based / unrecognised units.
    pub metric_unit: String,
    /// Quantity in `imperial_unit` (ceiled to nearest whole unit).
    /// `None` for count-based or unrecognised units.
    pub imperial_quantity: Option<f32>,
    /// `"oz"` for weight, `"fl oz"` for volume, `None` otherwise.
    pub imperial_unit: Option<String>,
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

fn is_finite_positive(val: f32) -> Result<(), validator::ValidationError> {
    if val.is_finite() && val >= 0.0 {
        Ok(())
    } else {
        Err(validator::ValidationError::new("quantity must be finite and non-negative"))
    }
}