# mcp-forge — Progress Tracker

> Read at the start of every session, update at the end. See `PROMPT.md §1` for the session protocol.
> Legend: `[ ]` todo · `[~]` in progress / partial · `[x]` done · `[-]` dropped (say why)

## Next up
Stage 0 review → then Stage 1: IR types + validation in `crates/core`

## Current stage
**Stage 0 — Scaffold** · status: done · review: requested

---

## Stage checklist

### Stage 0 — Scaffold
- [x] Cargo workspace + crate skeletons (core, store, importers, exec_http, exec_sql, mcp, server)
- [x] clap CLI: `serve`, `stdio`
- [x] tracing + config loading (CLI > env > defaults)
- [x] SQLite init + sqlx migrations runner
- [x] `/healthz`
- [x] CI: fmt, clippy -D warnings, test
- [ ] Stage review approved

### Stage 1 — IR + store + REST API
- [ ] IR types + validation
- [ ] Migrations: projects, tools, auth_configs
- [ ] Repository layer
- [ ] REST CRUD under `/api`
- [ ] Integration tests
- [ ] Stage review approved

### Stage 2 — MCP server + HTTP executor
- [ ] rmcp ServerHandler (list_tools / call_tool) from store
- [ ] Schema derivation + arg location map
- [ ] Streamable HTTP `/mcp/{slug}`
- [ ] stdio mode
- [ ] HTTP executor + auth via secret refs (env:, keyring:)
- [ ] LLM-friendly errors
- [ ] `tools/list_changed` notifications
- [ ] wiremock + MCP client tests
- [ ] Stage review approved

### Stage 3 — Embedded UI (manual flow)
- [ ] Vite React TS app + rust-embed
- [ ] Projects / tools / auth screens
- [ ] Tool form (params table, body schema, fixed values)
- [ ] Test console
- [ ] Endpoint view + client config snippets
- [ ] Loopback default + admin token for non-loopback
- [ ] Stage review approved

### Stage 4 — OpenAPI 3.x importer
- [ ] File / URL import
- [ ] $ref resolution (depth cap, circular)
- [ ] Naming + dedupe + collision prefixing
- [ ] Preview + filter UI
- [ ] Raw spec storage
- [ ] Tests (Petstore + large real spec)
- [ ] Stage review approved

### Stage 5 — Swagger 2.0 importer
- [ ] 2.0 → IR conversion
- [ ] Preview flow reuse
- [ ] Equivalence test vs 3.x Petstore
- [ ] Stage review approved

### Stage 6 — Re-import, overrides, export/import
- [ ] Override tracking
- [ ] Re-import with diff preview
- [ ] YAML export/import (CLI + UI), secrets as refs only
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

## Open questions
<!-- Append: YYYY-MM-DD · question · temporary choice made -->
- 2026-10-21 · What should `mcp-forge stdio` do before Stage 2? · opens the store then exits with a clear "not implemented yet (Stage 2)" error

## Known issues / tech debt
<!-- Append: YYYY-MM-DD · issue · stage to fix -->
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

### Session 1 — 2026-10-21
- **Stage/task:** Stage 0 — Scaffold (all tasks)
- **Done:** workspace with 7 crates; clap CLI (`serve`, `stdio`); tracing (stderr, `RUST_LOG`); config precedence; `Store::open` creates data dir + DB and runs migrations; `/healthz`; GitHub Actions CI (fmt, clippy -D warnings, test on linux/windows/macos)
- **Not done / partial:** `stdio` is a stub until Stage 2
- **Tests:** passing (8: config parsing ×4, store ×3, healthz integration ×1); clippy + fmt clean; manual smoke test of `serve` OK
- **Gotchas:** no MSVC linker locally → GNU toolchain override; don't run cargo from Git Bash
- **Next step:** Stage 0 review, then Stage 1 IR types + validation
