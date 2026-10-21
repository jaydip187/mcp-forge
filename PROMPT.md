# mcp-forge — Implementation Prompt

> Working name: `mcp-forge` (rename freely). This file is the source of truth for scope and stages.
> Progress lives in `PROGRESS.md`. Read both at the start of every session.

---

## 1. Session protocol (follow every session)

**On start**
1. Read `PROMPT.md` (this file) and `PROGRESS.md`.
2. Run `cargo build && cargo test` (and `npm --prefix ui run build` once the UI exists). If anything is broken, fix it before new work and log it.
3. Pick the first unchecked task in the current stage from `PROGRESS.md`. Do not skip ahead to later stages.
4. State in one line which stage/task you are working on.

**While working**
- Keep changes scoped to the current task. No speculative features from later stages.
- Every task ends with tests (unit and/or integration) that pass.
- Record any non-obvious design decision in `PROGRESS.md → Decisions`.
- If something in this prompt is ambiguous or conflicts with reality, add it to `PROGRESS.md → Open questions` and pick the most conservative option.

**On end (before stopping, always)**
1. Update `PROGRESS.md`:
   - tick finished tasks, mark partial ones `[~]` with a note
   - append a **Session log** entry (date, stage, done, not done, next step, gotchas)
   - set **Next up** to the exact next task
2. Commit with message `stage-N: <summary>`.
3. **Stop at the end of each stage** and request review. Do not start the next stage without approval.

---

## 2. Product summary

A **self-hosted, single-binary** tool that turns:
- HTTP APIs (manual entry, OpenAPI 3.x, Swagger 2.0)
- SQL databases (SQLite, Postgres, MySQL)

into **MCP tools**, managed through an embedded web UI, served over MCP (streamable HTTP + stdio).

Principles:
- **Own canonical Tool IR.** OpenAPI/Swagger/manual are importers into it; UI and MCP server read only the IR.
- **Curation over 1:1 wrapping.** Include/exclude, rename, rewrite descriptions, shape responses.
- **Secrets are references**, never raw values in the DB by default (`env:NAME`, `keyring:NAME`).
- **Safety in layers** for SQL: DB role + read-only session + AST check + limits.
- Everything runs on the user's machine. No telemetry, no external calls except to user-configured upstreams.

---

## 3. Tech stack

| Area | Choice |
|---|---|
| Language | Rust (stable, edition 2021+) |
| HTTP server | `axum`, `tokio`, `tower-http` |
| MCP | `rmcp` (official Rust SDK) — streamable HTTP + stdio |
| Internal DB | SQLite via `sqlx` (migrations in `migrations/`) |
| HTTP client | `reqwest` (rustls) |
| Spec parsing | `openapiv3` (3.x); Swagger 2.0 via own converter or `openapi` crate |
| SQL parsing | `sqlparser` |
| Target DBs | `sqlx` separate pools: sqlite, postgres, mysql (not `sqlx::Any`) |
| Response shaping | `jaq` (jq expressions) |
| Secrets | `keyring`; optional `chacha20poly1305` encrypted-at-rest |
| CLI | `clap` |
| Errors / logs | `thiserror`, `anyhow` (bin only), `tracing` |
| UI | Vite + React + TypeScript SPA, embedded via `rust-embed` |
| Packaging | `cargo-dist`, Docker |

Suggested layout:
```
crates/
  core/        # IR types, validation, errors
  store/       # sqlx SQLite repo + migrations
  importers/   # openapi3, swagger2
  exec_http/   # HTTP executor + shaping
  exec_sql/    # SQL connections, classification, guards
  mcp/         # rmcp ServerHandler over the store
  server/      # axum app, REST API, UI embedding, CLI (bin)
ui/            # Vite React app
migrations/
```

---

## 4. Canonical IR (starting point; refine in Stage 1)

