# Recipe App — Action Items & Technical Roadmap

> Generated from code review session. Items are ordered by implementation priority.
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

## Phase 2 — Soon (Correctness & Robustness)

### 27. [x] Allow Multiple Entries Per Slot

**Context:** Currently `meal_plan` has `UNIQUE(user_id, date, slot)` enforced at the DB level, and `add_meal_entry` uses `INSERT OR REPLACE`. This means only one recipe per slot per day. The intended model is multiple entries per slot (main dish + salad + dessert all at dinner, for example).

**Actions:**

- Add a new migration that drops the `UNIQUE(user_id, date, slot)` constraint (SQLite requires recreating the table — `ALTER TABLE` cannot drop constraints)
- Change `add_meal_entry` in `calendar_storage.rs` from `INSERT OR REPLACE` to plain `INSERT`
- Change `delete_meal_entry` to delete by `id` rather than `(user_id, date, slot)` so individual entries can be targeted
- Add `id` field to `MealEntry` in `model.rs`; populate it from the query result in `load_meal_entries_in_range`
- Update `remove_planned_meal` in `calendar_manager.rs` to accept `id: i64` instead of `(date, slot)`
- Update `handle_delete_meal_entry` in `network.rs` to read the entry `id` from the request body
- Update `calendar.html` to send the entry `id` on delete
- Flip `test_meal_entry_replace_on_same_slot` in `calendar_storage` — it now asserts the old behaviour being removed; replace with a test asserting both entries persist
- Update integration test `test_calendar_plan_and_shopping_list` if it relies on slot uniqueness

---

## Phase 3 — Nice to Have (UX & Polish)

### 15. [x] Add Loading States to the Calendar UI

**Actions:**

- Disable week navigation buttons during `loadWeek()`
- Show a spinner or skeleton state in the calendar grid while data is fetching

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

### 25. [ ] Shopping List Export Endpoint + Apple Shortcuts Integration

**Context:** The shopping list (aggregated ingredients across meal plan entries for a date range) should be exposed as a clean JSON API endpoint. An Apple Shortcut on iPhone calls this endpoint and creates Reminders items from the response — no native iOS app needed.

**Design decisions to make:**

- **Auth for the endpoint:** Session cookies work but expire and are fragile in Shortcuts. A static read-only `SHOPPING_LIST_TOKEN` env var checked as a Bearer token or query param is simpler and sufficient for a read-only LAN/personal endpoint. Decide before implementing.
- **Ingredient unit normalisation:** ✅ Done — all weight units (g/kg/oz/lb) normalise to `g`; all volume units (ml/l/tsp/tbsp/cup) normalise to `ml`; names matched case-insensitively. Cross-dimension (weight vs. volume) stays separate.

**Actions:**

- Add `ShoppingListItem` struct to `model.rs` (aggregated ingredient: name, total quantity, unit)
- Add a storage query in `calendar_storage.rs` that fetches ingredients for all meal entries in a date range, grouped and summed by `(name, unit)` after normalisation
- Add a manager method in `calendar_manager.rs`
- Add `GET /api/shopping-list?start_date=YYYY-MM-DD&end_date=YYYY-MM-DD` to `network.rs`; default range is the current week (Mon–Sun) if params are omitted
- ~~Implement ingredient unit normalisation (at minimum: `g`/`kg`, `ml`/`l`, `tsp`/`tbsp`/`cup`)~~ ✅ Done (see above)
- Document the Shortcut setup: "Get Contents of URL" → parse JSON → loop → "Add New Reminder"

---

### 26. [ ] Full Multi-User Support

**Context:** The app currently uses a hardcoded `SINGLE_USER_ID` as an interim placeholder. Full multi-user support means users can register (or be invited), and all data is scoped to their `user_id`.

**Actions:**

- Remove `SINGLE_USER_ID` constant; derive `user_id` from the authenticated session on every request
- Decide on registration model: open registration vs. admin-invite-only (the latter is more appropriate for a personal/family app)
- Add user management routes if needed (admin creates accounts, changes passwords)
- Audit all storage queries to ensure `user_id` filtering is present everywhere

---

### 28. [x] GitHub Actions CI

**Context:** Prerequisite for the agentic workflow (item 29). PRs should be blocked from merging until tests pass. Merge requires manual approval.

**Actions:**

- Create `.github/workflows/ci.yml` that runs on `push` and `pull_request` to `main`
- Steps: checkout → install Rust stable → `cargo build --locked` → `cargo clippy -- -D warnings` → `cargo test`
- Cache the `~/.cargo` registry and the `target/` directory between runs to avoid full recompiles on the Pi-class runners
- Add a branch protection rule on `main`: require the `ci` status check to pass and require at least one human approval before merge

---

### 29. [x] Agentic Workflow (Plan → Implement → Test → Review → Merge)

**Context:** Automates the path from a GitHub issue to a reviewed, tested PR. Human stays in the loop at two gates: approving the plan before implementation starts, and approving the PR before merge.

**Flow:**

1. A GitHub issue is created and assigned to `claude`
2. **Planning agent** (triggered by GitHub Actions `issues` event) reads the issue, inspects the codebase, and posts a structured implementation plan as an issue comment
3. **Human approves** the plan by replying "approved" in the issue thread
4. **Implementation agent** (triggered by a comment-match workflow) checks out a branch, implements the plan, and opens a PR
5. **Test-writing agent** (triggered on PR open) reads the plan + diff and adds or updates tests to maintain coverage
6. **Review agent** (triggered on PR open or push) posts a code review comment on the PR
7. **GitHub Actions CI** (item 28) runs `cargo test`; PR is blocked until green
8. **Human approves** the PR and merges

**Actions:**

- Implement planning agent: GitHub Actions workflow on `issues` event (type: assigned), use Claude API to read issue + relevant source files, post plan comment
- Implement approval detection: workflow on `issue_comment` event, check comment body matches "approved" and commenter is the repo owner; dispatch implementation event
- Implement implementation agent: checks out feature branch, calls Claude Code or Claude API with plan + codebase context, commits changes, opens PR
- Implement test-writing agent: triggered on PR open, reads plan + diff, appends tests, pushes to same branch
- Implement review agent: triggered on PR open/push, posts review as PR review comment
- Store `ANTHROPIC_API_KEY` as a GitHub Actions secret
- All agent prompts should reference `CLAUDE.md` for architecture rules and coding conventions

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
- Note: add this test only after item 27 (multiple entries per slot) is implemented if the test needs two entries on the same date+slot; otherwise it can be added now using different slots

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
