# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Commands

```bash
# Build
cargo build

# Run (first boot requires INITIAL_USERNAME and INITIAL_PASSWORD in .env)
cargo run
RUST_LOG=debug cargo run   # with verbose logging

# Test all
cargo test

# Run a single test
cargo test test_handle_all_recipes_empty

# Lint
cargo clippy
```

On first boot, create a `.env` file with:
```
INITIAL_USERNAME=your_username
INITIAL_PASSWORD=your_password
```

The app listens on `http://127.0.0.1:7878`. The SQLite database is created at `db/recipe_app.db` on startup.

## Architecture

The codebase has a strict three-layer boundary:

```
network.rs / main.rs          — HTTP handlers and routing (thin; delegates to manager)
manager.rs / calendar_manager.rs  — Business logic, input validation, per-user quotas
storage.rs / calendar_storage.rs  — SQL queries only; no business logic
```

**Key rule:** `storage.rs` and `calendar_storage.rs` are the only files that may contain SQL. When the storage backend changes, only these files change.

### Authentication

`auth.rs` provides:
- `hash_password` / `verify_password` — argon2id wrappers
- `AuthUser` — an Axum extractor that reads the session and redirects to `/login` if unauthenticated

Add `_auth: AuthUser` as a handler parameter to protect any route. The extractor handles the redirect; individual handlers never check auth themselves.

### Multi-user state

`SINGLE_USER_ID` (defined in `lib.rs`) is a placeholder constant used by `manager.rs` and `calendar_manager.rs`. All domain tables (`recipes`, `meal_plan`, `cooked_log`) already have `user_id` columns. When full multi-user support is implemented (ActionItems.md item 26), replace `SINGLE_USER_ID` with `auth.user_id` from the `AuthUser` extractor.

### Frontend

Plain HTML/JS files in `html/`. Served by reading the file on each request via `tokio::fs::read_to_string`. Static assets (CSS, JS bundles) are in `static/` and served by `tower-http`'s `ServeDir`.

### Database

Single migration file: `migrations/001_initial.sql`. It is embedded at compile time via `include_str!` and run on every startup (idempotent — all tables use `CREATE TABLE IF NOT EXISTS`).

Tables: `users`, `recipes`, `meal_plan`, `cooked_log`. Ingredients and instructions are stored as JSON arrays in `recipes`. Sessions are stored in a `tower-sessions` SQLite table managed by `tower-sessions-sqlx-store`.

### Security layers (outermost to innermost)

1. Rate limiting — `tower_governor`, IP-based, 60 req/min burst
2. Body size limit — 64 KB max via `DefaultBodyLimit`
3. Session auth — `AuthUser` extractor on all non-login routes
4. Input validation — `validator` crate on model structs, applied in manager layer
5. Per-user quotas — enforced in manager layer (e.g. 500 recipes max)
6. CSP header — middleware in `main.rs` covering all responses

### Work guidelines (from README)

- Document all public components with Rust doc comments
- Tests are required for all public functions/interfaces
- Tests use an in-memory SQLite database (`:memory:`) set up via a `setup()` helper
- Image uploads are out of scope — `source_url` stores a link to an external source only