```rust
struct Project { id: Uuid, name: String, slug: String, created_at, updated_at }

struct ToolDef {
    id: Uuid,
    project_id: Uuid,
    name: String,              // MCP-safe: [a-zA-Z0-9_-], <= 64 chars, unique per project
    description: String,
    kind: ToolKind,
    enabled: bool,
    source: Source,            // Manual | OpenApi{spec_id, op_id} | Swagger{spec_id, op_id}
    overrides: Overrides,      // which fields the user edited (survive re-import)
    response_shape: Option<String>, // jaq expr
    created_at, updated_at,
}

enum ToolKind { Http(HttpSpec), Sql(SqlSpec) }

struct HttpSpec {
    method: HttpMethod,
    base_url: String,
    path_template: String,     // /users/{id}
    params: Vec<ParamDef>,
    body_schema: Option<serde_json::Value>,
    auth_ref: Option<Uuid>,    // -> AuthConfig
    timeout_ms: u32,
}

struct ParamDef {
    name: String, location: ParamLocation, // Path | Query | Header | Body
    schema: serde_json::Value, required: bool,
    description: Option<String>,
    fixed_value: Option<serde_json::Value>, // hidden from agent when set
}

struct AuthConfig { id, project_id, kind: AuthKind /* ApiKey{header|query} | Bearer | Basic */, secret_ref: String }

struct DbConnection {
    id, project_id, name, driver: Driver /* Sqlite | Postgres | Mysql */,
    dsn_ref: String,           // env:/keyring:
    access_mode: AccessMode,   // ReadOnly | CuratedWrite | FullAccess
    row_limit: u32, timeout_ms: u32, max_affected_rows: u32,
    allow_ddl: bool,
    table_allowlist: Vec<String>,
    column_masks: Vec<ColumnMask>, // e.g. users.password_hash, *.email
}

struct SqlSpec { connection_id: Uuid, mode: SqlMode }

enum SqlMode {
    Query { sql: String, params: Vec<ParamDef> }, // curated, params always bound
    ListTables, DescribeTable, RunSql,           // exploration, opt-in
}
```

MCP input schema is **derived** from the IR (never stored separately). Keep a location map so `call_tool` args split back into path/query/header/body.

---

## 5. Stages

Each stage = reviewable PR-sized chunk. Stop and request review after each.

### Stage 0 — Scaffold
- Cargo workspace with the crates above (empty but compiling).
- `clap` CLI: `mcp-forge serve [--bind 127.0.0.1:7410] [--data-dir ~/.mcp-forge]`, `mcp-forge stdio --project <slug>`.
- `tracing` setup, config loading (CLI > env > defaults).
- SQLite store init + `sqlx` migrations runner.
- CI: fmt, clippy (`-D warnings`), test.
- **Done when:** `mcp-forge serve` starts, creates the DB, serves `/healthz`.

### Stage 1 — IR + store + REST API
- IR types in `core` with validation (tool name rules, unique names, path params match template).
- Migrations + repository for Project, ToolDef, AuthConfig (JSON columns for nested specs are fine).
- REST: CRUD for projects, tools, auth configs under `/api`.
- **Done when:** integration tests create a project + manual HTTP tool via API and read it back.

### Stage 2 — MCP server + HTTP executor
- `rmcp` `ServerHandler` backed by the store: `list_tools` (enabled tools, derived schemas), `call_tool`.
- Streamable HTTP at `/mcp/{project_slug}`; stdio mode for the same project.
- HTTP executor: build request from args, apply auth via secret refs (`env:` first, `keyring:` next), timeout.
- LLM-friendly errors: status + upstream error body, truncated; validation errors name the bad arg.
- Emit `notifications/tools/list_changed` when tools in a project change.
- **Done when:** an MCP client (MCP Inspector) lists and calls a manual GET tool against a mock server (`wiremock`) in tests.

### Stage 3 — Embedded UI (manual flow)
- Vite React TS app in `ui/`, built and embedded with `rust-embed`, served at `/`.
- Screens: projects list, project detail, tool form (method, URL, params table, body schema JSON editor, fixed values), auth configs (secret ref only).
- **Test console:** fill args → run → show request sent, raw response, shaped response.
- **Endpoint view:** MCP URL + copyable client config snippets (HTTP and stdio).
- Security: bind `127.0.0.1` by default; if binding non-loopback, require `--admin-token` (bearer on `/api` and UI).
- **Done when:** a user can create a tool entirely in the UI and call it from an MCP client.

### Stage 4 — OpenAPI 3.x importer
- Import from file upload or URL (URL fetch only on explicit user action).
- `$ref` resolution with depth cap; circular refs handled; `allOf/oneOf/anyOf` passed through.
- Naming: `operationId` → sanitized; fallback `{method}_{path}`; dedupe.
- Param collisions across locations → prefix only on collision (`path_id`, `body_id`).
- Preview screen: list operations with checkboxes, filter by tag/method/path glob; create selected tools.
- Store the raw spec (`specs` table) for re-import.
- **Done when:** Petstore + one large real spec import cleanly; tests cover refs, collisions, naming.

