# Recipe App — Action Items & Technical Roadmap

> Items are ordered by implementation priority.
> Check off items as they are completed.

---

## Project Context & Security Notes

This is a multi-user recipe and meal planning application, intended to run on a home network at minimum and potentially be published to the web. The target deployment is a **Raspberry Pi 3** on the home LAN (Raspberry Pi OS, `systemd`-managed), but all architectural decisions should preserve a clear upgrade path to public internet hosting. LAN-only solutions are acceptable only if they can be upgraded later without structural rework.

The following threat model should be kept in mind when making architectural decisions:

- **Authentication is required** before the app leaves localhost. A single Axum middleware handles this for all routes.
- **Resource abuse is a real risk** for any multi-user app. Bad actors may attempt to use the app as general-purpose storage. Mitigations are layered across the stack: request body size limits (framework layer), field length and count limits (validation layer), per-user quotas (manager layer), and rate limiting (middleware layer).
- **The `picture` field has been removed** from the `Recipe` model (see item 18). Storing image data or large base64 strings in that field was the primary storage abuse vector. It has been replaced with an optional `source_url` field.
- **Image uploads are explicitly out of scope** until a proper object storage strategy (disk or S3/R2) with per-user byte quotas is designed. Do not add image upload functionality without addressing storage limits first.
- **SQLite size management:** SQLite has no native file size cap. Size is managed through application-level controls (field length limits, per-user record quotas, purging old calendar entries) rather than raw file size limits. `PRAGMA auto_vacuum = INCREMENTAL` should be enabled to reclaim space from deleted rows on a schedule.
- **Raspberry Pi SD card wear:** SQLite writes (even with WAL) cause ongoing SD card wear. For a 24/7 Pi deployment, the database file and `.env` should live on a USB SSD rather than the SD card. The systemd `WorkingDirectory` and `DATABASE_URL` in `.env` should point to the USB mount path.

---

## Phase 4 — Deployment & Integrations

### 22. [ ] Raspberry Pi Deployment via `systemd`

**Context:** The app will run on a Raspberry Pi 3 on the home LAN, managed by `systemd` so it restarts automatically on reboot or crash.

**Actions:**

- Cross-compile for `aarch64-unknown-linux-gnu` on the dev machine, or compile directly on the Pi
- Create `/etc/systemd/system/recipe-app.service` with:
  - `After=network.target`
  - `EnvironmentFile=` pointing to the `.env` file (keep secrets out of the unit file)
  - `Restart=on-failure` and `RestartSec=5`
- Ensure `.env` is `chmod 600` and owned by the service user
- Store the SQLite database on a USB SSD, not the SD card — set `DATABASE_URL` in `.env` to the USB mount path
- Run `sudo systemctl enable recipe-app` to auto-start on boot
- Verify with `sudo systemctl status recipe-app` after reboot

**Notes:** This approach works as-is for LAN. If the app is later exposed to the internet, a reverse proxy (`caddy` or `nginx`) should sit in front of it to handle TLS. The systemd unit itself doesn't need to change.

---

### 23. [ ] TLS / HTTPS Strategy (Decide Before Going Public)

**Context:** Plain HTTP is acceptable on a trusted LAN. Before the app is accessible from the internet, TLS is required. This item is about deciding the approach — not necessarily implementing it immediately.

**Decision to make:** Choose one of:

- **A) Caddy reverse proxy** — `caddy` handles TLS termination (Let's Encrypt or local CA), proxies to the Axum app on localhost. Minimal Axum changes; Caddy manages cert renewal. Works for both LAN (self-signed) and public (ACME).
- **B) TLS in Axum directly** — `axum-server` with `rustls`. More control, no extra process, but cert renewal must be handled manually or via a sidecar.
- **C) Cloudflare Tunnel** — No port-forwarding, no public IP needed. Cloudflare terminates TLS. Works well for a home server going public without router changes.

**Recommendation:** Option A (Caddy) or C (Cloudflare Tunnel) are the most practical for a Pi home server going public. Decide before exposing to the internet.

---

### 24. [ ] Per-User Rate Limiting (Upgrade from IP-Based)

**Context:** Current rate limiting uses `PeerIpKeyExtractor` (IP-based). For a multi-user app, user-based limiting is more meaningful and accurate (multiple users may share an IP; a single user may rotate IPs).

**Decision to make:** Implementing user-based rate limiting with `tower_governor` requires a custom `KeyExtractor` that reads the session. This implies reorganising routes into an authenticated sub-router so the extractor can assume a valid session exists. Decide whether to implement now or defer until full multi-user support (item 26).

**Actions (when ready):**

- Implement a custom `KeyExtractor` that extracts `user_id` from the session
- Wrap authenticated routes in a sub-router with the user-keyed governor layer
- Keep IP-based limiting as an outer layer for unauthenticated routes (login page, static assets)

