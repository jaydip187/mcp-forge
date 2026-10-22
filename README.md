# mcp-forge

Turn HTTP APIs and SQL databases into **curated MCP tools** for Claude and other AI agents, from one
self-hosted binary with a web UI.

- **HTTP APIs**: describe requests by hand, or import an OpenAPI 3.x / Swagger 2.0 spec, then rename,
  trim and reshape what agents see (jq).
- **Databases** (SQLite, PostgreSQL, MySQL): saved queries with typed parameters, optional schema
  exploration and agent-written SQL, all read-only by default, with writes as a deliberate opt-in.
- **Safe by default**: listens on localhost only, stores secrets as references (never values), and
  checks every SQL statement before it reaches your database.
- Serves each project over **MCP streamable HTTP** (`/mcp/<project>`) and **stdio**.

Everything runs on your machine. mcp-forge only connects to the APIs and databases you configure,
and to spec URLs you import on purpose. No telemetry.

---

## Contents

- [Install](#install)
- [Quick start (5 minutes)](#quick-start-5-minutes)
- [Connect Claude and other MCP clients](#connect-claude-and-other-mcp-clients)
- [Secrets](#secrets)
- [Databases](#databases)
- [Security model](#security-model)
- [Docker](#docker)
- [CLI reference](#cli-reference)
- [Troubleshooting](#troubleshooting)
- [Building from source](#building-from-source)

---

## Install

**macOS / Linux**

```sh
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/jaydip187/mcp-forge/releases/latest/download/mcp-forge-installer.sh | sh
```

**Windows (PowerShell)**

```powershell
powershell -ExecutionPolicy Bypass -c "irm https://github.com/jaydip187/mcp-forge/releases/latest/download/mcp-forge-installer.ps1 | iex"
```

The installers put `mcp-forge` in `~/.cargo/bin`; open a new terminal afterwards so it's on your
`PATH`. Prefer a manual install? Download the archive for your platform from the
[releases page](https://github.com/jaydip187/mcp-forge/releases), unpack it and put the binary
anywhere on your `PATH`. Builds exist for macOS (Apple Silicon and Intel), Linux (x86_64 and arm64)
and Windows (x86_64).

Check it works:

```sh
mcp-forge --version
```

Docker users: see [Docker](#docker).

## Quick start (5 minutes)

### 1. Start the server

```sh
mcp-forge serve
```

Open <http://127.0.0.1:7410/>. Data lives in `~/.mcp-forge` (change it with `--data-dir`).

### 2. Create a project

Click **+** next to *Projects*, name it `github`. A project is one MCP server: agents connect to
`http://127.0.0.1:7410/mcp/github` and see that project's tools.

### 3. Add a tool

Click **New HTTP tool** and fill in:

| Field       | Value                                     |
|-------------|-------------------------------------------|
| Name        | `get_repo`                                |
| Description | `Look up a GitHub repository: stars, language, open issues.` |
| Method      | `GET`                                     |
| Base URL    | `https://api.github.com`                  |
| Path        | `/repos/{owner}/{repo}`                   |

Leave the path field: `owner` and `repo` appear as required parameters. Optionally add a
**Response shape** so the agent gets only what it needs:

```jq
{name: .full_name, stars: .stargazers_count, language, open_issues: .open_issues_count}
```

Click **Create tool**, then try it in the **Test console** on the right (`owner` = `rust-lang`,
`repo` = `rust`). *Agent sees* is exactly what an agent will get back.

> Have an OpenAPI or Swagger spec? Use **Import API spec** on the Tools tab instead: paste a URL or
> upload a file, filter the operations, and import the ones you want.

### 4. Connect Claude

Open the project's **Connect** tab and copy a snippet, for example for Claude Code:

```sh
claude mcp add --transport http github http://127.0.0.1:7410/mcp/github
```

Ask Claude *"How many stars does rust-lang/rust have?"* and it calls `get_repo`.

That's it. The rest of this README covers the details.

## Connect Claude and other MCP clients

Every project is served two ways. The **Connect** tab shows ready-to-copy versions of all of these.

**Streamable HTTP** (the server must be running):

```sh
claude mcp add --transport http <project> http://127.0.0.1:7410/mcp/<project>
```

```json
{
  "mcpServers": {
    "<project>": { "type": "http", "url": "http://127.0.0.1:7410/mcp/<project>" }
  }
}
```

**stdio** (the client starts mcp-forge itself; no server needed, edits made in a running UI are
picked up within a couple of seconds):

```sh
claude mcp add <project> -- mcp-forge stdio --project <project>
```

```json
{
  "mcpServers": {
    "<project>": { "command": "mcp-forge", "args": ["stdio", "--project", "<project>"] }
  }
}
```

The JSON form works for Claude Desktop (`claude_desktop_config.json`), `.mcp.json` files and most
other MCP clients. If you started the server with an admin token, HTTP clients must also send
`Authorization: Bearer <token>` (the Connect tab adds the header for you).

Tool changes are pushed to connected clients (`notifications/tools/list_changed`), so you can keep
editing while an agent is connected.

## Secrets

mcp-forge never stores secret values. Auth configs and database connections hold a **reference**,
resolved each time a tool runs:

| Reference      | Where the value comes from                                                        |
|----------------|-----------------------------------------------------------------------------------|
| `env:NAME`     | Environment variable `NAME` of the mcp-forge process (or of the stdio client).    |
| `keyring:NAME` | Your OS keychain: service `mcp-forge`, account `NAME` (macOS Keychain, Windows Credential Manager, Secret Service on Linux). |

Anything else, such as a pasted token, is rejected. Secret values are redacted from errors, URLs,
the test console and the audit log. `tools.yaml` exports contain only the references, so they're
safe to share.

Example: a GitHub token for the quick-start project.

```sh
export GITHUB_TOKEN=ghp_...        # PowerShell: $env:GITHUB_TOKEN = "ghp_..."
mcp-forge serve
```

Then on the **Auth** tab add *Bearer token* with reference `env:GITHUB_TOKEN`, and pick it in the
tool's *Auth* field.

## Databases

Add a connection on the **Databases** tab. The connection string is a secret reference too:

| Database   | Value of the referenced variable                         |
|------------|----------------------------------------------------------|
| PostgreSQL | `postgres://readonly:PASSWORD@db.example.com:5432/shop`  |
| MySQL      | `mysql://readonly:PASSWORD@db.example.com:3306/shop`     |
| SQLite     | `/path/to/shop.db`                                       |

Click **Test** to check the connection: it shows the server version, the login, and whether that
login can change data.

**Saved queries** (*New SQL tool*): one statement with `:name` placeholders, for example
`SELECT id, status, total FROM orders WHERE customer_id = :customer_id ORDER BY id DESC`. Each
placeholder becomes a typed argument (string, integer, number or boolean, optionally a list of
allowed values). Values are always sent separately from the SQL, never pasted into it. Agents get
`{columns, rows, truncated}`.

**Agent tools** (opt-in per connection, *Agent tools* on the Databases tab):

- `<db>_list_tables`: tables and views the agent may use
- `<db>_describe_table`: columns, types, keys, and which columns are masked
- `<db>_run_sql`: one statement the agent writes itself

They only see the tables you list under *Tables agents may explore* (empty means every non-system
table). System catalogs are always off limits.

**Masked columns** (`users.password_hash`, `*.email`): their values come back as `[masked]`. In
agent-written SQL they can't be selected, filtered on or used in expressions at all.

**Access modes** (*What agents may do*):

| Mode          | Saved queries         | Agent `run_sql`        |
|---------------|-----------------------|------------------------|
| Read only (default) | read             | read                   |
| Saved writes  | read and write        | read                   |
| Full access   | read and write        | read and write         |

Schema changes (CREATE, ALTER, DROP, TRUNCATE) additionally need *Allow schema changes*. Admin
statements (GRANT, SET, COPY, PRAGMA, CALL, …) never run. Full access must be confirmed explicitly.
Every write:

- runs in a transaction and is **undone if it changes more rows than allowed** (*Most rows one
  write may change*, default 100);
- needs a `WHERE` clause on UPDATE and DELETE (`WHERE true` if you really mean every row);
- can be tried first with `dry_run: true`, which reports the row count and undoes the change;
- is listed on the **Audit** tab: tool, agent or test console, SQL, values, rows, and whether it was
  committed, undone or failed.

## Security model

mcp-forge gives AI agents real access to your APIs and data, so it is built to fail closed.

**Network**

- Listens on `127.0.0.1:7410` by default. Requests must carry a loopback `Host` header, which blocks
  DNS-rebinding attacks from web pages you visit.
- Listening on any other address (`--bind 0.0.0.0:7410`) requires an admin token (16+ characters,
  `MCP_FORGE_ADMIN_TOKEN` or `--admin-token`). It is checked on `/api` and `/mcp`. The UI asks for it
  once per browser tab. `/healthz` stays open. Put TLS in front (a reverse proxy) if the server is
  reachable from other machines.

**Secrets**: references only (see [Secrets](#secrets)); raw values are refused, redacted in output,
and never exported.

**HTTP tools**: agent values are percent-encoded into paths and queries (no `/`, `?`, `#` or `..`
tricks), redirects are followed only to the same host so credentials never leave it, and responses
are size-capped.

**SQL**: independent layers, each enough on its own to stop a write on a read-only connection:

1. **Database login.** Use a login that may only SELECT (the Test button tells you what yours can do).
2. **Read-only session.** Reads run in a read-only transaction with a server-side time limit:
   PostgreSQL `BEGIN READ ONLY` + `statement_timeout`, MySQL `START TRANSACTION READ ONLY` +
   `max_execution_time`, SQLite opened read-only with `query_only`. Everything is rolled back.
3. **Statement check.** The SQL is parsed. Exactly one statement is allowed; it's classified as
   read, write, schema change or admin, and checked against the access mode. Writes hidden in CTEs
   are refused.
4. **Function blocklist.** Functions that write, read server files, reach other servers, sleep or
   take locks (`pg_read_file`, `dblink`, `lo_*`, `nextval`, `SLEEP`, `LOAD_FILE`, `load_extension`,
   …) are refused.
5. **Limits.** Row limit, time limit, a 1,000-character cap per cell, column masks, table allowlist,
   and for writes the row cap, dry run and audit log.

Know the limits:

- Checks 3–5 read the SQL text. A custom database function that reads other tables, or sorting by a
  masked column's position (`ORDER BY 3`), can get around the allowlist or masks. For hard isolation, give the login access
  only to the tables and columns agents should see.
- The write row cap counts what the database reports for the statement. Rows changed by triggers or
  cascading deletes aren't counted, and MySQL counts rows actually changed, not rows matched.
- MySQL has no server-side time limit for writes. A write that runs too long is rolled back by
  mcp-forge, but the database keeps working on it until that finishes.
- Saved queries mask by result column name, so an alias in SQL you write yourself can show a masked
  column. Agent-written SQL is checked strictly.

**Project curation**: a project can be put in read-only mode (only GET/HEAD/OPTIONS tools and SQL
tools that don't write are visible), filtered by tag or path, and can offer two tools,
`search_tools` and `call_tool`, instead of listing hundreds.

## Docker

```sh
docker build -t mcp-forge .
docker run -d --name mcp-forge -p 127.0.0.1:7410:7410 \
  -v mcp-forge-data:/data \
  -e MCP_FORGE_ADMIN_TOKEN="$(openssl rand -hex 24)" \
  mcp-forge
docker logs mcp-forge          # "mcp-forge listening …"
```

- Runs as an unprivileged user (uid 10001). Data lives in the `/data` volume.
- The container listens on `0.0.0.0` inside, so an admin token is required. Read it back with
  `docker exec mcp-forge printenv MCP_FORGE_ADMIN_TOKEN` and paste it when the UI asks.
- Pass secrets as environment variables (`-e GITHUB_TOKEN=…`) and reference them as `env:NAME`.
  There is no OS keychain inside a container.
- Clients connect over HTTP with `Authorization: Bearer <token>`. stdio needs the binary on the
  client's machine.

## CLI reference

```text
mcp-forge serve  [--bind 127.0.0.1:7410] [--admin-token T] [--data-dir DIR]
mcp-forge stdio  --project SLUG [--data-dir DIR]
mcp-forge export --project SLUG [-o tools.yaml]
mcp-forge import --project SLUG [FILE|-] [--on-conflict fail|skip|replace] [--confirm-full-access]
```

| Variable                | Same as          |
|-------------------------|------------------|
| `MCP_FORGE_BIND`        | `--bind`         |
| `MCP_FORGE_ADMIN_TOKEN` | `--admin-token`  |
| `MCP_FORGE_DATA_DIR`    | `--data-dir`     |
| `MCP_FORGE_PROJECT`     | `stdio --project`|
| `RUST_LOG`              | log level, e.g. `info,mcp_forge=debug` (logs go to stderr) |

`tools.yaml` (from `export`, or *Settings → Download tools.yaml*) holds a project's tools, auth
configs and database connections, with secrets as references only. Import it into another project
or machine with `import` or *Settings → Import tools.yaml*. Imported tools become manual tools.

The server also exposes `GET /healthz` and a REST API under `/api` (the UI uses it).

## Troubleshooting

- **"Configuration error: … environment variable `X` is not set"**: the variable must be set in the
  environment of the process running the tool: the `serve` process, or for stdio the MCP client that
  starts mcp-forge.
- **A client can't connect to `/mcp/<project>`**: check the project slug on the Connect tab, that
  `mcp-forge serve` is running, and, with an admin token, that the client sends the `Authorization`
  header.
- **Tools don't show up**: they may be turned off, or hidden by the project's filters (Settings).
  The Tools tab says why next to each hidden tool.
- **Postgres says "cannot execute … in a read-only transaction"**: that's layer 2 doing its job.
  Allow writes on the connection if this tool should write.
- **"binding to non-loopback address … requires --admin-token"**: listening beyond localhost (as the
  Docker image does) needs `MCP_FORGE_ADMIN_TOKEN`. Set one, or bind to `127.0.0.1`.
- **Port 7410 in use**: `mcp-forge serve --bind 127.0.0.1:7420`.

## Building from source

Needs a recent stable Rust (developed with 1.91) and Node 22.

```sh
npm --prefix ui ci
npm --prefix ui run build      # the UI is embedded into the binary
cargo build --release          # target/release/mcp-forge
```

Without the UI build the binary still works and serves a "UI not built" page. Tests:
`cargo test --workspace` and `npm --prefix ui test`. The PostgreSQL and MySQL tests need Docker:
`cargo test -p forge-exec-sql --test containers -- --ignored`.

Releases are built by [cargo-dist](https://opensource.axo.dev/cargo-dist/): pushing a tag like
`v0.1.0` runs `.github/workflows/release.yml`.

## License

MIT or Apache-2.0, at your option.
