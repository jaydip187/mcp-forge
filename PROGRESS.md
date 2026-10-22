# mcp-forge — Progress Tracker

> Read at the start of every session, update at the end. See `PROMPT.md §1` for the session protocol.
> Legend: `[ ]` todo · `[~]` in progress / partial · `[x]` done · `[-]` dropped (say why)

## Next up
Stage 11 review. All stages done; v0.1.0 released at github.com/jaydip187/mcp-forge

## Current stage
**Stage 11 — Packaging & docs** · status: done · review: requested

---

## Stage checklist

### Stage 0 — Scaffold
- [x] Cargo workspace + crate skeletons (core, store, importers, exec_http, exec_sql, mcp, server)
- [x] clap CLI: `serve`, `stdio`
- [x] tracing + config loading (CLI > env > defaults)
- [x] SQLite init + sqlx migrations runner
- [x] `/healthz`
- [x] CI: fmt, clippy -D warnings, test
- [x] Stage review approved

### Stage 1 — IR + store + REST API
- [x] IR types + validation
- [x] Migrations: projects, tools, auth_configs
- [x] Repository layer
- [x] REST CRUD under `/api`
- [x] Integration tests
- [x] Stage review approved

### Stage 2 — MCP server + HTTP executor
- [x] rmcp ServerHandler (list_tools / call_tool) from store
- [x] Schema derivation + arg location map
- [x] Streamable HTTP `/mcp/{slug}`
- [x] stdio mode
- [x] HTTP executor + auth via secret refs (env:, keyring:)
- [x] LLM-friendly errors
- [x] `tools/list_changed` notifications
- [x] wiremock + MCP client tests
- [x] Stage review approved

### Stage 3 — Embedded UI (manual flow)
- [x] Vite React TS app + rust-embed
- [x] Projects / tools / auth screens
- [x] Tool form (params table, body schema, fixed values)
- [x] Test console
- [x] Endpoint view + client config snippets
- [x] Loopback default + admin token for non-loopback
- [x] Stage review approved

### Stage 4 — OpenAPI 3.x importer
- [x] File / URL import
- [x] $ref resolution (depth cap, circular)
- [x] Naming + dedupe + collision prefixing
- [x] Preview + filter UI
- [x] Raw spec storage
- [x] Tests (Petstore + large real spec)
- [x] Stage review approved

### Stage 5 — Swagger 2.0 importer
- [x] 2.0 → IR conversion
- [x] Preview flow reuse
- [x] Equivalence test vs 3.x Petstore
- [x] Stage review approved

### Stage 6 — Re-import, overrides, export/import
- [x] Override tracking
- [x] Re-import with diff preview
- [x] YAML export/import (CLI + UI), secrets as refs only
- [x] Stage review approved

### Stage 7 — Curation & shaping
- [x] jaq response shaping
- [x] Null stripping + size truncation
- [x] Project filters (GET-only, tag/path)
- [x] `search_tools` meta-tool
- [x] Stage review approved

### Stage 8 — DB connections + curated read-only queries
- [x] DbConnection CRUD + UI
- [x] Test connection + privilege check
- [x] Curated Query tools (bound params)
- [x] Read-only session per driver
- [x] AST check (single SELECT/WITH, full tree walk)
- [x] Function denylist
- [x] Row limit, timeout, cell cap, column masks
- [x] SQLite → Postgres → MySQL
- [x] testcontainers tests per layer
- [x] Stage review approved

### Stage 9 — Exploration + read-only run_sql
- [x] list_tables / describe_table / run_sql (opt-in)
- [x] Table allowlist + masks
- [x] Generated descriptions + readOnlyHint
- [x] Stage review approved

### Stage 10 — Writes
- [x] Statement classifier (Read/Write/DDL/Admin)
- [x] AccessMode gating + allow_ddl
- [x] Curated write tools
- [x] Transaction + affected-rows cap rollback
- [x] dry_run
- [x] WHERE required on UPDATE/DELETE
- [x] destructiveHint annotations
- [x] Audit log + UI
- [x] FullAccess confirmation UX
- [x] Stage review approved

### Stage 11 — Packaging & docs
- [x] cargo-dist releases
- [x] Dockerfile
- [x] README + security model
- [x] Fresh-install test (Docker image + published v0.1.0 installer on a clean container)
- [ ] Stage review approved

---