### Stage 5 — Swagger 2.0 importer
- Convert 2.0 → IR (body/formData params, `definitions`, `securityDefinitions`, `basePath`/`host`/`schemes`).
- Same preview flow as Stage 4.
- **Done when:** a Swagger 2.0 Petstore imports equivalently to the 3.x one.

### Stage 6 — Re-import, overrides, export/import
- Track user-edited fields in `overrides`; re-import regenerates base fields, preserves overrides, reports added/removed/changed operations in a diff preview.
- `mcp-forge export --project x > tools.yaml` / `import`; UI buttons too. **Secrets exported as refs only.**
- **Done when:** edit description → re-import changed spec → description preserved, new op shown in diff.

### Stage 7 — Curation & shaping
- Per-tool `response_shape` (jaq), null-field stripping, max response size with truncation notice.
- Per-project filters: read-only mode (GET only), tag/path include/exclude.
- Optional `search_tools` meta-tool for projects with many tools.
- **Done when:** shaped output shown in test console; tests for jaq shaping and truncation.

### Stage 8 — DB connections + curated read-only queries
- `DbConnection` CRUD + UI; **Test connection** that reports whether the role can write (`has_table_privilege` / `SHOW GRANTS`), warns if it exceeds `access_mode`.
- Curated `SqlMode::Query` tools; args always bound, never interpolated.
- Read-only enforcement, all layers:
  1. recommend SELECT-only role (UI warning)
  2. session: PG `BEGIN READ ONLY` + `statement_timeout`; MySQL `START TRANSACTION READ ONLY` + `max_execution_time`; SQLite `?mode=ro` + `PRAGMA query_only = ON`
  3. `sqlparser` AST: single statement, `SELECT`/`WITH` only, walk full tree (reject data-modifying CTEs)
  4. function denylist (e.g. `pg_terminate_backend`, `pg_read_file`, `dblink`, `lo_*`, `nextval`, `setval`)
  5. row limit + timeout; cap cell length
- Output: `{columns: [...], rows: [[...]], truncated: bool}`; column masking applied.
- Order: SQLite → Postgres → MySQL.
- **Done when:** tests (testcontainers for PG/MySQL) prove writes are rejected at each layer.

### Stage 9 — Exploration tools + read-only `run_sql`
- `list_tables`, `describe_table`, `run_sql` (read-only), opt-in per connection, respecting `table_allowlist` and masks.
- Tool descriptions generated from connection mode; set MCP annotations (`readOnlyHint: true`).
- **Done when:** agent can explore schema and query within allowlist; disallowed tables rejected.

### Stage 10 — Writes (CuratedWrite / FullAccess)
- Statement classifier: Read | Write | DDL | Admin; gate by `access_mode` (+ `allow_ddl`).
- Curated write tools (e.g. `update_order_status`) with enum/typed params.
- All writes in a transaction; roll back if affected rows > `max_affected_rows`.
- `dry_run` arg: execute, report affected count, always roll back.
- Require `WHERE` on UPDATE/DELETE.
- Annotations: `destructiveHint: true` for write-capable tools.
- **Audit log** table + UI view: ts, tool, sql, params, rows affected, committed/rolled back.
- `FullAccess` behind an explicit confirmation in the UI with a loud warning.
- **Done when:** tests cover cap rollback, dry-run, WHERE enforcement, mode gating, audit entries.

### Stage 11 — Packaging & docs
- `cargo-dist` release builds (Linux, macOS, Windows), Dockerfile (non-root, data volume).
- README: install, quick start, connecting Claude/other MCP clients, security model, secret refs.
- **Done when:** fresh machine install → working tool in under 5 minutes following README.

---

## 6. Non-negotiables

- Never log or return secret values; redact in errors and audit log.
- Never interpolate agent input into SQL or URLs without encoding/binding.
- No network calls except to user-configured upstreams/DBs and explicit spec URL imports.
- `cargo clippy -- -D warnings` and all tests green at the end of every task.
- Don't add dependencies outside the stack table without logging the reason in `PROGRESS.md → Decisions`.
