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

/// Rate limiting utilities: session-user-id injection middleware and user-keyed extractor.
pub mod rate_limit;

/// CSRF protection middleware: Origin-check for state-mutating requests.
pub mod csrf;