## Decisions
<!-- Append: YYYY-MM-DD · decision · reason · alternatives considered -->
- 2026-10-21 · Package names are `forge-<crate>` (dirs stay `crates/core` etc.); the binary package is `mcp-forge` · a package named `core` would shadow Rust's `core` crate · `mcp-forge-core` (too long)
- 2026-10-21 · Config precedence via clap `env` attrs: `MCP_FORGE_BIND`, `MCP_FORGE_DATA_DIR`, `MCP_FORGE_PROJECT`; no config file yet · simplest thing that meets CLI > env > defaults · `figment`/`config` crates (not in stack)
- 2026-10-21 · Home dir resolved from `HOME` then `USERPROFILE` instead of the `dirs` crate · avoids a dependency outside the stack · `dirs`, `std::env::home_dir`
- 2026-10-21 · Logs always go to stderr · keeps stdout clean for MCP stdio transport
- 2026-10-21 · Internal DB: `<data-dir>/mcp-forge.db`, WAL, foreign keys on, busy_timeout 5s, pool of 5; migrations embedded via `sqlx::migrate!("../../migrations")` (empty for now, `.gitkeep` keeps the dir)
- 2026-10-21 · Dev-dependency `tempfile` added (not in stack table) · temp data dirs in store/server tests · in-memory SQLite only (cannot verify file creation)
- 2026-10-21 · `/healthz` pings the DB; returns `{"status":"ok"}` or 503 · health should reflect store availability
- 2026-10-21 · Deps added: `uuid` (ids), `chrono` (timestamps, + sqlx `chrono` feature), `url` (base_url validation; already transitive via reqwest), axum `macros` feature (custom JSON/Path rejections) · not in stack table, all small/standard
- 2026-10-21 · `ToolKind` has only `Http` for now; `Sql` variant + `SqlSpec` land in Stage 8 · avoid speculative types; serde tag `type` keeps the JSON shape forward-compatible
- 2026-10-21 · `SecretRef` is a newtype that only parses `env:NAME` / `keyring:NAME`; raw values fail deserialization (400) so they can never reach the DB
- 2026-10-21 · `AuthConfig` gained a `name` (unique per project) for UI display. `Basic { username }` + secret_ref = password; `ApiKey { location: header|query, name }`
- 2026-10-21 · Param names follow `[a-zA-Z0-9_.-]{1,64}` (MCP input-schema key rule) and are also the wire name · see Open questions
- 2026-10-21 · Body: either `Body` params or a whole-body `body_schema`, not both · keeps Stage 2 arg→request mapping unambiguous
- 2026-10-21 · Path params must be `required` or have a `fixed_value`; every `{placeholder}` needs a matching path param and vice versa; no `?`/`#` in path_template; base_url must be http(s), have a host, and contain no credentials/query/fragment
- 2026-10-21 · Tool `kind.auth_ref` is mirrored in a `tools.auth_ref` FK column (NO ACTION, so project cascade works); the store also checks the config belongs to the same project → 422 `kind.auth_ref`. Deleting an in-use auth config → 409 listing the tools
- 2026-10-21 · Updating a project without `slug` keeps the existing slug (renames don't move the MCP URL)
- 2026-10-21 · REST: ids (UUID) in paths; POST 201, DELETE 204, PUT = full replace of user-editable fields (`source`/`overrides` system-managed, API-created tools are always `Source::Manual`). Errors: `{"error":{"code","message","fields"?}}` with 400/404/409/422/500; internal errors are logged, not echoed
- 2026-10-21 · rmcp is **3.6** (supports both initialize-based protocol ≤2025-11-25 and stateless 2026-07-28). reqwest bumped 0.12→0.13 to share rmcp's version
- 2026-10-21 · Deps added: `rmcp`, `keyring` 3.6 (apple-native, windows-native, async-secret-service+crypto-rust → no system libs on Linux), `wiremock` (dev), `tokio-util` (CancellationToken for rmcp config; already transitive)
- 2026-10-21 · Input schema derived in `forge-core` (`HttpSpec::derive_input`): fixed-value params hidden; a name used in >1 location is prefixed `{location}_{name}` (only on collision); `body_schema` exposed as arg `body`; `additionalProperties: false`. Validation rejects derived-name clashes and names >64 chars
- 2026-10-21 · Arg checking is light (unknown/missing/top-level `type`/`enum`), not full JSON Schema · `jsonschema` crate not in stack; upstream validates the rest
- 2026-10-21 · Executor: path values percent-encoded (unreserved only, `..` blocked); query arrays repeat the key; header/path arrays comma-joined; JSON body from body params or whole `body`. Redirects followed only to the same host (max 5) so custom auth headers never leave the origin
- 2026-10-21 · Secret values redacted (raw + URL-encoded; Basic token too) from reported URL, error messages and response bodies; reqwest errors use `without_url()`. Secret resolution runs in `spawn_blocking` (keyring may block). `SecretResolver` trait lives in core; `DefaultSecretResolver` (env, then keyring service `mcp-forge`, user = NAME) in exec_http
- 2026-10-21 · Tool results: 2xx/3xx → body text (empty → status line); ≥400 → `isError` with status line, method, redacted URL, body cut to 4000 chars; arg errors list each arg; config/transport problems → `isError` with a clear prefix. Unknown/disabled tool → JSON-RPC invalid_params
- 2026-10-21 · Annotations for HTTP tools: `readOnlyHint` true for GET/HEAD/OPTIONS, `openWorldHint` true, `destructiveHint` true for DELETE. Empty description → `"METHOD /path"`
- 2026-10-21 · `/mcp/{slug}`: one `StreamableHttpService` (LocalSessionManager, rmcp defaults incl. loopback-only Host allowlist) per project id, created lazily; handler re-reads tools from the store per request. Shutdown cancels a token so SSE streams don't block graceful shutdown
- 2026-10-21 · list_changed: in-process `ChangeBus` (broadcast) published by REST tool create/update/delete and project delete. Legacy sessions get push notifications (forwarder task per session, exits when transport closes); 2026-07-28 clients via `subscriptions/listen`. stdio mode polls a store fingerprint every 2s to catch edits from the `serve` process
- 2026-10-21 · UI uses **shadcn/ui** (radix base, "nova" preset) + Tailwind v4 (`@tailwindcss/vite`), lucide icons, sonner toasts · user request mid-stage; replaced hand-written CSS. Theme retuned to steel + teal tokens (light/dark follow OS via `.dark` class); `next-themes` dropped from the generated Toaster
- 2026-10-21 · Fonts bundled via `@fontsource-variable/geist` + `geist-mono` (embedded in the binary) · PROMPT forbids network calls beyond upstreams, so no Google Fonts
- 2026-10-21 · Hash routing (`#/p/{id}/tools/{tid}`) · server never needs an SPA fallback; unknown asset paths 404
- 2026-10-21 · rust-embed `allow_missing` + `crates/server/build.rs` `rerun-if-changed=../../ui/dist` · binary builds without Node (serves a "UI not built" page); rebuilding the UI re-embeds it. CI builds the UI first and passes `ui/dist` to the Rust jobs
- 2026-10-21 · UI responses carry CSP (`default-src 'self'`, inline styles allowed for Radix), nosniff, no-referrer; hashed `assets/*` cached immutable, `index.html` no-cache
- 2026-10-21 · Security: loopback bind (default) → requests must carry a loopback `Host` (blocks DNS rebinding against /api, /mcp and UI). Non-loopback bind → `--admin-token`/`MCP_FORGE_ADMIN_TOKEN` required (≥16 chars), checked constant-time as Bearer on **/api and /mcp**, and rmcp's Host allowlist is disabled. `/healthz` is always open
- 2026-10-21 · Admin token in the browser lives in `sessionStorage` (per tab) and is sent as a Bearer header; 401 → unlock screen
- 2026-10-21 · New endpoints: `GET /api/info` (version, exe, data_dir, auth_required for snippets), `GET /api/projects/{pid}/tools/{tid}/mcp` (tool exactly as MCP clients see it), `POST …/tools/{tid}/test {arguments}` → `{request, response, error, agent_view, shaped:null}`. Test console and MCP `tools/call` share `forge_mcp::run_tool`, so results are identical. Console runs disabled tools too
- 2026-10-21 · Snippets: Claude Code CLI (`claude mcp add --transport http …` / `-- <exe> stdio …`) + `mcpServers` JSON for HTTP and stdio; single-quoted shell args (literal in bash and PowerShell); token shown only as `<your-admin-token>`
- 2026-10-21 · vitest 5 (vitest 3 pulled tinypool with critical advisories); `npm audit` clean
- 2026-10-22 · Shaping uses jaq 3.1 (`jaq-core`/`jaq-std`/`jaq-json` 2.0); values cross the boundary as JSON text. Expression output: none → `null`, one → value, many → array; >10 000 outputs is an error (runaway guard). jaq differs from jq in places (e.g. `.[0]` on an object is `null`, not an error)
- 2026-10-22 · Pipeline for successful, non-empty responses: parse JSON (if the body looks like JSON and shaping is wanted) → strip nulls (project setting; object members only, array items keep positions) → jq (`response_shape`) → compact JSON → cap at `max_response_chars` with a notice saying how much was cut and how to get less. A failing expression returns the unshaped body prefixed with a note (not an error). Error responses keep the fixed 4 000-char body excerpt. Shared by MCP and the test console (`ToolRun.shaped`)
- 2026-10-22 · `response_shape` must compile at save time (API create/update and tools.yaml import → 422 on `response_shape`)
- 2026-10-22 · `ProjectSettings` (projects.settings JSON, migration 0004): read_only, include/exclude tags, include/exclude path globs, strip_nulls (default off), max_response_chars (default 50 000, 500–2 000 000), tool_search (default off). Changed only via `PUT /api/projects/{id}/settings` (keeps `ProjectInput` unchanged); a change notifies MCP sessions
- 2026-10-22 · Exposure = enabled AND (read-only → GET/HEAD/OPTIONS) AND tag include/exclude (case-insensitive) AND path include/exclude (same glob rules as the UI: `*`/`?` within a segment, `**` across, no wildcard → substring). Hidden tools are neither listed nor callable (unknown tool). UI mirrors the rules (`lib/visibility.ts`) to label hidden tools
- 2026-10-22 · Tools gained `tags` (tools.tags JSON column; ≤20, 1–64 chars; overridable field `tags`); importers fill them from operation tags
- 2026-10-22 · Tool search: when on, `tools/list` returns only `search_tools {query, limit≤25}` (ranks name 5 > tag 3 > path 2 > description 1 per term; empty query lists tools) and `call_tool {name, arguments}` (proxies to any exposed tool; can't call the meta-tools). Results carry name, description and input schema as JSON. Search respects filters
- 2026-10-22 · Overrides: 11 tracked fields (`name`, `description`, `enabled`, `response_shape`, `kind.method|base_url|path_template|params|body_schema|auth_ref|timeout_ms`); params tracked as one unit. `update_tool` on an imported tool adds every changed field; manual tools never track. Overrides are only removed explicitly (`POST …/tools/{id}/overrides/reset`, all or listed fields), which keeps current values but lets the next re-import overwrite them
- 2026-10-22 · Re-import merge (`forge_core::merge_overrides`): start from the fresh spec tool, copy back overridden fields; `kind.path_template` and `kind.params` travel together (path params must match the template)
- 2026-10-22 · Re-import matching by `Source.op_id` (operationId, else `"METHOD /path"`); an operationId rename shows up as removed + added. Results: changed (with changed fields and kept overrides), unchanged, added (selectable; supported ones pre-selected), removed (kept unless ticked for deletion), blocked (op now unsupported → tool left as is). Renamed-by-spec tools are deduped against other tools
- 2026-10-22 · Specs store their import options (`base_url`, `auth_ref`; migration 0003) and re-import reuses them unless changed (`""` clears). Re-import without new content re-runs the stored spec (e.g. to change base URL). The spec row is updated in place (same id) and gets `updated_at`
- 2026-10-22 · `Store::apply_reimport` is one transaction: spec row, deletes (scoped to the spec), system updates (overrides untouched, no new ones), inserts
- 2026-10-22 · Bundle `tools.yaml` v1: `project {name, slug}`, `auth_configs` (name, kind, secret_ref), `tools` (ToolInput with `auth: <config name>` instead of ids; nulls/empties dropped). Raw secrets fail to parse on import. Imported tools become manual (spec links/overrides aren't exported). Conflict policy fail (default) / skip / replace; identical existing auth configs are reused silently. All-or-nothing
- 2026-10-22 · CLI `mcp-forge export --project x [-o file]` (stdout by default) and `mcp-forge import --project x [file|-] [--on-conflict …]`; import creates the project (name from the file) if the slug doesn't exist. API: `GET …/export` (attachment), `POST …/import/bundle {content, on_conflict}`
- 2026-10-22 · Swagger 2.0 is converted to an OpenAPI 3.0 document (`swagger2::to_openapi3`) and then imported by the 3.x importer · one mapping path means 2.0 and 3.x imports can't drift; equivalence is structural. `import_any` dispatches on the `swagger` field
- 2026-10-22 · 2.0 conversion: servers = preferred https scheme + host + basePath (no host → basePath, resolved against fetch URL); `definitions` → components.schemas with refs rewritten; global/op/path-level params inlined (op overrides path); `body` → JSON requestBody (first JSON-ish `consumes`); `formData` → urlencoded/multipart requestBody (required if any field is); `type: file` → string/binary; `x-nullable` → nullable; securityDefinitions basic/apiKey/oauth2 → 3.0 schemes; `collectionFormat` → style/explode
- 2026-10-22 · Specs record `kind` (`openapi3`/`swagger2`); 2.0 tools get `Source::Swagger { spec_id, op_id }`. Same preview/commit endpoints (`/import/openapi…`) serve both
- 2026-10-22 · Array query params declared delimited (2.0 csv/ssv/pipes, 3.x `explode: false`, space/pipeDelimited) import with a warning: the executor sends repeated keys. See known issues
- 2026-10-22 · OpenAPI parsed as `serde_json::Value`, **not** the `openapiv3` crate · `openapiv3` only models 3.0.x; real specs (e.g. GitHub's descriptions-next) are 3.1. Value-based handling covers both and makes $ref inlining with depth/cycle control simple
- 2026-10-22 · YAML via `serde_yaml_ng` 0.10 (maintained fork of deprecated `serde_yaml`); non-string YAML keys (`200:`) are stringified. Stage 6 YAML export will reuse it
- 2026-10-22 · `$ref` handling: local refs only (external → descriptive stub); schemas fully inlined up to 8 nested refs; a ref already on the expansion stack becomes `{type: object, description: "…recursive; not expanded"}`; sibling keywords next to $ref override (3.1). 3.0 `nullable: true` → type union. `allOf/oneOf/anyOf` passed through
- 2026-10-22 · **ParamDef.wire_name** (resolves the Stage 1 open question): agent-facing `name` is MCP-safe (sanitized), `wire_name` keeps the upstream name (`filter[name]`, `$top`, `@type`). Not allowed on path params — the importer rewrites `{placeholder}`s instead. Duplicate check is on (location, wire name)
- 2026-10-22 · Import mapping: tool name = sanitized operationId, else `{method}_{path}`, deduped `_2…` within the spec and against existing project tools (preview shows final names). Description = [Deprecated.] summary + description. JSON request bodies that are plain objects are flattened into body params (readOnly props dropped); composed/non-object bodies become one `body` arg. Accept/Content-Type/Authorization header params dropped. Cookie params skipped (required → op unsupported). Required non-JSON bodies → unsupported; optional ones skipped with a warning. Undeclared `{placeholders}` become required string params
- 2026-10-22 · Base URL: op/path-item `servers` win, else user override, else first doc server (variables → defaults; relative URLs resolved against the fetch URL). No base → op unsupported until the user enters one
- 2026-10-22 · Auth on import: user picks an existing auth config; applied to ops whose effective `security` is non-empty (or to all ops if the spec declares no security). Security schemes are described with a suggested AuthKind; auth configs aren't auto-created
- 2026-10-22 · Stateless import API: `POST …/import/openapi/preview {content|url, base_url}` and `POST …/import/openapi {content, source_url, filename, base_url, auth_ref, operations:[{key, name?}]}`; both re-parse. URL fetched server-side only on the preview call (http/https, 30 s, 48 MB cap, ≤5 redirects); fetched content echoed back for the commit. Body limit raised to 48 MB on these two routes
- 2026-10-22 · Commit is one transaction (spec row + all tools); tools get `Source::OpenApi { spec_id, op_id }` with op_id = operationId or `"METHOD /path"` (the op key, used for Stage 6 re-import)
- 2026-10-22 · Preview UI: specs with ≤50 operations start fully selected, larger ones start empty; filters by tag, method and path (glob: `*`/`?` within a segment, `**` across; no wildcard → substring)
- 2026-10-22 · `DbConnection` IR in core (`db.rs`): driver, `dsn_ref: SecretRef` (raw DSNs fail to parse), access_mode, row_limit (default 100, ≤10 000), timeout_ms (default 10 s, 100 ms–300 s), max_affected_rows, allow_ddl, table_allowlist, column_masks. Only `access_mode: read_only` and `allow_ddl: false` validate until Stage 10 · storing a write mode that does nothing would mislead
- 2026-10-22 · Storage: `db_connections` table (migration 0005; limits/allowlist/masks in one `settings` JSON column) + `tools.connection_ref` FK mirroring `kind.connection_id` (same pattern as auth_ref). In-use connections can't be deleted or change driver (409); tools must use a connection from their own project (422 `kind.connection_id`)
- 2026-10-22 · `ToolKind::Sql(SqlSpec { connection_id, mode: Query { sql, params: [SqlParam] } })`. `SqlParam` is its own type (name, scalar schema type string/integer/number/boolean, required, description), not `ParamDef` · location/fixed_value/wire_name are meaningless for SQL. Names `[A-Za-z_][A-Za-z0-9_]{0,63}`
- 2026-10-22 · SQL placeholders are `:name`, rewritten with sqlparser's tokenizer (skips strings, comments, `::` casts) by splicing source offsets, so the rest of the SQL is byte-for-byte unchanged: PG `$n` (repeats reuse the number), SQLite/MySQL `?` per occurrence. Raw `?`/`$1` in curated SQL are refused. Binds are typed from the declared param type; missing optional params bind typed NULL
- 2026-10-22 · Read-only layers: (2) PG pools start with `default_transaction_read_only=on`, each run `BEGIN READ ONLY` + `SET LOCAL statement_timeout`; MySQL `SET SESSION max_execution_time` (MariaDB fallback `max_statement_time`) + `START TRANSACTION READ ONLY`; SQLite `mode=ro` + `PRAGMA query_only=ON` + a progress handler that interrupts at the deadline; every run rolls back; a client-side timer (timeout + 2 s) backs it up. (3) sqlparser 0.63 (visitor) on the *rewritten* SQL: exactly one `Statement::Query`; any nested non-query statement (data-modifying CTEs), `FOR UPDATE/SHARE`, `SELECT … INTO` rejected. (4) function denylist on the unqualified name, in expressions and FROM table functions (pg_read_file, dblink*, lo_*, nextval/setval, pg_advisory*, query_to_xml, set_config, pg_sleep, SLEEP, BENCHMARK, LOAD_FILE, GET_LOCK, load_extension, writefile, …). (5) row limit (stream stops at limit+1 → `truncated`), cell cap 1 000 chars, masks
- 2026-10-22 · Layers 3–4 also run at save time (API create/update, bundle import, CLI import) against the connection's dialect, plus placeholder ↔ param matching → 422 on `kind.mode.sql` / `kind.mode.params[i]`
- 2026-10-22 · Result to agents: `{columns, rows, truncated}` as JSON text, then the project's shaping (jq, null stripping, size cap) like HTTP. NUMERIC/DECIMAL → string (exact), timestamps → ISO 8601, binary → `"<binary, N bytes>"`, unknown PG types → a note to cast. MySQL TINYINT(1) → boolean. Masked values → `"[masked]"` (NULL stays NULL)
- 2026-10-22 · Column masks (`table.column`, `schema.table.column`, `*.column`, case-insensitive) match result column names when the query reads that table (tables collected from the AST; `users` ≈ `public.users`)
- 2026-10-22 · Test connection (`POST …/db-connections/{id}/test`) never fails: reports ok, version, user, `can_write`, the privileges that allow writing and warnings. PG: superuser, INSERT/UPDATE/DELETE/TRUNCATE via `has_table_privilege` on user tables, CREATE on schemas/database. MySQL: parses `SHOW GRANTS`. SQLite: whether the file is writable. Warns when it can write. PG/MySQL DSNs must be URLs (`postgres://`, `mysql://`) · a SQLite path under the PG driver silently connected to localhost with defaults during browser QA
- 2026-10-22 · `forge_mcp::Executors { http, sql }` replaces the bare `HttpExecutor` in `ForgeMcp`, `serve_stdio`, `AppState.exec`; `ToolRun.outcome` is `RunOutcome::{Http, Sql}`. Test console response gains `kind`; SQL runs report `request {sql, params}`, `response {columns, rows, row_count, truncated, masked_columns, elapsed_ms}`
- 2026-10-22 · SQL tools: annotations `readOnlyHint: true`, `destructiveHint: false`, `openWorldHint: false`; project read-only mode keeps them; path filters ignore them (tags still apply). Imported tools can't switch kind (422 `kind.type`)
- 2026-10-22 · `check_args` moved from exec_http to core (shared by both executors; exec_http re-exports it)
- 2026-10-22 · sqlx `raw_sql` avoided in the executor: its generic `async fn` broke rustc's `Send` proof once awaited inside the rmcp handler; `Executor::execute(&str)` (also unprepared) works · found by bisecting PG/MySQL vs SQLite paths
- 2026-10-22 · Bundle v1 gains optional `db_connections` (DbConnectionInput, DSN as ref); SQL tools reference them by name (`connection: shop`), never by id; identical existing connections are kept, `replace` refuses a driver change
- 2026-10-22 · Deps: `sqlparser` 0.63 (visitor), sqlx features postgres/mysql/json/uuid/rust_decimal/tls-rustls-ring-webpki, `futures-util`, dev `testcontainers-modules` 0.15 (postgres:16-alpine, mysql:8.4)
- 2026-10-22 · Docker tests (`crates/exec_sql/tests/containers.rs`) are `#[ignore = "needs Docker"]`; a dedicated ubuntu CI job runs them with `--ignored`
- 2026-10-22 · UI: Databases tab (list, add/edit, Test with the write-access warning, SELECT-only recommendation), SQL tool editor (connection, SQL with `:name` param sync on blur, typed params, jq shape), SQL rows/query view in the test console; "New SQL tool" route `#/p/{id}/tools/new-sql`. `table_allowlist` is not shown until Stage 9 enforces it
- 2026-10-22 · Exploration tools are stored SQL tools with `SqlMode::ListTables | DescribeTable | RunSql` (no SQL/params; fixed inputs `pattern?`, `table`, `sql`). Opt-in per connection via `POST …/db-connections/{id}/exploration-tools {modes}`: creates `{connection}_{mode}` (deduped `_2…`, tag `database`, empty description), skips modes the connection already has. Disable/delete like any tool · matches the PROMPT IR; no hidden virtual tools
- 2026-10-22 · Descriptions of exploration tools (and of curated queries with no description) are generated at list time from the connection: database name + driver, row/time limits, allowed tables, masked columns. A user description is shown first · stays current when the connection changes. `to_mcp_tool(tool, Option<&DbConnection>)`
- 2026-10-22 · Table allowlist applies to the exploration tools only (curated queries are admin-written). Entries `*`, `schema.*`, `schema.table`, `table` (any schema), case-insensitive; empty = every non-system table. Unqualified references match `schema.…` entries only for the fixed default schema (PG `public`, SQLite `main`; none for MySQL, so use unqualified names there). System catalogs (pg_catalog, information_schema, pg_toast, mysql, performance_schema, sys, unqualified `pg_*` on PG, `sqlite_*`) are always refused in run_sql/describe and hidden from list_tables
- 2026-10-22 · Table references are collected with CTE scoping: a reference to a visible CTE isn't a table, but inside a non-recursive CTE's own body (or earlier siblings) a same-named reference is the real table, and a CTE inside a subquery doesn't hide outer tables · stops `WITH users AS (SELECT * FROM secrets) …`-style allowlist bypasses
- 2026-10-22 · run_sql masks are strict (curated queries keep result-name masking): when a mask applies to a table the query reads (or is `*.col`), the query is refused if any identifier names a masked column (aliases, functions, WHERE, ORDER BY), it uses a whole-row reference (`to_json(u)`), `t.*` inside expressions/function calls, `(expr).*`, or `*` anywhere but the outermost plain SELECT. The outermost `SELECT *` stays allowed and its masked columns come back as `[masked]` by name
- 2026-10-22 · run_sql refuses placeholders ("write values inline"); denylist gained `table_to_xml*`, `schema_to_xml*`, `database_to_xml*`, `cursor_to_xmlschema`, `ts_stat` (read tables by name / run SQL from a string)
- 2026-10-22 · list_tables/describe_table use information_schema (PG, MySQL `DATABASE()` only) or sqlite_master/`pragma_table_info`, run through the same read-only session (≤2 000 catalog rows), then filter by allowlist + system rules. describe_table output: `column, type, nullable, default, primary_key, masked`; a name matching several *visible* schemas asks for `schema.table`; schemas the connection can't read are ignored. Results use the `{columns, rows, truncated}` shape
- 2026-10-22 · Statement classes from the AST: Read (queries), Write (INSERT/UPDATE/DELETE/MERGE, locking reads), Ddl (CREATE/ALTER/DROP/TRUNCATE/RENAME/COMMENT, SELECT INTO), Admin (everything else: GRANT, SET, COPY, PRAGMA, CALL, ATTACH, VACUUM, EXPLAIN, …). `check_statement` returns the class; `check_read_only` = class must be Read (old messages kept)
- 2026-10-22 · Gating (`gate`): ReadOnly → reads only; CuratedWrite → saved queries may write, agent `run_sql` stays read-only; FullAccess → run_sql may write too; Ddl additionally needs `allow_ddl` (only valid with a write mode); Admin never. Checked at save time and at run time (a connection downgraded later makes its write tools fail cleanly)
- 2026-10-22 · Write rules: one top-level statement; any write nested in a query/CTE is refused (it would hide its row count from the cap); UPDATE/DELETE need WHERE (`WHERE true` is the explicit escape), reported only once writes are otherwise allowed so read-only connections still say "only SELECT"
- 2026-10-22 · Writes run in a read-write transaction (PG `BEGIN READ WRITE` overriding the pool's read-only default + `statement_timeout`; MySQL `START TRANSACTION READ WRITE` + `innodb_lock_wait_timeout`, since `max_execution_time` only covers SELECT; SQLite a separate writable pool without `mode=ro`/`query_only`, progress-handler timeout). Affected rows = `rows_affected`, or the count of RETURNING rows (streamed; stops at cap+1). Over `max_affected_rows` → rollback + `TooManyRows`; `dry_run` → always rollback; else commit. A timeout drops the transaction (never commits). MySQL DDL can't dry-run (implicit commit) → refused
- 2026-10-22 · Write-capable tools (curated statements classified Write/Ddl; run_sql under FullAccess) get a `dry_run` boolean argument, `readOnlyHint: false`, `destructiveHint: true`, and a generated note (cap, dry_run). Read-only projects hide them. Result JSON adds `affected_rows`, `committed`, `dry_run` (rows = RETURNING output)
- 2026-10-22 · Audit log (migration 0006, `audit_log`): every write attempt that reached the database — committed, rolled_back (cap), dry_run, failed (DB error/timeout) — with tool, connection, origin (`mcp`/`console`), class, SQL as sent, bound params, rows affected, error. Refused statements (guard/gate/args) aren't logged. Written by `forge_mcp::run_tool` after the run; an insert failure is logged, not surfaced. `GET /api/projects/{id}/audit?limit&before`; cascades with the project; tool ids aren't FKs so entries outlive tools
- 2026-10-22 · FullAccess needs explicit confirmation: API `confirm_full_access: true` when a create/update turns it on (422 on `access_mode` otherwise); bundle import (API flag, CLI `--confirm-full-access`); UI shows a destructive warning with a required checkbox. Connection updates now notify MCP sessions (schemas/annotations depend on the mode)
- 2026-10-22 · SQL params can carry `enum` (UI "Allowed values"); checked by `check_args` before anything runs
- 2026-10-22 · `[profile.dev] debug = "line-tables-only"`, deps `debug = false` · target/ hit 33 GB twice on a small drive; now ~1.6 GB for a build
- 2026-10-22 · Releases: cargo-dist 0.33.0 (six weeks old; 0.33.1 was a day old), config in `dist-workspace.toml`, workflow `.github/workflows/release.yml` (generated; edit the toml and run `dist generate`). Targets: macOS arm64/x64, Linux x64/arm64 (native arm runner), Windows x64 MSVC; shell + PowerShell installers into `~/.cargo/bin`; `profile.dist` = release + thin LTO. The UI build runs first via `github-build-setup = "../build-setup.yml"`. `mcp-forge` has `publish = false`, so `[package.metadata.dist] dist = true` opts it in
- 2026-10-22 · Dockerfile: node:22-bookworm-slim (UI) → rust:1.91-bookworm (release build, BuildKit cache mounts) → debian:bookworm-slim with ca-certificates, user `forge` uid 10001, `/data` volume, `MCP_FORGE_BIND=0.0.0.0:7410` (so an admin token is mandatory), entrypoint `mcp-forge serve`. 201 MB. No keyring in containers → `env:` refs. CI job builds it and checks /healthz, 401 without token, 200 with
- 2026-10-22 · Logs use ANSI colours only when stderr is a terminal (`docker logs` showed escape codes)
- 2026-10-22 · Licence files LICENSE-MIT / LICENSE-APACHE added to match the declared `MIT OR Apache-2.0`; copyright "the mcp-forge authors"
- 2026-10-21 · `.gitattributes` forces LF (esp. `migrations/*.sql`) · sqlx checksums migrations; CRLF conversion on Windows checkouts would break existing DBs
- 2026-10-21 · Timestamps truncated to ms so they round-trip through SQLite exactly

## Open questions
<!-- Append: YYYY-MM-DD · question · temporary choice made -->
- 2026-10-21 · PROMPT says bearer on "/api and UI". Static files can't carry a bearer from a browser navigation. · UI shell (no data) is public; all data goes through /api, which requires the token. Revisit if a cookie session is wanted
- 2026-10-22 · PROMPT types `SqlMode::Query.params` as `Vec<ParamDef>`; HTTP locations/fixed values/wire names don't apply to SQL. · own `SqlParam` type
- 2026-10-22 · PROMPT makes `dsn_ref` a secret ref for every driver, including SQLite file paths (not secret). · kept as refs (`env:SHOP_DB` holding the path) for one consistent rule; revisit if it's annoying
- 2026-10-22 · No git remote was configured, but cargo-dist needs a repository URL. · resolved: the user chose `https://github.com/jaydip187/mcp-forge` (set as `origin`, used in `[workspace.package] repository/homepage` and the README install commands)
- 2026-10-21 · Should tool `description` be required? · optional (empty allowed) for now; importers will fill from summary/description

## Known issues / tech debt
<!-- Append: YYYY-MM-DD · issue · stage to fix -->
- 2026-10-22 · The README quick start was walked through on a fresh container via the REST API the UI calls plus a raw MCP client, not by clicking through the UI (browser-mcp input problems this session). · try the UI flow at review
- 2026-10-22 · The affected-rows cap counts what the database reports for the statement: rows changed by triggers or cascading foreign keys aren't counted, and MySQL reports *changed* (not matched) rows for UPDATE. · document in the Stage 11 security model
- 2026-10-22 · MySQL writes have no server-side statement timeout (max_execution_time is SELECT-only); lock waits are bounded and the client timer rolls back, but a long write keeps running server-side until it finishes or is rolled back. · document
- 2026-10-22 · Audit rows are written after the transaction ends; if the internal DB write fails the change isn't recorded (error logged). · consider write-ahead audit if needed
- 2026-10-22 · UI "writes" badge in the tool list is a keyword heuristic (labels only; the server classifies properly)
- 2026-10-22 · Stage 10 browser pass incomplete: browser-mcp stopped delivering mouse events to the tab (keyboard worked); stopped after two attempts per the browser rules. The connection form's new "What agents may do" control was seen rendering; confirmation/gating/audit behaviour is covered by API tests. · eyeball at review
- 2026-10-22 · Curated queries still mask by result column *name* (an alias escapes it); run_sql uses the strict check. · fine while curated SQL is admin-written
- 2026-10-22 · The allowlist/mask checks are static: a user-defined function that reads other tables, or `ORDER BY 3` on a masked column (ordering side channel), isn't caught. Hard isolation needs a DB role limited to the allowed tables/columns (the Test button shows what the role can do). · document in Stage 11 security model
- 2026-10-22 · Postgres binds are typed (`string` → TEXT), so comparing a string param to a date/uuid column needs a cast in the SQL (`:since::timestamptz`); the editor hint says so. · consider more param types (date, uuid) later
- 2026-10-22 · Privilege check limits: MySQL roles granted to the user aren't expanded (reported as a warning); SQLite only checks file permissions; PgBouncer may reject the `default_transaction_read_only` startup option (the per-run `BEGIN READ ONLY` still applies). · revisit on user reports
- 2026-10-22 · Pools are cached per connection in the process; `serve` drops them on edit/delete, but a CLI bundle import that changes a connection isn't seen by a running `serve` until its DSN/driver/timeout changes. · same as the existing out-of-process note
- 2026-10-21 · Tested with the rmcp client + raw JSON-RPC (protocol 2025-06-18), not the MCP Inspector UI itself (needs npx/network). · manual check at review
- 2026-10-22 · Large-spec test (`large_real_spec`) is `#[ignore]` and reads `MCP_FORGE_LARGE_SPEC`; GitHub's 13 MB spec isn't vendored. Result on 2026-10-22: GitHub REST 3.0.3 and 3.1.0 → 1242/1242 ops supported, unique names, all valid, largest tool 25 KB, ~95 ms (release). · consider a CI job that downloads it
- 2026-10-22 · Executor can't send comma/space/pipe-joined array query values (2.0 default `csv`, 3.x `explode: false`); such params import with a warning and are sent as repeated keys. · add a per-param array style to the IR when a real API needs it
- 2026-10-22 · A runaway jq expression (e.g. `repeat(.)` that never yields) can spin a request thread; only output count is capped, not CPU time. · consider running shaping under a time budget
- 2026-10-22 · Stage 7 UI (Response section, tags field, Curation card, hidden-tool notes, Shaped badge) not checked in a browser: browser-mcp disconnected. · eyeball at review
- 2026-10-22 · CLI `import` (and any other out-of-process write) doesn't notify MCP sessions of a running `serve`; only stdio mode polls the DB. · consider polling in serve too (Stage 7+)
- 2026-10-22 · Stage 6 UI (Specs tab, re-import diff page, overrides note, export/import card) not checked in a browser: browser-mcp still disconnected. Backed by API tests. · eyeball at review
- 2026-10-22 · Stage 5 UI change (copy only) not checked in a browser: browser-mcp disconnected this session. · eyeball at review
- 2026-10-22 · Import preview renders all operations without virtualization (fine at 1242 rows in manual testing). · revisit if slower specs show up
- 2026-10-21 · No browser-level (Playwright) UI tests; UI logic is unit-tested (vitest) and the full flow was verified manually in Chrome. · consider for Stage 11
- 2026-10-21 · Local Windows dev box has no MSVC `link.exe`; builds use `stable-x86_64-pc-windows-gnu` via `rustup override` (not committed). Also, in Git Bash, coreutils `link` shadows MSVC's — run cargo from PowerShell. CI uses default toolchains. · env only

---

## Session log
<!-- Newest first. Template:

### Session N — YYYY-MM-DD
- **Stage/task:** 
- **Done:** 
- **Not done / partial:** 
- **Tests:** passing / failing (which)
- **Gotchas:** 
- **Next step:** 
-->

### Session 13 — 2026-10-22
- **Stage/task:** Stage 11 follow-up: repository set to github.com/jaydip187/mcp-forge (user's choice), pushed `master`, tagged and pushed `v0.1.0`
- **Done:** release run 39769017310 succeeded (plan, 5 platform builds, global artifacts, host, announce); release page has archives + checksums for 5 targets, shell + PowerShell installers, source tarball. Clean-machine test: fresh `debian:bookworm-slim`, README one-liner verbatim → `mcp-forge 0.1.0` in `~/.cargo/bin`, `serve` serves the embedded UI, quick start (project, `get_repo`, console, MCP initialize/list/call) works against the real GitHub API; 24 s total, 16 s of it installing curl in the bare image
- **Not done / partial:** PowerShell installer not run (would install into the user's own profile); CI workflow didn't run on the `master` push (it triggers on `main` and PRs)
- **Next step:** review; consider renaming the branch to `main` so CI runs on pushes

### Session 12 — 2026-10-22
- **Stage/task:** Stage 11 — Packaging & docs; Stage 10 approved ("continue")
- **Done:** cargo-dist setup (dist-workspace.toml, release workflow with UI build steps, package metadata, licence files); Dockerfile + .dockerignore + CI docker smoke job; README (install, 5-minute quick start, clients, secrets, databases, security model, Docker, CLI, troubleshooting, building); plain-text logs when not on a terminal
- **Not done / partial:** installer path untestable without a published release; UI click-through of the quick start
- **Tests:** passing — workspace + clippy + fmt + UI; `dist plan` (5 targets, 2 installers) and `dist generate --check`; Docker image built locally and the README walkthrough run on a fresh container + empty volume: healthy 1 s after `docker run`, non-root uid 10001, 401 without token, project + `get_repo` tool created, test console and MCP initialize → tools/list → tools/call returned shaped GitHub data; ~4 s machine time, so the human part (reading, typing) is what fills the 5 minutes
- **Gotchas:** cargo-dist skips `publish = false` packages unless `dist = true`; newest cargo-dist was a day old → pinned the previous release
- **Next step:** push to GitHub, tag `v0.1.0`, then install with the README one-liner on a clean machine (repository URL set to github.com/jaydip187/mcp-forge after review)

### Session 11 — 2026-10-22
- **Stage/task:** Stage 10 — Writes (all tasks); Stage 9 approved ("continue")
- **Done:** core `AuditEntry`/`AuditOutcome`, write access modes + allow_ddl rule; guard `StatementClass` + `check_statement` (nested-write and missing-WHERE detection, insert targets, RETURNING); session `run_write` per driver (read-write txn, cap, dry run); executor `gate`, `tool_writes`, `tool_input` (dry_run), writable pools, `run` → `SqlRun` with `AuditDraft`; store audit table (migration 0006) + list; MCP destructive annotations, write descriptions, read-only project hiding, audit writes (origin mcp/console); API full-access confirmation, audit endpoint, bundle/CLI confirm flag; UI access-mode controls with full-access warning + confirm, allowed values for params, write badges/console view, Audit tab, import opt-in
- **Not done / partial:** browser pass cut short (browser-mcp mouse input stopped reaching the tab)
- **Tests:** passing — workspace + clippy + fmt; exec_sql lib 20 (classification, WHERE/nesting, gating matrix, save-time gating), sqlite_writes 4 (commit+audit, dry run, cap with/without RETURNING, WHERE + mode gating + DDL/admin), containers 2 (PG + MySQL now also commit / dry run / cap rollback; MySQL DDL dry-run refused), store audit paging, server `writes_are_gated_capped_and_audited` (the "done when": confirmation, save-time rules, annotations + dry_run in schema, cap error text, console vs mcp audit entries, read-only project hiding, upgrade to full access); UI 25
- **Gotchas:** J: filled again (target 33 GB) → debug-info profile change + cargo clean; reporting "needs WHERE" on read-only connections was misleading → WHERE check moved into the gate
- **Next step:** Stage 10 review, then Stage 11 packaging & docs

### Session 10 — 2026-10-22
- **Stage/task:** Stage 9 — Exploration tools + read-only run_sql (all tasks); Stage 8 approved ("continue")
- **Done:** core exploration `SqlMode`s with fixed inputs; guard CTE scoping, allowlist/system-table rules, strict mask check, more denied functions; `explore.rs` (per-driver catalog SQL, filtering, describe output); executor dispatch (`run_query` handles all modes, shared `execute`); MCP generated descriptions from the connection; opt-in endpoint; UI allowlist field, Agent tools panel, exploration tool editor view, SQL textarea in the console
- **Not done / partial:** none
- **Tests:** passing — workspace + clippy + fmt; exec_sql lib 17 (CTE shadowing, allowlist/system, 10 mask bypass shapes, explore filtering); containers 2 (PG 16 + MySQL 8.4 now also cover list/describe/run_sql, schema shadowing, pg_user/pg_roles/mysql.user/information_schema refused); server `exploration_tools_respect_allowlist_and_masks` (the "done when", over MCP); UI 23. Browser QA: set allowlist, added all three agent tools, `lower(email)` refused as masked, a join/aggregate ran
- **Gotchas:** first container run found describe_table calling `users` ambiguous because a non-allowed schema also had `users` → filter by allowlist before the ambiguity check; sqlparser doesn't parse a bare `TABLE x` statement (refused anyway)
- **Next step:** Stage 9 review, then Stage 10 writes

### Session 9 — 2026-10-22
- **Stage/task:** Stage 8 — DB connections + curated read-only queries (all tasks); Stage 7 approved ("continue")
- **Done:** core `db.rs` (DbConnection, ColumnMask, SqlSpec/SqlParam, ToolKind::Sql) + `check_args` moved to core; store `db_connections` (migration 0005), `connection_ref`, bundle connections; `forge-exec-sql` (guard: placeholders + AST + denylist; session: per-driver read-only runs; value conversion; executor with pool cache, masks, redaction, test_connection); MCP `Executors` + SQL dispatch/annotations; API connection CRUD/test + save-time SQL checks; bundle export/import of connections; UI Databases tab, SQL tool editor, SQL test console view; CI job for Docker tests
- **Not done / partial:** none
- **Tests:** see the final run in this session's report; new: core +3, exec_sql unit 13, SQLite integration 8, containers 2 (PG 16, MySQL 8.4 — run locally with Docker, pass), store +2, server SQL 2, UI +5 (22 total). Browser QA (browser-mcp): added a SQLite connection, Test showed the write warning, created a SQL tool (param synced from `:customer_id`), test console showed 2 rows with email masked, saving a `WITH … DELETE` was refused
- **Gotchas:** J: drive filled up (target/ was 33 GB) → `cargo clean`; sqlx `raw_sql` breaks `Send` inside rmcp handlers; Radix Select only opens from the keyboard in browser-mcp (focus + Enter)
- **Next step:** Stage 8 review, then Stage 9 exploration tools + read-only run_sql

### Session 8 — 2026-10-22
- **Stage/task:** Stage 7 — Curation & shaping (all tasks)
- **Done:** `forge_exec_http::shape` (jaq apply/check, strip_nulls, truncate, shape_body); core `ProjectSettings` + `path_matches` + tool `tags`; store settings/tags (migration 0004); MCP filters, shaping in `run_tool`, `search_tools`/`call_tool`; API settings endpoint + shape validation + `shaped` in test console; UI Response section, tags, Curation card, hidden-tool notes, Shaped badge
- **Not done / partial:** none
- **Tests:** passing (shape 10, settings 3, search ranking 1, server shaping 5 incl. the "done when" console test + filters + tool search over MCP; UI 17); clippy + fmt clean. No browser pass (browser-mcp disconnected)
- **Gotchas:** bash heredocs containing apostrophes broke under the tool harness twice → write patch scripts to the scratchpad instead
- **Next step:** Stage 7 review, then Stage 8 (DB connections + curated read-only queries)

### Session 7 — 2026-10-22
- **Stage/task:** Stage 6 — Re-import, overrides, export/import (all tasks)
- **Done:** core `changed_fields`/`merge_overrides` + `Source::spec_id/op_id`; store override tracking, `reset_overrides`, spec options (migration 0003), `apply_reimport`, `tools_for_spec`, `import_bundle` with conflict policies; server re-import preview/commit, override reset, bundle export/import endpoints, `export`/`import` CLI; UI Specs tab, re-import diff page, imported-tool note with reset, export/import card
- **Not done / partial:** none
- **Tests:** passing (core overrides 3, store +4, server re-import 2 incl. the "done when" flow, bundle 2 incl. CLI round trip + 2 unit, config +1; UI 15); clippy + fmt clean. No browser pass (browser-mcp disconnected)
- **Gotchas:** re-import must not add overrides (system update path `write_tool`, not `update_tool`)
- **Next step:** Stage 6 review, then Stage 7 curation & shaping

### Session 6 — 2026-10-22
- **Stage/task:** Stage 5 — Swagger 2.0 importer (all tasks)
- **Done:** `swagger2::to_openapi3` converter + `import_any`; spec kind + `Source::Swagger`; delimited-array warning in the 3.x importer; server uses `import_any`; UI copy covers Swagger 2.0
- **Not done / partial:** none
- **Tests:** passing (importer +7: converter units, Petstore 2.0 ↔ 3.0 equivalence, details, csv warning, YAML/basePath; server +1 Swagger end-to-end); UI 14; clippy + fmt clean
- **Gotchas:** the published Petstore 2.0 and 3.0 specs differ on 4 operations (form vs query, multipart vs octet-stream, array vs string `status`, required vs optional login params) — verified against the fixtures and excluded from the equivalence check with reasons
- **Next step:** Stage 5 review, then Stage 6 override tracking + re-import

### Session 5 — 2026-10-22
- **Stage/task:** Stage 4 — OpenAPI 3.x importer (all tasks)
- **Done:** `ParamDef.wire_name` (core + executor + UI type); `forge-importers` (JSON/YAML parse, $ref resolver, OpenAPI 3.0/3.1 → ToolInput with naming/dedupe/flattening/warnings, security scheme summary); migration `0002_specs` + transactional `Store::import_tools`; preview/commit/list-specs endpoints with URL fetch; Import page (file or URL, base URL, auth, tag/method/glob filters, selection)
- **Not done / partial:** none
- **Tests:** passing (Rust 98 + 1 ignored large-spec test, run manually on GitHub 3.0 + 3.1; UI 14); clippy + fmt clean; browser QA via browser-mcp: Petstore fetched by URL, filtered to /store/**, 4 tools imported; GitHub spec previewed and glob-filtered in the UI
- **Gotchas:** in a `$ref` cycle the stub appears at the first self-reference, not one level deeper. axum's default 2 MB JSON limit would reject real specs → per-route `DefaultBodyLimit`
- **Next step:** Stage 4 review, then Stage 5 Swagger 2.0

### Session 4 — 2026-10-21
- **Stage/task:** Stage 3 — Embedded UI (all tasks)
- **Done:** shadcn/Tailwind React UI (projects, project tabs Tools/Auth/Connect/Settings, tool editor with request-line preview, params editor with fixed values + per-param JSON schema, body schema, auth configs, test console with Agent sees/Response/Request, connect snippets, token unlock); rust-embed serving + CSP; security guard (loopback Host / admin token); info, MCP-view and test endpoints; CI builds UI before Rust
- **Not done / partial:** none
- **Tests:** passing (Rust 77 incl. 5 new stage3 + security/config units; UI vitest 13); clippy + fmt clean; manual browser run: created project + tool in UI, ran it in the console (200), listed + called it over MCP JSON-RPC
- **Gotchas:** rust-embed only re-embeds when the crate recompiles → build.rs. Browser QA found prefilled `https://`/`/` defaults got appended to; now empty with placeholders
- **Next step:** Stage 3 review, then Stage 4 OpenAPI importer

### Session 3 — 2026-10-21
- **Stage/task:** Stage 2 — MCP server + HTTP executor (all tasks)
- **Done:** schema derivation + arg map (core); `forge-exec-http` executor (args check, path/query/header/body, auth via env/keyring refs, timeouts, redaction, same-host redirects, 5 MiB cap); `forge-mcp` ServerHandler (list/call, annotations, LLM-friendly errors, list_changed for both protocol generations, stdio + store polling); `/mcp/{slug}` streamable HTTP; REST publishes changes
- **Not done / partial:** none
- **Tests:** passing (69: core 24, exec_http 13, store 14, server 18 incl. MCP-over-HTTP e2e ×4 and stdio-binary e2e ×2); clippy + fmt clean; raw JSON-RPC smoke against the binary OK
- **Gotchas:** rmcp 3.x structs are `#[non_exhaustive]` → use builders (`StreamableHttpServerConfig::default().with_…`). Unknown tools must be protocol errors per rmcp docs, everything else `CallToolResult::error`
- **Next step:** Stage 2 review, then Stage 3 UI

### Session 2 — 2026-10-21
- **Stage/task:** Stage 1 — IR + store + REST API (all tasks)
- **Done:** `forge-core` IR (Project, ToolDef/ToolInput, HttpSpec, ParamDef, AuthConfig, SecretRef, Source, Overrides) with field-level validation; migration `0001`; store repos with conflict/not-found mapping, cross-project auth check, cascade delete; `/api` CRUD for projects, tools, auth-configs with JSON errors
- **Not done / partial:** none
- **Tests:** passing (45: core 20, store 13, server config 4 + api 7 + healthz 1); clippy + fmt clean; manual smoke via real binary OK
- **Gotchas:** sqlite FK `RESTRICT` would break project cascade (auth_configs deleted before tools) → use `NO ACTION`
- **Next step:** Stage 1 review, then Stage 2 rmcp ServerHandler

### Session 1 — 2026-10-21
- **Stage/task:** Stage 0 — Scaffold (all tasks)
- **Done:** workspace with 7 crates; clap CLI (`serve`, `stdio`); tracing (stderr, `RUST_LOG`); config precedence; `Store::open` creates data dir + DB and runs migrations; `/healthz`; GitHub Actions CI (fmt, clippy -D warnings, test on linux/windows/macos)
- **Not done / partial:** `stdio` is a stub until Stage 2
- **Tests:** passing (8: config parsing ×4, store ×3, healthz integration ×1); clippy + fmt clean; manual smoke test of `serve` OK
- **Gotchas:** no MSVC linker locally → GNU toolchain override; don't run cargo from Git Bash
- **Next step:** Stage 0 review, then Stage 1 IR types + validation
