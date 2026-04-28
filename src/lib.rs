/// Contains models used within the app
pub mod model;

/// Authentication utilities: password hashing and the AuthUser session extractor.
pub mod auth;

/// network module meant to handle network related commands,
/// including connection handling and response to requests
pub mod network;

/// Storage layer: all SQL queries for recipes, users, and schema migrations.
/// No other module should reference table names or SQL directly — when the
/// backend changes (e.g. SQLite → Postgres), only this file changes.
pub mod storage;

/// calendar_storage module meant to handle interactions with the meal plan
/// and cooked log databases.
pub mod calendar_storage;

/// Business logic layer: input validation, per-user quota enforcement, password
/// hashing, and user management. All manager functions validate their inputs and
/// enforce quotas before delegating persistence to the storage layer.
pub mod manager;

/// calendar_manager meant to handle meal planning, cooked log, and shopping list logic.
pub mod calendar_manager;

/// Rate limiting utilities: session-user-id injection middleware and user-keyed extractor.
pub mod rate_limit;

/// CSRF protection middleware: Origin-check for state-mutating requests.
pub mod csrf;