---

### 25. [ ] Shopping List — Copy to Clipboard

**Context:** The shopping list panel has a generate button but no way to get the list onto a phone quickly. A "Copy" button using the Clipboard API (`navigator.clipboard.writeText()`) lets the user paste into Reminders, Notes, or any messaging app. Requires HTTPS or localhost (already satisfied) and a user gesture (button click).

**Actions:**

- Add a "Copy to clipboard" button next to (or below) the "Generate" button in `html/calendar.html`
- In `static/calendar.js`, after the list renders, format it as plain text (one line per ingredient: `name — qty unit`) and call `navigator.clipboard.writeText(text)`
- Show brief inline feedback ("Copied!") on success; show an error message if the Clipboard API is unavailable or denied

---

### 26. [ ] Full Multi-User Support

**Context:** The app currently uses a hardcoded `SINGLE_USER_ID` as an interim placeholder. Full multi-user support means users can register (or be invited), and all data is scoped to their `user_id`.

**Actions:**

- Remove `SINGLE_USER_ID` constant; derive `user_id` from the authenticated session on every request
- Decide on registration model: open registration vs. admin-invite-only (the latter is more appropriate for a personal/family app)
- Add user management routes if needed (admin creates accounts, changes passwords)
- Audit all storage queries to ensure `user_id` filtering is present everywhere

---

### 35. [ ] CSRF Protection on State-Mutating Endpoints

**Context:** A security review (2026-04-25) flagged that no CSRF token middleware is applied to the router. All state-mutating routes (POST `/calendar/entries`, DELETE `/recipes/:id`, POST `/calendar/cooked`, etc.) rely solely on session cookies. `SameSite=Lax` blocks cross-site form POSTs but not credentialed `fetch` calls from pages the user visits.

**Actions:**

- Add a CSRF middleware — either `tower-csrf` or a double-submit cookie pattern.
- Alternatively, explicitly set `SameSite=Strict` on the session cookie (already using `tower-sessions`; set via `.with_same_site(SameSite::Strict)` on the cookie config) — this alone closes most practical CSRF vectors for a same-origin app.
- Verify that `Origin` / `Referer` header validation is in place for mutating endpoints as a defence-in-depth measure.

---

## Phase 5 — Test Coverage Gaps

> Identified in code review. All tests use in-memory SQLite via the existing `setup()` helpers.

### 31. [ ] Validation Edge Case Tests

**Context:** Several validation constraints on `Recipe` and `Ingredient` structs are untested.

**Actions:**

- `test_add_recipe_empty_name` — name `""` should fail (`min = 1`)
- `test_add_recipe_too_many_ingredients` — 51 ingredients should fail (`max = 50`)
- `test_add_recipe_too_many_instructions` — 101 instructions should fail (`max = 100`)
- `test_add_recipe_ingredient_name_too_long` — ingredient name of 101 chars should fail (`max = 100`)
- `test_add_recipe_ingredient_unit_too_long` — unit of 33 chars should fail (`max = 32`)
- `test_add_recipe_source_url_empty_string` — `source_url: Some("".to_string())` should fail (URL validator rejects empty strings; documents the behaviour of `Option<String>` + `#[validate(url)]`)

---

### 32. [ ] Auth / Session Tests

**Actions:**

- `test_logout_invalidates_session` — log in, POST `/logout`, assert a subsequent authenticated request redirects to `/login` (session cookie is no longer valid)

---

### 33. [ ] Missing API Integration Tests

**Context:** Several handlers have no direct integration-test coverage.

**Actions:**

- `test_delete_meal_entry_direct` — POST a meal entry, DELETE it via `DELETE /calendar/entries` with the entry id, assert the range query returns empty (tests the handler directly, not just via cascade)
- `test_get_calendar_entries_invalid_range` — GET `/calendar/entries?start=2026-05-07&end=2026-05-01` (start after end), assert `500` or `400` and pin the status code
- `test_body_size_limit` — POST `/recipes` with a body larger than 64 KB, assert `413 Payload Too Large`
- `test_index_route_smoke` — authenticated GET `/` returns `200`
- `test_404_fallback` — authenticated GET `/does-not-exist` returns `404`

---

### 34. [ ] Shopping List Unit Distinction Test

**Context:** The shopping list merges ingredients with the same `(name, unit)`. Ingredients with the same name but different units (e.g. "Flour g" vs "Flour oz") must stay as separate entries. This is not currently tested.

**Actions:**

- `test_get_shopping_list_same_name_different_unit` — plan two meals with "Flour 200g" and "Flour 8oz"; assert the shopping list returns two entries (not one merged entry)

---

## Phase 6 — Future / Deferred

### 37. [ ] User Deletion (Admin)

**Context:** Admins can create and manage users (item 26) but cannot yet delete them. Deleting a user cascades automatically to their recipes, meal plan entries, and cooked log entries via `ON DELETE CASCADE`. Deferred until there is a clear operational need.

