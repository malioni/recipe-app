# Recipe App — Action Items & Technical Roadmap

> Generated from code review session. Items are ordered by implementation priority.
> Check off items as they are completed.

---

## Project Context & Security Notes

This is a multi-user recipe and meal planning application, intended to run on a home network at minimum and potentially be published to the web. The following threat model should be kept in mind when making architectural decisions:

- **Authentication is required** before the app leaves localhost. A single Axum middleware handles this for all routes.
- **Resource abuse is a real risk** for any multi-user app. Bad actors may attempt to use the app as general-purpose storage. Mitigations are layered across the stack: request body size limits (framework layer), field length and count limits (validation layer), per-user quotas (manager layer), and rate limiting (middleware layer).
- **The `picture` field has been removed** from the `Recipe` model (see item 18). Storing image data or large base64 strings in that field was the primary storage abuse vector. It has been replaced with an optional `source_url` field.
- **Image uploads are explicitly out of scope** until a proper object storage strategy (disk or S3/R2) with per-user byte quotas is designed. Do not add image upload functionality without addressing storage limits first.
- **SQLite size management:** SQLite has no native file size cap. Size is managed through application-level controls (field length limits, per-user record quotas, purging old calendar entries) rather than raw file size limits. `PRAGMA auto_vacuum = INCREMENTAL` should be enabled to reclaim space from deleted rows on a schedule.

---

## Phase 1 — Do Now (Blockers / Gets Harder Later)

### 1. [x] Switch to SQLite (`sqlx`)

**Why now:** Eliminates race conditions, ID reuse bugs, and orphaned meal plan references. Auth (next item) depends on having a `users` table. Every week this waits, more code assumes the file-based shape.

**Actions:**

- Add `sqlx = { version = "0.7", features = ["sqlite", "runtime-tokio", "macros", "chrono"] }` to `Cargo.toml`
- Design schema with `users` table and `user_id` foreign keys on `recipes`, `meal_plan`, and `cooked_log` from the start — even if not enforced immediately
- Enable `PRAGMA journal_mode = WAL` for concurrent reads with serialized writes
- Enable `PRAGMA auto_vacuum = INCREMENTAL` to reclaim space from deleted rows
- Enable `PRAGMA foreign_keys = ON` to enforce referential integrity
- Replace internals of `storage.rs` and `calendar_storage.rs` only — keep the public interfaces identical so the rest of the codebase is unaffected
- Use the `query!` macro for compile-time query checking
- Commit `Cargo.lock` — treat it as the source of truth for reproducible builds

**Schema notes:**

- `recipes` table: `id`, `user_id`, `name`, `source_url` (nullable), `ingredients` (JSON or normalized), `instructions` (JSON or normalized)
- `meal_plan` table: `id`, `user_id`, `date`, `slot`, `recipe_id` (FK → recipes, `ON DELETE CASCADE`)
- `cooked_log` table: `id`, `user_id`, `date`, `recipe_id` (FK → recipes, `ON DELETE CASCADE`)
- `users` table: `id`, `username`, `password_hash`, `created_at`

---

### 2. [ ] Add Authentication

**Why now:** Auth is the most foundational "gets harder later" item once multi-user is the goal. One middleware added now protects all current and future routes automatically. Added after more routes exist, every handler becomes a risk of being missed.

**Actions:**

- Add dependencies: `argon2` (password hashing), `tower-sessions` + `tower-sessions-sqlx-store` (session persistence in SQLite — no separate Redis needed)
- Add a `users` table to the SQLite schema (from item 1)
- Implement an Axum middleware/extractor that validates the session on every request
- Implement login/logout routes and a basic login page
- Hash all passwords with `argon2` — never store plaintext

---

### 3. [ ] Add Request Body Size Limit Middleware

**Why now:** Without this, a single POST request with a multi-MB payload is buffered entirely in memory before any handler or validation runs. One line of middleware protects all current and future routes. Should be added at the same time as auth since both are framework-level protections.

**Actions:**

- Add `DefaultBodyLimit::max(1024 * 64)` (64KB) as a layer in `main.rs` — adjust if legitimate use cases require larger payloads
- This is the first line of defence against storage abuse and oversized request attacks

---

### 4. [ ] Fix XSS Vulnerabilities

**Why now:** Low risk on localhost, real attack surface once the app moves to a home network or the web.

**Actions:**

- In `calendar.html`: replace `innerHTML` assignments that include server data in `makeMealChip` and the shopping list renderer with `textContent` / `createElement` / `setAttribute`
- In `index.html`: audit the recipe grid rendering (`recipes.map(...)` template literal injected via `innerHTML`) for the same issue
- General rule going forward: never use `innerHTML` with any value that originates from user input or the server

---

### 5. [ ] Add `tracing` for Structured Logging

