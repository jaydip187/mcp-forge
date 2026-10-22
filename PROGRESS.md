# mcp-forge — Progress Tracker

> Read at the start of every session, update at the end. See `PROMPT.md §1` for the session protocol.
> Legend: `[ ]` todo · `[~]` in progress / partial · `[x]` done · `[-]` dropped (say why)

## Next up
Stage 6 review → then Stage 7: jaq response shaping

## Current stage
**Stage 6 — Re-import, overrides, export/import** · status: done · review: requested

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
- [ ] Stage review approved

### Stage 7 — Curation & shaping
- [ ] jaq response shaping
- [ ] Null stripping + size truncation
- [ ] Project filters (GET-only, tag/path)
- [ ] `search_tools` meta-tool
- [ ] Stage review approved

### Stage 8 — DB connections + curated read-only queries
- [ ] DbConnection CRUD + UI
- [ ] Test connection + privilege check
- [ ] Curated Query tools (bound params)
- [ ] Read-only session per driver
- [ ] AST check (single SELECT/WITH, full tree walk)
- [ ] Function denylist
- [ ] Row limit, timeout, cell cap, column masks
- [ ] SQLite → Postgres → MySQL
- [ ] testcontainers tests per layer
- [ ] Stage review approved

### Stage 9 — Exploration + read-only run_sql
- [ ] list_tables / describe_table / run_sql (opt-in)
- [ ] Table allowlist + masks
- [ ] Generated descriptions + readOnlyHint
- [ ] Stage review approved

### Stage 10 — Writes
- [ ] Statement classifier (Read/Write/DDL/Admin)
- [ ] AccessMode gating + allow_ddl
- [ ] Curated write tools
- [ ] Transaction + affected-rows cap rollback
- [ ] dry_run
- [ ] WHERE required on UPDATE/DELETE
- [ ] destructiveHint annotations
- [ ] Audit log + UI
- [ ] FullAccess confirmation UX
- [ ] Stage review approved

### Stage 11 — Packaging & docs
- [ ] cargo-dist releases
- [ ] Dockerfile
- [ ] README + security model
- [ ] Fresh-install test
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
- 2026-10-21 · `.gitattributes` forces LF (esp. `migrations/*.sql`) · sqlx checksums migrations; CRLF conversion on Windows checkouts would break existing DBs
- 2026-10-21 · Timestamps truncated to ms so they round-trip through SQLite exactly

## Open questions
<!-- Append: YYYY-MM-DD · question · temporary choice made -->
- 2026-10-21 · PROMPT says bearer on "/api and UI". Static files can't carry a bearer from a browser navigation. · UI shell (no data) is public; all data goes through /api, which requires the token. Revisit if a cookie session is wanted
- 2026-10-21 · Should tool `description` be required? · optional (empty allowed) for now; importers will fill from summary/description

## Known issues / tech debt
<!-- Append: YYYY-MM-DD · issue · stage to fix -->
- 2026-10-21 · Tested with the rmcp client + raw JSON-RPC (protocol 2025-06-18), not the MCP Inspector UI itself (needs npx/network). · manual check at review
- 2026-10-22 · Large-spec test (`large_real_spec`) is `#[ignore]` and reads `MCP_FORGE_LARGE_SPEC`; GitHub's 13 MB spec isn't vendored. Result on 2026-10-22: GitHub REST 3.0.3 and 3.1.0 → 1242/1242 ops supported, unique names, all valid, largest tool 25 KB, ~95 ms (release). · consider a CI job that downloads it
- 2026-10-22 · Executor can't send comma/space/pipe-joined array query values (2.0 default `csv`, 3.x `explode: false`); such params import with a warning and are sent as repeated keys. · add a per-param array style to the IR when a real API needs it
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