**Actions:**

- Add `DELETE /admin/users/:id` route and handler
- Add `storage::delete_user(pool, user_id)` function
- Add `manager::admin_delete_user(pool, admin_user_id, target_user_id)` — prevent self-deletion
- Add integration test: deleting a user removes their recipes and calendar data

---

### 38. [ ] Self-Service Password Change

**Context:** Currently only an admin can change passwords (item 26). Users should eventually be able to change their own password from a profile or settings page.

**Actions:**

- Add `GET /profile` page route serving `html/profile.html`
- Add `POST /profile/password` handler accepting `{ current_password, new_password }`
- In manager: verify `current_password` against stored hash before applying change
- In manager: enforce minimum password length (≥ 8 chars)

---

### 36. [ ] Apple Shortcuts Integration for Shopping List

**Context:** An Apple Shortcut on iPhone could call `GET /calendar/shopping-list` and create individual Reminders items from the response — no native iOS app needed. Deferred until the clipboard approach (item 25) proves insufficient.

**Design decisions to make:**

- **Auth for the endpoint:** Session cookies expire and are fragile in Shortcuts. A static read-only `SHOPPING_LIST_TOKEN` env var checked as a Bearer token or query param is simpler and sufficient for a read-only LAN/personal endpoint.

**Actions:**

- Implement token-based auth for the shopping list endpoint
- Document the Shortcut setup: "Get Contents of URL" → parse JSON → loop → "Add New Reminder"

---

## Dependency Reference

| Package                     | Current | Purpose             | Notes                                                                             |
| --------------------------- | ------- | ------------------- | --------------------------------------------------------------------------------- |
| `axum`                      | `0.7`   | Web framework       | Keep                                                                              |
| `tokio`                     | `1.37`  | Async runtime       | Keep                                                                              |
| `serde` / `serde_json`      | `1.0`   | Serialization       | Keep                                                                              |
| `chrono`                    | `0.4`   | Date types          | Keep; watch for friction with `sqlx` — may switch to `time` crate                 |
| `sqlx`                      | `0.8`   | SQLite              | `{ version = "0.8", features = ["sqlite", "runtime-tokio", "macros", "chrono"] }` |
| `argon2`                    | added   | Password hashing    | In use                                                                            |
| `tower-sessions`            | `0.13`  | Session management  | In use; version must align with `tower-sessions-sqlx-store`                       |
| `tower-sessions-sqlx-store` | added   | Session persistence | In use                                                                            |
| `validator`                 | `0.18`  | Input validation    | In use                                                                            |
| `tower_governor`            | added   | Rate limiting       | In use; IP-based for now — see item 24 for user-based upgrade                     |
| `tracing`                   | `0.1`   | Structured logging  | In use                                                                            |
| `tracing-subscriber`        | `0.3`   | Log formatting      | In use; `{ version = "0.3", features = ["env-filter"] }`                          |
| `dotenvy`                   | added   | `.env` loading      | In use                                                                            |

---

## Architecture Notes

- **Storage layer boundary:** `storage.rs` and `calendar_storage.rs` are the only files that should know about file paths or SQL queries. When migrating to SQLite, only these files change. The rest of the codebase is unaffected.
- **Auth middleware:** Should be implemented as a single Axum extractor/middleware, not per-handler checks. One middleware protects all routes automatically.
- **`user_id` on all domain tables:** Add to `recipes`, `meal_plan`, and `cooked_log` during the SQLite schema design — not as a migration after the fact.
- **Resource abuse defence is layered:** body size limit (framework) → field/count validation (manager) → per-user quotas (manager) → rate limiting (middleware) → monitoring via `tracing` logs.
- **Image uploads are out of scope** until an object storage strategy with per-user byte quotas is designed. The `source_url` field stores a link to an external source only.
- **SQLite → Postgres migration path:** If the app ever needs horizontal scaling or a hosted environment that doesn't persist local files, switching from SQLite to Postgres requires changing only the `sqlx` feature flag, the connection pool type, and the connection string. The rest of the codebase is unaffected by the storage layer boundary.
- **Commit `Cargo.lock`:** For a binary application, `Cargo.lock` should be committed and treated as the source of truth for reproducible builds.
- **Reverse proxy is the upgrade path for TLS:** The Axum app should always bind to `localhost:<port>` in production. A reverse proxy (`caddy`, `nginx`) or tunnel (Cloudflare) sits in front and handles TLS, domain routing, and cert renewal. This means the app itself never needs to change when adding HTTPS.
- **Shopping list auth:** The `/api/shopping-list` endpoint will be called by Apple Shortcuts, which cannot easily manage expiring session cookies. A static read-only token (env var) is the appropriate auth mechanism for this endpoint. It's read-only and personal, so the risk profile is low.