**Why now:** `eprintln!` disappears in any real deployment. Axum emits `tracing` spans natively so this is low effort for high debuggability gain. Establish the pattern before auth and more routes are added — structured logs are also important for detecting resource abuse (e.g. identifying which user is creating excessive records).

**Actions:**

- Add `tracing = "0.1"` and `tracing-subscriber = { version = "0.3", features = ["env-filter"] }` to `Cargo.toml`
- Initialize `tracing-subscriber` in `main.rs` (a single `tracing_subscriber::fmt::init()` call)
- Replace all `eprintln!` calls in `network.rs` with `tracing::error!` or `tracing::warn!`
- Add `tracing::info!` for key lifecycle events (server start, recipe added/deleted, meal planned)

---

### 6. [ ] Declare `pendingDate` / `pendingSlot` as Proper Variables in `calendar.html`

**Why now:** These are currently implicit globals. Breaks in strict mode and will cause silent bugs if a bundler or linter is ever added.

**Actions:**

- Add `let pendingDate = null;` and `let pendingSlot = null;` alongside the other state variable declarations at the top of the `<script>` block in `calendar.html`

---

### 7. [ ] Fix Timezone Bug in `toISO` (`calendar.html`)

**Why now:** Correctness bug that affects all users west of UTC. Can corrupt stored dates if not fixed before real data accumulates.

**Actions:**

- Replace `date.toISOString().slice(0, 10)` with a local-time formatter:
  ```javascript
  const toISO = (d) =>
    `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, "0")}-${String(
      d.getDate()
    ).padStart(2, "0")}`;
  ```

---

## Phase 2 — Soon (Correctness & Robustness)

### 8. [ ] Replace `picture` Field with `source_url` (see item 18 for full scope)

**Note:** This is the backend portion of item 18. Listed here as a reminder that model and storage changes are needed alongside the frontend changes.

---

### 9. [ ] Add Server-Side Input Validation

**Actions:**

- Add `validator = "0.18"` to `Cargo.toml`
- Derive validation rules on model structs:
  - Max string lengths on all name and text fields
  - `source_url` must be a valid URL if present
  - Ingredient quantity must be finite and non-negative (`f32`)
  - Max ingredient and step count per recipe
- Apply validation at the manager layer before any storage call
- Add per-user record quotas at the manager layer (e.g. max recipes per user, max meal plan entries retained) as the primary defence against storage abuse

---

### 10. [ ] Add Rate Limiting

**Actions:**

- Add `tower_governor` or `axum-ratelimit` to `Cargo.toml`
- Limit requests per authenticated user over a time window to prevent automated abuse within quotas
- Rate limiting by user ID is more meaningful than by IP once auth is in place

---

### 11. [ ] Serve `404.html` for Unmatched Routes

**Actions:**

- Add a fallback handler in `main.rs` using `.fallback(...)` that reads and returns `html/404.html` with a `404 Not Found` status

---

### 12. [ ] Fix HTML Handlers to Return Proper Error Status Codes

**Actions:**

- Replace `unwrap_or_else(|_| "<h1>Error</h1>".to_string())` in all HTML-serving handlers with a proper `500 Internal Server Error` response when the file cannot be read
- Consider reading HTML files once at startup and storing in `Arc<String>` to avoid a disk read on every request

---

### 13. [ ] Implement `Display` for `MealSlot`

**Actions:**

- Add `impl std::fmt::Display for MealSlot` producing lowercase strings (`"breakfast"`, `"lunch"`, `"dinner"`)
- Replace `{:?}` with `{}` in error messages across `calendar_storage.rs`

---

### 14. [ ] Cascade-Delete Meal Plan Entries When a Recipe Is Deleted

**Note:** If SQLite with foreign key constraints is implemented in item 1 with `ON DELETE CASCADE`, this is handled automatically at the database level and no separate application code is needed.

**Actions:**

- Verify `ON DELETE CASCADE` is in place on `meal_plan.recipe_id` and `cooked_log.recipe_id` after the SQLite migration
- If not using cascades, implement in `manager::delete_recipe` by calling into `calendar_storage` to remove referencing entries before deleting the recipe

---

## Phase 3 — Nice to Have (UX & Polish)

### 15. [ ] Add Loading States to the Calendar UI

**Actions:**

- Disable week navigation buttons during `loadWeek()`
- Show a spinner or skeleton state in the calendar grid while data is fetching

---

### 16. [ ] Invalidate `allRecipes` Cache in the Calendar Modal

**Actions:**

- Re-fetch the recipe list on each modal open, or add a visible refresh button in the modal header

---

### 17. [ ] Add a `Content-Security-Policy` Header

**Actions:**

- Add CSP as Axum middleware — defense-in-depth even after XSS fixes in item 4

