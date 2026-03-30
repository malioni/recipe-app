/// Contains models used within the app
pub mod model;

/// Authentication utilities: password hashing and the AuthUser session extractor.
pub mod auth;

/// network module meant to handle network related commands,
/// including connection handling and response to requests
pub mod network;

/// storage module meant to handle interactions with the recipe database.
pub mod storage;

/// calendar_storage module meant to handle interactions with the meal plan
/// and cooked log databases.
pub mod calendar_storage;

/// manager meant to handle page state and recipes indices
pub mod manager;

/// calendar_manager meant to handle meal planning, cooked log, and shopping list logic.
pub mod calendar_manager;

/// The ID of the single placeholder user.
/// All data is owned by this user until real authentication is fully wired up.
/// When auth is complete, remove this constant and pass `auth.user_id` from
/// the `AuthUser` extractor through the manager functions instead.
pub const SINGLE_USER_ID: i64 = 1;