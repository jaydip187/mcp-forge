# mcp-forge

Turn HTTP APIs and SQL databases into **curated MCP tools** for Claude and other AI agents, from one
self-hosted binary with a web UI.

- **HTTP APIs**: describe requests by hand, or import an OpenAPI 3.x / Swagger 2.0 spec, a Postman
  collection, a HAR file or curl commands, then rename, trim and reshape what agents see (jq).
- **GraphQL APIs**: write a query or mutation, or import a schema (introspection or SDL) and pick
  the operations to offer; the variables become the agent's arguments.
- **Databases** (PostgreSQL, MySQL, SQL Server, SQLite, ClickHouse): saved queries with typed
  parameters, optional schema exploration and agent-written SQL, all read-only by default, with
  writes as a deliberate opt-in.
- **NoSQL** (MongoDB, Redis, DynamoDB, Elasticsearch): saved operations with typed parameters and
  the same opt-in exploration tools, read-only by default.
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
- [OAuth2 APIs](#oauth2-apis)
- [GraphQL APIs](#graphql-apis)
- [Databases](#databases)
- [NoSQL databases](#nosql-databases)
- [Resources and prompts](#resources-and-prompts)
- [Chained tools](#chained-tools)
- [Approvals](#approvals)
- [Call log](#call-log)
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

On first run there is no account yet. The server prints a one-time **setup code** in its log
(`no accounts yet: … setup code ABCD-EFGH-JKLM`); enter it in the browser with a username and
password to create the first account. After that the UI asks you to sign in.

### 2. Create a project

Click **+** next to *Projects*, name it `github`. A project is one MCP server: agents connect to
`http://127.0.0.1:7410/mcp/github` and see that project's tools.

### 3. Add a tool

Click **New tool → HTTP request** and fill in:

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

> Have an OpenAPI or Swagger spec, a Postman collection, a HAR file or a few curl commands? Use
> **Import** on the Tools tab instead: upload a file, paste text or give a URL, filter the
> operations, and import the ones you want.

### Importing Postman, HAR and curl

These are converted to an OpenAPI document first, then imported like a spec (preview, filters,
base URL, auth, re-import):

- **Postman collections** (v2.0/v2.1): folders become tags, request names become tool names,
  collection variables with values are filled in (`{{baseUrl}}`), `:id` and unresolved
  `{{var}}` path segments become parameters, and auth blocks become suggested auth.
- **HAR files** (browser dev tools → Network → *Save all as HAR*): only API calls are kept (JSON
  responses, or anything but GET); repeats of the same endpoint merge, and id-like path segments
  (`/users/42`, UUIDs) become parameters.
- **curl commands**: paste one or several (bash `\`, cmd `^` and PowerShell `` ` `` line breaks
  work). `-X`, `-H`, `-d`/`--data*`, `--json`, `-G`, `-u` and `-F` are understood.

What never carries over: cookies, `Authorization` and other secret-looking headers or query keys
(they become suggested auth configs instead), and example values from request bodies (only their
shape is kept). mcp-forge stores the converted document, not your original file.

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

### Access keys

Rather than sharing the admin token, give each client or teammate its own key: **Connect → Access
keys → New key**. A key belongs to one project, can be limited to some of its tools, and can
expire (30 days, 90 days, a year, or never). It's shown once (`mcpf_…`); mcp-forge keeps only its
SHA-256. Clients send it as `Authorization: Bearer mcpf_…`; deleting the key cuts them off.

- On a server that listens on the network, a project key is accepted on that project's `/mcp`
  instead of the admin token.
- On localhost, `/mcp` stays open unless you switch on **Require a key on this machine too**.
- A key limited to some tools only lists and calls those; others look like unknown tools.
- The Calls tab shows which key each call came in with, and the key list shows when each was last
  used.

### Sign-in for apps (claude.ai connectors and others)

Apps that support MCP's sign-in (OAuth 2.1), such as claude.ai custom connectors or MCP Inspector,
need only the project URL (`https://your-host/mcp/<project>`). On first connect they open a page on
mcp-forge where you sign in and **Allow** the app for that one project. Behind the scenes:

- `/mcp` answers 401 with a pointer to `/.well-known/oauth-protected-resource/mcp/<project>`, which
  names mcp-forge itself as the authorization server (`/.well-known/oauth-authorization-server`).
- Apps register themselves (`/oauth/register`), then use authorization code + PKCE (S256 only)
  against `/oauth/authorize` and `/oauth/token`. Access tokens last an hour; refresh tokens 30 days
  and are replaced on every use. `/oauth/revoke` revokes. Only hashes of tokens are stored.
- A sign-in covers one project and every tool agents can see in it; tools that require approval
  still wait. **Connect → Connected apps** lists who approved what and disconnects apps.
- The app has to reach the server, so this is for servers behind an HTTPS reverse proxy (start
  with `--admin-token`, and have the proxy send `X-Forwarded-Proto: https`). Addresses in the
  metadata come from the request's `Host` header.

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

## OAuth2 APIs

For APIs that hand out tokens through OAuth2 (Google, Microsoft, Salesforce, Auth0, …), add an auth
config of one of these types. The **client secret** is a reference like any other secret.

- **OAuth2: sign in (authorization code)**, for acting as a person. Fill in the provider's
  authorization URL, token URL, client ID and scopes, and register the redirect URL the form shows
  (`http://127.0.0.1:7410/oauth/callback` by default) with the provider. Save, then click
  **Connect** in the list: you sign in at the provider and come back to mcp-forge. PKCE is always
  used.
- **OAuth2: client credentials**, for service accounts (machine to machine): token URL, client ID,
  scopes and an optional audience. No sign-in step.

mcp-forge sends the access token as a bearer token, caches it until shortly before it expires, and
refreshes it (from the refresh token, or a new client-credentials request). If the API still
answers 401, it fetches a fresh token and retries once. Sign-in tokens are stored encrypted with a
key in `secret.key` in the data directory (keep that file with the database; without it,
connections must be redone). Changing a config's endpoints, client ID or scopes disconnects it.
Tokens never appear in the UI, API responses, exports or logs.

Importing an OpenAPI spec whose security scheme describes an OAuth2 authorization-code or
client-credentials flow suggests the matching auth config.

## GraphQL APIs

A GraphQL tool is one fixed query or mutation sent to an endpoint. The agent fills in its
`$variables`; they are sent as `variables`, never pasted into the document. **New tool → GraphQL
query or mutation** opens the editor:

```graphql
query User($login: String!) {
  user(login: $login) {
    name
    bio
  }
}
```

Variables are read from the document (`String!` makes `login` required). Add a description for
each so the agent knows what to pass. Auth configs, OAuth2, timeouts, response shapes and approvals
work as for HTTP tools.

**Import** offers a whole API at once. Either choose *A GraphQL endpoint* on the *From a URL* tab
(mcp-forge sends the standard introspection query, with the auth config you pick), or upload or
paste the schema as SDL (`schema.graphql`) or as a saved introspection result (JSON); for a file,
enter the endpoint in the preview. Each query and mutation field becomes a tool with a generated
selection set: scalar fields, nested objects and interfaces up to three levels deep, and `... on`
fragments for unions. Fields that need arguments of their own are left out; edit the document after import
to add them. Deprecated fields are skipped. Re-import keeps your edits to a tool's document.

What agents see:

- Queries are marked read-only. Mutations are marked as changing data, and a **read-only project**
  hides them.
- A response with `data` returns the data (after your response shape). A response with `errors`
  fails the call with each message and its path, plus any partial data.

## Databases

Add a connection on the **Databases** tab. The connection string is a secret reference too:

| Database   | Value of the referenced variable                         |
|------------|----------------------------------------------------------|
| PostgreSQL | `postgres://readonly:PASSWORD@db.example.com:5432/shop`  |
| MySQL      | `mysql://readonly:PASSWORD@db.example.com:3306/shop`     |
| SQLite     | `/path/to/shop.db`                                       |
| SQL Server | `mssql://reader:PASSWORD@db.example.com:1433/shop` (ADO.NET `Server=…;` and `jdbc:sqlserver://…` strings work too; add `?trust_server_certificate=true` for self-signed certificates) |
| ClickHouse | `https://reader:PASSWORD@db.example.com:8443/shop` (the HTTP interface) |

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

How each database is kept read-only:

- **PostgreSQL, MySQL**: read-only transactions with a server-side time limit.
- **SQLite**: the file is opened read-only.
- **SQL Server** has no read-only transactions: every statement is checked first, and every read
  runs in a transaction that is rolled back. *Test* warns when the login can write; a login with only
  `db_datareader` is the safest choice.
- **ClickHouse**: every query is sent with `readonly=2`. ClickHouse has no transactions, so its
  connections are always read-only.

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

## NoSQL databases

MongoDB, Redis, DynamoDB and Elasticsearch connections are added on the same **Databases** tab:

| Database      | Value of the referenced variable |
|---------------|----------------------------------|
| MongoDB       | `mongodb://reader:PASSWORD@db.example.com:27017/shop` (or `mongodb+srv://…/shop`; the database name is required) |
| Redis         | `redis://:PASSWORD@cache.example.com:6379/0` (`rediss://` for TLS) |
| DynamoDB      | `dynamodb://ACCESS_KEY_ID:SECRET@us-east-1` (percent-encode `/` and `+` in the secret; `?session_token=…`, or `?endpoint=http://localhost:8000` for DynamoDB Local) |
| Elasticsearch | `https://reader:PASSWORD@search.example.com:9200` (or `?api_key=KEY`; OpenSearch works too) |

A **NoSQL tool** (*New tool → NoSQL operation*) is one operation on one target, with a JSON body:

| Database      | Operations | Target | Body, for example |
|---------------|------------|--------|-------------------|
| MongoDB       | `find`, `find_one`, `aggregate`, `count`, `distinct`; writes `insert_one`, `update_one/many`, `delete_one/many` | collection | `{"filter": {"email": "{{email}}"}, "limit": 20}` |
| Redis         | `GET`, `HGETALL`, `LRANGE`, `SMEMBERS`, `ZRANGE`, … ; writes `SET`, `DEL`, `HSET`, `EXPIRE`, … | key, e.g. `user:{{id}}` | `[0, 49]` (extra arguments) |
| DynamoDB      | `GetItem`, `Query`, `Scan`; writes `PutItem`, `UpdateItem`, `DeleteItem` | table | `{"KeyConditionExpression": "customer = :c", "ExpressionAttributeValues": {":c": "{{customer}}"}}` |
| Elasticsearch | `search`, `count`, `get`; writes `index`, `update`, `delete` | index | `{"query": {"match": {"title": "{{q}}"}}}` |

`{{name}}` marks where an agent's value goes; each becomes a typed argument (string, integer,
number or boolean). A whole-string `"{{name}}"` is replaced by the value itself (a number stays a
number), never by text that could add an operator, and object keys are never filled in. DynamoDB
bodies can use plain JSON values; they're converted to DynamoDB's typed format, and items come back
as plain JSON. Agents get `{documents, truncated}`, plus counts, totals or aggregations.

What keeps it safe:

- Only the listed operations run (no `EVAL`, `CONFIG` or `FLUSHALL`; no Elasticsearch scripts;
  no MongoDB `$where`, `$function`, `$out` or `$merge`). `$lookup` and `$unionWith` may only read
  allowed collections.
- *Names agents may use* limits collections, tables, indexes and keys (`user:*` patterns work);
  system names (`system.*`, `.security`, …) are always off limits.
- Masked fields (`users.password`, `*.ssn`) are replaced at any depth, and agent-written queries
  can't mention them.
- Results are capped by the row limit (MongoDB `limit`, Elasticsearch `size`, DynamoDB `Limit`,
  Redis scans instead of `HGETALL`/`SMEMBERS` on huge values) and the time limit.
- Writes follow the access mode like SQL: *Saved writes* allows them in saved tools, *Full access*
  in `run_query` too. At most *Most items one write may change* change per call (MongoDB counts the
  matches first), `dry_run` reports the count without changing anything, and every write is on the
  **Audit** tab.
- **Test** reports the server version, the user and whether it can write (MongoDB privileges, Redis
  ACL, Elasticsearch index privileges; DynamoDB permissions come from IAM).

**Agent tools** work as for SQL: `<db>_list_targets`, `<db>_describe_target` (a collection's
fields from a sample, a table's keys, an index mapping, a key's type) and `<db>_run_query`, where the
agent writes the operation, target and body itself.

## Resources and prompts

Besides tools, each project can offer MCP **resources** (things an agent reads) and **prompts**
(reusable instructions). Manage both on the **Context** tab.

Resources agents can list and read:

- **Your documents** (`forge://resources/<name>`): Markdown, text, JSON, CSV or YAML you write in
  mcp-forge, such as a style guide, a glossary or a runbook.
- **Database schemas** (`schema://<connection>`): tables or collections and their columns or
  fields, built from the connection's exploration queries. Offered only while that connection's
  *Describe* agent tool is on, and limited to the allowed tables, with masks applied.
- **Imported specs** (`spec://<id>`): the OpenAPI, Swagger or GraphQL documents you imported, when
  *Share imported specs* is on.

**Prompts** are templates with arguments: `Look up ticket {{ticket}} and draft a reply in a
{{tone}} tone.` Each `{{name}}` is an argument (required or not). Claude shows prompts as slash
commands; other clients list them under prompts. Connected clients are told when either list
changes. The **Context** tab previews exactly what agents get.

## Chained tools

A **chain** (*New tool → Chain of tools*) calls other tools of the project in order and shows up to
agents as one tool. Each step names a tool and computes its arguments with jq over
`{input, steps, last}`: the chain's own inputs, every earlier step's result, and the previous one.
An optional output expression (jq over `{input, steps}`) builds the result; otherwise it's the last
step's.

```text
inputs:  login (required)
step 1:  get_user     {login: .input.login}
step 2:  list_repos   {owner: .last.login}
output:  {user: .steps[0].login, repos: [.steps[1][].name]}
```

- Every step runs like a normal call: argument checks, limits, masks, and its own call-log entry,
  shown as a step of the chain's call.
- A failing step stops the chain; the agent is told which step failed and why.
- Up to 10 steps; chains may call chains three levels deep. Calling itself, loops between chains,
  missing tools and jq that doesn't compile are refused when you save.
- If any step needs approval, the chain waits for one approval before it starts.
- A chain changes data if any step does: agents see it as such, read-only projects hide it, and it
  gets a `dry_run` argument (database steps report what they'd change; other writing steps are
  skipped). The test console shows each step's arguments and result.

## Approvals

Turn on **Require approval** in a tool's editor and agent calls to it wait for a person before
anything is sent. New tools start with it on when they can change things (HTTP `DELETE`, SQL that
writes); imported tools start with it off.

- The call shows up on the **Approvals** page (a badge in the sidebar counts waiting calls, and a
  toast pops up in any open mcp-forge tab), with the tool, arguments and client. Approve runs it;
  Deny refuses it.
- Clients that support MCP form elicitation also get an Approve/Decline prompt in their own app.
  Whichever answer comes first wins, and the other prompt closes.
- Nobody answering within the project's timeout (**Settings → Approvals**, 300 seconds by default)
  refuses the call. Refused calls never reach the API or database, and the agent is told why.
- Test console runs don't wait. In `stdio` mode there's no web UI, so only clients with
  elicitation can approve; others get a clear refusal.
- Waiting calls live in memory: restarting the server refuses them. The call log records every
  decision, who made it, and how long the call waited.

## Call log

Each project's **Calls** tab lists every tool call, from agents and from the test console: when,
which tool, OK or error (with the upstream HTTP status), how long it took, the MCP client's name
and version (or the signed-in user for the console), the arguments, and the error text the agent
was given. Above the list: call count, error rate, median and 95th-percentile time, calls per hour
or day, and a per-tool breakdown for the last 24 hours, 7 days or 30 days.

- Argument values whose names look secret (`password`, `token`, `api_key`, `authorization`,
  `cookie`, …) are stored as `[redacted]`. Arguments are cut to 4 000 characters and error text to
  1 000.
- Calls to tools that don't exist (or are hidden) are recorded as *Unknown tool*.
- Calls are kept for 30 days by default (**Settings → Call log**, up to 365). Setting 0 stops
  recording and clears the log within the hour. Each project keeps at most 100 000 calls.
- The log is separate from the audit log, which records SQL writes in detail.
- API: `GET /api/projects/{id}/calls?tool=&status=&origin=&before=&limit=`,
  `GET /api/projects/{id}/calls/{call_id}` and `GET /api/projects/{id}/calls/summary?window=24h|7d|30d`.

## Security model

mcp-forge gives AI agents real access to your APIs and data, so it is built to fail closed.

**Network**

- Listens on `127.0.0.1:7410` by default. Requests must carry a loopback `Host` header, which blocks
  DNS-rebinding attacks from web pages you visit.
- Listening on any other address (`--bind 0.0.0.0:7410`) requires an admin token (16+ characters,
  `MCP_FORGE_ADMIN_TOKEN` or `--admin-token`). MCP clients must send it (or a project's
  [access key](#access-keys)) on `/mcp`; scripts may send it on `/api` instead of signing in. `/healthz` stays open. Put TLS in front (a reverse proxy) if the
  server is reachable from other machines.

**Accounts** (web UI and `/api`)

- Signing in is always required, on localhost too. The first account is created with the one-time
  setup code from the server log, so nobody else on the network can claim it first. More accounts,
  password resets and removal: the **Users** page (admins), or `mcp-forge user …` on the server.
- **Roles.** Admins can do everything: users, creating and deleting projects. Members see only the
  projects an admin gives them, each with **Can view** (read: tools, specs, auth references,
  databases, calls, audit log, connect snippets) or **Can change** (write: edits, the test console,
  access keys, connected apps, approvals and app sign-ins for that project). Deleting a project
  stays with admins. The server enforces this on every `/api` call; the UI hides what you can't do.
  Each project's **Settings → People** shows who has access. The first account is an admin,
  accounts from before roles existed are admins, the last admin can't be removed or demoted, and
  the admin token counts as admin. MCP access (keys, sign-in, stdio) doesn't depend on roles.
- Passwords are hashed with argon2id (8–256 characters). A wrong username and a wrong password get
  the same answer in the same time. After 5 failed attempts for a username (20 per client address)
  sign-in backs off, from 30 seconds doubling to 15 minutes; the backoff is stored, so restarting
  the server doesn't reset it.
- The session is a random 256-bit token in an `HttpOnly`, `SameSite=Strict` cookie (`Secure` behind
  an HTTPS proxy that sets `X-Forwarded-Proto`); only its SHA-256 is stored. Sessions end after
  7 days idle or 30 days in all, on sign-out, or when the password changes (other sessions only).
  The cookie name includes the port (`mcp_forge_session_7410`), so servers on different ports
  keep separate sign-ins.
- State-changing API calls with the cookie must carry an `X-Mcp-Forge` header, which other web
  pages can't add without a CORS preflight that the server never grants.
- MCP clients can't sign in: `/mcp` stays open on loopback (unless the project requires an access
  key) and needs the admin token or a project access key elsewhere. stdio is unaffected.

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
  `docker exec mcp-forge printenv MCP_FORGE_ADMIN_TOKEN` for MCP clients and scripts.
- On first start, `docker logs mcp-forge` shows the setup code for the first account. Manage
  accounts with `docker exec -it mcp-forge mcp-forge user …`.
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
mcp-forge user add NAME [--password-stdin] [--role admin|member]   # create an account (default admin)
mcp-forge user passwd NAME [--password-stdin]   # new password; ends all its sessions
mcp-forge user list
mcp-forge user remove NAME
```

`user` works on the data directory directly, while the server runs or not, so it also recovers a
forgotten password. Removing the last account brings back the first-run setup code.

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

The server also exposes `GET /healthz` and a REST API under `/api` (the UI uses it). Scripts
authenticate to `/api` with the admin token (`Authorization: Bearer <token>`).

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
- **Lost the setup code**: it's printed again on every start until the first account exists. Or
  create the account directly: `mcp-forge user add <name>`.
- **Forgot a password**: on the server, `mcp-forge user passwd <name>`.
- **"Too many failed attempts"**: wait for the time shown (the lockout survives restarts), or
  reset the password with `mcp-forge user passwd <name>`, which also lifts it.
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