---

### 18. [ ] Replace `picture` Field with `source_url` Across the Full Stack

**Context:** The `picture` field was designed for an image URL or path but is a storage abuse risk if it ever accepts raw image data. It has been replaced conceptually with `source_url` — an optional link to the website where the recipe was obtained (for attribution, photos, videos, or more detailed steps). Image uploads are explicitly out of scope until an object storage strategy with per-user byte quotas is in place.

**Actions:**

**Backend (`model.rs`, `storage.rs`, SQLite schema):**

- Rename `picture: String` to `source_url: Option<String>` on the `Recipe` struct
- Update the SQLite schema column accordingly
- `Option<String>` correctly represents "not all recipes have a source"

**Frontend (`index.html`):**

- Remove the recipe card image rendering (the `<img>` block in the card grid)
- Remove the recipe detail image (`<img id="recipe-image">`) from the recipe view
- Add a "Source" link in the recipe detail view that opens `source_url` in a new tab when present

**Frontend (`add-recipe.html`):**

- Remove the `picture` input if one exists (currently it's set silently to `""` in `submitRecipe()`)
- Add an optional "Source URL" text input field
- Validate on the client side that the value, if provided, looks like a URL

---

### 19. [ ] Align `handle_delete_recipe` with REST Conventions

**Actions:**

- Change `/recipes/:id/delete` from `POST` to `DELETE` to match the calendar API convention
- Update the frontend `deleteRecipe()` function to use `method: "DELETE"` and the direct `/recipes/:id` URL

---

### 20. [ ] Pin CDN Dependencies with Integrity Hashes

**Actions:**

- Add `integrity="sha384-..."` and `crossorigin="anonymous"` attributes to all Bootstrap `<link>` and `<script>` tags in `index.html`, `add-recipe.html`, and `calendar.html`

---

### 21. [ ] Fix Floating Point Display in Shopping List

**Actions:**

- Guard against floating point imprecision in `calendar.html`'s shopping list renderer (e.g. `0.30000000000000004 g`)
- Consider rounding to 2 decimal places for all non-integer quantities

---

## Dependency Reference

| Package                     | Current | Purpose             | Notes                                                                                  |
| --------------------------- | ------- | ------------------- | -------------------------------------------------------------------------------------- |
| `axum`                      | `0.7`   | Web framework       | Keep                                                                                   |
| `tokio`                     | `1.37`  | Async runtime       | Keep                                                                                   |
| `serde` / `serde_json`      | `1.0`   | Serialization       | Keep                                                                                   |
| `chrono`                    | `0.4`   | Date types          | Keep; watch for friction with `sqlx` — may switch to `time` crate                      |
| `sqlx`                      | —       | SQLite (planned)    | Add: `{ version = "0.7", features = ["sqlite", "runtime-tokio", "macros", "chrono"] }` |
| `argon2`                    | —       | Password hashing    | Add when implementing auth                                                             |
| `tower-sessions`            | —       | Session management  | Add when implementing auth                                                             |
| `tower-sessions-sqlx-store` | —       | Session persistence | Add when implementing auth                                                             |
| `validator`                 | —       | Input validation    | Add: `"0.18"`                                                                          |
| `tower_governor`            | —       | Rate limiting       | Add when implementing rate limiting (item 10)                                          |
| `tracing`                   | —       | Structured logging  | Add: `"0.1"`                                                                           |
| `tracing-subscriber`        | —       | Log formatting      | Add: `{ version = "0.3", features = ["env-filter"] }`                                  |

---

## Architecture Notes

- **Storage layer boundary:** `storage.rs` and `calendar_storage.rs` are the only files that should know about file paths or SQL queries. When migrating to SQLite, only these files change. The rest of the codebase is unaffected.
- **Auth middleware:** Should be implemented as a single Axum extractor/middleware, not per-handler checks. One middleware protects all routes automatically.
- **`user_id` on all domain tables:** Add to `recipes`, `meal_plan`, and `cooked_log` during the SQLite schema design — not as a migration after the fact.
- **Resource abuse defence is layered:** body size limit (framework) → field/count validation (manager) → per-user quotas (manager) → rate limiting (middleware) → monitoring via `tracing` logs.
- **Image uploads are out of scope** until an object storage strategy with per-user byte quotas is designed. The `source_url` field stores a link to an external source only.
- **SQLite → Postgres migration path:** If the app ever needs horizontal scaling or a hosted environment that doesn't persist local files, switching from SQLite to Postgres requires changing only the `sqlx` feature flag, the connection pool type, and the connection string. The rest of the codebase is unaffected by the storage layer boundary.
- **Commit `Cargo.lock`:** For a binary application, `Cargo.lock` should be committed and treated as the source of truth for reproducible builds.
