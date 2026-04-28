use serde::{Deserialize, Serialize};
use chrono::NaiveDate;
use validator::Validate;

/// A single ingredient in a recipe, combining a name with a measured quantity and unit.
///
/// Validated by the `validator` crate: `name` must be 1–100 characters, `quantity`
/// must be finite and non-negative, and `unit` must be at most 32 characters.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[derive(Validate)]
pub struct Ingredient {
    /// Display name of the ingredient (e.g. `"flour"`, `"unsalted butter"`).
    /// Must be between 1 and 100 characters.
    #[validate(length(min = 1, max = 100))]
    pub name: String,
    /// How much of the ingredient is required, expressed in `unit`.
    /// Must be finite and non-negative (zero is allowed for trace amounts).
    #[validate(custom(function = "is_finite_positive"))]
    pub quantity: f32,
    /// The unit of measurement (e.g. `"g"`, `"ml"`, `"cup"`, `"clove"`).
    /// An empty string is accepted for unitless count-based ingredients.
    /// Must be at most 32 characters.
    #[validate(length(max = 32))]
    pub unit: String,
}

/// The core domain type representing a saved recipe.
///
/// Ingredients and instructions are stored as JSON arrays in the `recipes` SQLite
/// table and deserialised into `Vec<Ingredient>` / `Vec<String>` on load. Validation
/// constraints are applied by the `validator` crate and enforced in the manager layer
/// before any storage call.
#[derive(Serialize, Deserialize, Debug, Clone, Default, PartialEq)]
#[derive(Validate)]
pub struct Recipe {
    /// Database primary key assigned by SQLite `AUTOINCREMENT`.
    /// Use `0` (the `Default`) when constructing a new recipe for insertion;
    /// the value is replaced by the real ID after `add_recipe` returns.
    /// `i64` is used throughout to match the type returned by `sqlx`.
    #[serde(default)]
    pub id: i64,
    /// Human-readable name for the recipe (e.g. `"Sourdough Bread"`).
    /// Must be between 1 and 200 characters.
    #[validate(length(min = 1, max = 200))]
    pub name: String,
    /// Link to the website where this recipe was originally found.
    /// Not all recipes have a source, so this field is optional.
    #[validate(url, length(max = 500))]
    pub source_url: Option<String>,
    /// Ordered list of ingredients. At most 50 ingredients are allowed per recipe.
    #[validate(length(max = 50), nested)]
    pub ingredients: Vec<Ingredient>,
    /// Ordered list of preparation steps. At most 100 steps are allowed per recipe.
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
    pub is_admin: bool,
}

/// A user record safe to return over the API — no password hash.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct UserInfo {
    pub id: i64,
    pub username: String,
    pub is_admin: bool,
    pub created_at: String,
}

/// The form payload submitted on the login page.
#[derive(Deserialize, Debug)]
pub struct LoginForm {
    pub username: String,
    pub password: String,
}

/// Admin form payload for creating a new user account.
#[derive(Deserialize, Debug)]
pub struct CreateUserForm {
    pub username: String,
    pub password: String,
}

/// Admin form payload for changing a user's password.
#[derive(Deserialize, Debug)]
pub struct ChangePasswordForm {
    pub target_user_id: i64,
    pub new_password: String,
}

/// Self-service form payload for changing the authenticated user's own password.
#[derive(Deserialize, Debug)]
pub struct SelfChangePasswordForm {
    pub current_password: String,
    pub new_password: String,
}

fn is_finite_positive(val: f32) -> Result<(), validator::ValidationError> {
    if val.is_finite() && val >= 0.0 {
        Ok(())
    } else {
        Err(validator::ValidationError::new("quantity must be finite and non-negative"))
    }
}