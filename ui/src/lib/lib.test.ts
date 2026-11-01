import { describe, expect, it } from "vitest";

import { buildArgs, prettyBody } from "./args";
import { ms } from "./format";
import { parseRoute } from "./router";
import { buildSnippets } from "./snippets";
import { parseTyped, placeholders, slugify, splitTemplate, syncPathParams } from "./template";
import { buildToolInput, newForm, paramIndex, type RowText } from "./toolForm";
import type { ParamDef } from "../types";

const param = (p: Partial<ParamDef>): ParamDef => ({
  name: "x",
  location: "query",
  schema: { type: "string" },
  required: false,
  description: null,
  fixed_value: null,
  ...p,
});
const row = (p: Partial<ParamDef>, extra: Partial<RowText> = {}): RowText => {
  const def = param(p);
  return { def, fixedOn: false, fixedText: "", schemaOpen: false, schemaText: JSON.stringify(def.schema), ...extra };
};

describe("template", () => {
  it("splits placeholders", () => {
    expect(splitTemplate("/u/{id}/p/{pid}")).toEqual([
      { kind: "text", text: "/u/" },
      { kind: "param", name: "id" },
      { kind: "text", text: "/p/" },
      { kind: "param", name: "pid" },
    ]);
    expect(placeholders("/a/{x}/{y}")).toEqual(["x", "y"]);
    expect(placeholders("/a/{x")).toEqual([]);
  });

  it("adds missing path params once, keeps existing ones", () => {
    const existing = [param({ name: "id", location: "path", required: true })];
    const synced = syncPathParams("/u/{id}/{tab}/{tab}", existing);
    expect(synced.map((p) => [p.name, p.location, p.required])).toEqual([
      ["id", "path", true],
      ["tab", "path", true],
    ]);
    expect(syncPathParams("/u/{id}", existing)).toBe(existing);
  });

  it("parses typed values", () => {
    expect(parseTyped("42", "integer")).toEqual({ value: 42 });
    expect(parseTyped("4.2", "integer")).toHaveProperty("error");
    expect(parseTyped("", "number")).toHaveProperty("error");
    expect(parseTyped("true", "boolean")).toEqual({ value: true });
    expect(parseTyped(" x ", "string")).toEqual({ value: " x " });
    expect(parseTyped("[1]", "array")).toEqual({ value: [1] });
    expect(parseTyped("{}", "array")).toHaveProperty("error");
    expect(parseTyped("nope", "object")).toHaveProperty("error");
  });

  it("slugifies like the server", () => {
    expect(slugify("My GitHub API")).toBe("my-github-api");
    expect(slugify("  --Hello__World!!  ")).toBe("hello-world");
    expect(slugify("日本")).toBe("");
  });
});

describe("args", () => {
  it("skips empty boxes and reports bad ones by name", () => {
    const props = { id: { type: "integer" }, q: { type: "string" }, tags: { type: "array" } };
    expect(buildArgs(props, { id: "5", q: "" })).toEqual({ args: { id: 5 }, errors: {} });
    const r = buildArgs(props, { id: "five", tags: "[1" });
    expect(Object.keys(r.errors).sort()).toEqual(["id", "tags"]);
  });

  it("pretty-prints JSON bodies only", () => {
    expect(prettyBody('{"a":1}', "application/json")).toBe('{\n  "a": 1\n}');
    expect(prettyBody("plain", "text/plain")).toBe("plain");
    expect(prettyBody("{broken", "application/json")).toBe("{broken");
  });
});

describe("router", () => {
  it("parses hash routes", () => {
    expect(parseRoute("")).toEqual({ name: "home" });
    expect(parseRoute("#/p/abc")).toEqual({ name: "project", projectId: "abc", tab: "tools" });
    expect(parseRoute("#/p/abc/connect")).toEqual({ name: "project", projectId: "abc", tab: "connect" });
    expect(parseRoute("#/p/abc/bogus")).toEqual({ name: "project", projectId: "abc", tab: "tools" });
    expect(parseRoute("#/p/abc/tools/new")).toEqual({ name: "tool", projectId: "abc", toolId: null, kind: "http" });
    expect(parseRoute("#/p/abc/tools/new-sql")).toEqual({ name: "tool", projectId: "abc", toolId: null, kind: "sql" });
    expect(parseRoute("#/p/abc/tools/t1")).toEqual({ name: "tool", projectId: "abc", toolId: "t1", kind: "http" });
    expect(parseRoute("#/p/abc/databases")).toEqual({ name: "project", projectId: "abc", tab: "databases" });
    expect(parseRoute("#/p/abc/audit")).toEqual({ name: "project", projectId: "abc", tab: "audit" });
    expect(parseRoute("#/p/abc/import")).toEqual({ name: "import", projectId: "abc" });
    expect(parseRoute("#/p/abc/specs")).toEqual({ name: "project", projectId: "abc", tab: "specs" });
    expect(parseRoute("#/p/abc/specs/s1/reimport")).toEqual({ name: "reimport", projectId: "abc", specId: "s1" });
  });
});

describe("snippets", () => {
  const info = { version: "0.1.0", exe: "C:\\Program Files\\mcp-forge.exe", data_dir: "/home/me/.mcp-forge", auth_required: false };

  it("builds HTTP and stdio configs", () => {
    const s = buildSnippets("demo", "http://127.0.0.1:7410/mcp/demo", info);
    const byId = Object.fromEntries(s.map((x) => [x.id, x.code]));
    expect(byId["claude-http"]).toBe("claude mcp add --transport http demo http://127.0.0.1:7410/mcp/demo");
    expect(JSON.parse(byId["json-http"]!)).toEqual({ mcpServers: { demo: { type: "http", url: "http://127.0.0.1:7410/mcp/demo" } } });
    expect(JSON.parse(byId["json-stdio"]!)).toEqual({
      mcpServers: { demo: { command: info.exe, args: ["stdio", "--project", "demo", "--data-dir", "/home/me/.mcp-forge"] } },
    });
    expect(byId["claude-stdio"]).toBe(
      "claude mcp add demo -- 'C:\\Program Files\\mcp-forge.exe' stdio --project demo --data-dir /home/me/.mcp-forge",
    );
  });

  it("uses a token placeholder, never a real token", () => {
    const s = buildSnippets("demo", "http://h/mcp/demo", { ...info, auth_required: true });
    const http = s.find((x) => x.id === "json-http")!;
    expect(JSON.parse(http.code).mcpServers.demo.headers).toEqual({ Authorization: "Bearer <your-admin-token>" });
    expect(s.find((x) => x.id === "claude-http")!.code).toContain("--header");
  });

  it("uses an access-key placeholder when the project has keys", () => {
    const s = buildSnippets("demo", "http://h/mcp/demo", info, true);
    expect(JSON.parse(s.find((x) => x.id === "json-http")!.code).mcpServers.demo.headers).toEqual({
      Authorization: "Bearer <your-access-key>",
    });
    expect(s.find((x) => x.id === "claude-http")!.code).toContain("'Authorization: Bearer <your-access-key>'");
    expect(s.find((x) => x.id === "json-stdio")!.code).not.toContain("access-key");
  });
});

describe("toolForm", () => {
  const form = { ...newForm(), name: "get_user", base_url: "https://api.x.io", path_template: "/u/{id}" };

  it("builds a valid payload", () => {
    const r = buildToolInput(form, [
      row({ name: "id", location: "path", required: true, schema: { type: "integer" } }),
      row({ name: "v", location: "header" }, { fixedOn: true, fixedText: "2" }),
    ]);
    expect(r.ok).toBe(true);
    if (!r.ok || r.input.kind.type !== "http") throw new Error("expected an HTTP payload");
    expect(r.input.kind.params[1]!.fixed_value).toBe("2");
    expect(r.input.kind.timeout_ms).toBe(30000);
    expect(r.input.kind.body_schema).toBeNull();
  });

  it("types fixed values by schema and reports row errors", () => {
    const r = buildToolInput(form, [
      row({ name: "n", schema: { type: "integer" } }, { fixedOn: true, fixedText: "x" }),
      row({ name: "s" }, { schemaOpen: true, schemaText: "{oops" }),
    ]);
    expect(r.ok).toBe(false);
    if (r.ok) return;
    expect(Object.keys(r.rowErrors)).toEqual(["0", "1"]);
  });

  it("validates body schema and timeout", () => {
    const r = buildToolInput({ ...form, bodyMode: "schema", bodySchemaText: "[]", timeout: "abc" }, []);
    expect(r.ok).toBe(false);
    if (r.ok) return;
    expect(Object.keys(r.fieldErrors).sort()).toEqual(["kind.body_schema", "kind.timeout_ms"]);
  });

  it("maps server param errors to rows", () => {
    expect(paramIndex("kind.params[3].name")).toBe(3);
    expect(paramIndex("kind.params")).toBeNull();
  });
});

import { pathMatcher } from "./glob";

describe("glob", () => {
  it("matches globs and substrings", () => {
    const pets = pathMatcher("/pet/*");
    expect(pets("/pet/{petId}")).toBe(true);
    expect(pets("/pet/{petId}/uploadImage")).toBe(false);
    expect(pathMatcher("/pet/**")("/pet/{petId}/uploadImage")).toBe(true);
    expect(pathMatcher("/repos/*/*/issues")("/repos/{owner}/{repo}/issues")).toBe(true);
    expect(pathMatcher("store")("/store/inventory")).toBe(true);
    expect(pathMatcher("STORE")("/store/inventory")).toBe(true);
    expect(pathMatcher("/a.b/*")("/aXb/c")).toBe(false);
    expect(pathMatcher("")("/anything")).toBe(true);
  });
});

import { listLabels } from "./fields";

describe("fields", () => {
  it("lists override labels in plain words", () => {
    expect(listLabels(["description"])).toBe("description");
    expect(listLabels(["description", "kind.params"])).toBe("description and parameters");
    expect(listLabels(["name", "enabled", "kind.base_url"])).toBe("name, visibility and base URL");
    expect(listLabels([])).toBe("");
  });
});

import { hiddenReason } from "./visibility";
import { splitTags } from "./toolForm";
import type { ProjectSettings, Tool } from "../types";

describe("visibility", () => {
  const settings = (s: Partial<ProjectSettings>): ProjectSettings => ({
    read_only: false,
    include_tags: [],
    exclude_tags: [],
    include_paths: [],
    exclude_paths: [],
    strip_nulls: false,
    max_response_chars: 50000,
    tool_search: false,
    call_log_days: 30,
    approval_timeout_secs: 300,
    require_key: false,
    ...s,
  });
  const tool = (method: string, path: string, tags: string[], enabled = true) =>
    ({ enabled, tags, kind: { type: "http", method, path_template: path } }) as unknown as Tool;

  it("matches the server's rules", () => {
    expect(hiddenReason(tool("POST", "/a", []), settings({}))).toBeNull();
    expect(hiddenReason(tool("GET", "/a", [], false), settings({}))).toBe("disabled");
    expect(hiddenReason(tool("DELETE", "/a", []), settings({ read_only: true }))).toBe("read_only");
    expect(hiddenReason(tool("GET", "/a", ["Admin"]), settings({ exclude_tags: ["admin"] }))).toBe("tag");
    expect(hiddenReason(tool("GET", "/a", []), settings({ include_tags: ["pet"] }))).toBe("tag");
    expect(
      hiddenReason(
        tool("GET", "/repos/{o}/{r}/hooks/{id}", []),
        settings({ include_paths: ["/repos/**"], exclude_paths: ["/repos/*/*/hooks/**"] }),
      ),
    ).toBe("path");
    expect(hiddenReason(tool("GET", "/repos/{o}/{r}/issues", []), settings({ include_paths: ["/repos/**"] }))).toBeNull();
  });

  it("treats SQL tools as read-only and outside path filters", () => {
    const sql = { enabled: true, tags: ["db"], kind: { type: "sql" } } as unknown as Tool;
    expect(hiddenReason(sql, settings({ read_only: true, include_paths: ["/api/**"] }))).toBeNull();
    expect(hiddenReason(sql, settings({ exclude_tags: ["db"] }))).toBe("tag");
  });

  it("splits tags", () => {
    expect(splitTags(" a, b,,a , c ")).toEqual(["a", "b", "c"]);
    expect(splitTags("")).toEqual([]);
  });
});

import {
  buildSqlInput,
  enumText,
  looksLikeWrite,
  newSqlForm,
  splitList,
  sqlParamIndex,
  sqlPlaceholders,
  syncSqlParams,
  withEnum,
} from "./sqlForm";

describe("sql form", () => {
  it("finds :name placeholders outside strings, comments and casts", () => {
    expect(
      sqlPlaceholders(
        "SELECT a::text, ':nope', \"x:y\" FROM t -- :c\nWHERE id = :id /* :d */ AND n = :name AND m = :id",
      ),
    ).toEqual(["id", "name"]);
    expect(sqlPlaceholders("SELECT 1")).toEqual([]);
  });

  it("adds params for new placeholders only", () => {
    const params = syncSqlParams("SELECT * FROM t WHERE a = :a AND b = :b", [
      { name: "a", schema: { type: "integer" }, required: false, description: "kept" },
    ]);
    expect(params.map((p) => [p.name, p.schema.type, p.required])).toEqual([
      ["a", "integer", false],
      ["b", "string", true],
    ]);
  });

  it("builds the payload and requires a connection", () => {
    const form = { ...newSqlForm(), name: " q ", sql: "SELECT :x", params: [{ name: "x", schema: { type: "string" }, required: true, description: " " }] };
    expect(buildSqlInput(form)).toEqual({ ok: false, fieldErrors: { "kind.connection_id": expect.any(String) } });
    const r = buildSqlInput({ ...form, connection_id: "c1", tagsText: "a, b" });
    if (!r.ok || r.input.kind.type !== "sql") throw new Error("expected a SQL payload");
    expect(r.input.name).toBe("q");
    expect(r.input.tags).toEqual(["a", "b"]);
    expect(r.input.kind).toEqual({
      type: "sql",
      connection_id: "c1",
      mode: { type: "query", sql: "SELECT :x", params: [{ name: "x", schema: { type: "string" }, required: true, description: null }] },
    });
  });

  it("guesses writes for labels, ignoring strings and comments", () => {
    expect(looksLikeWrite("UPDATE orders SET status = :s WHERE id = :id")).toBe(true);
    expect(looksLikeWrite("SELECT 'delete me', replace(name, 'a', 'b') FROM t -- drop\n")).toBe(false);
    expect(looksLikeWrite("WITH x AS (SELECT 1) INSERT INTO t SELECT * FROM x")).toBe(true);
  });

  it("edits allowed values by type", () => {
    const p = { name: "n", schema: { type: "integer" }, required: true, description: null };
    const schema = withEnum(p.schema, "1, 2, x", "integer");
    expect(schema).toEqual({ type: "integer", enum: [1, 2, "x"] });
    expect(enumText({ ...p, schema })).toBe("1, 2, x");
    expect(withEnum(schema, " ", "integer")).toEqual({ type: "integer" });
  });

  it("builds exploration tools without SQL or params", () => {
    const r = buildSqlInput({ ...newSqlForm("c1"), name: "shop_run_sql", mode: "run_sql", sql: "ignored" });
    if (!r.ok || r.input.kind.type !== "sql") throw new Error("expected a SQL payload");
    expect(r.input.kind.mode).toEqual({ type: "run_sql" });
  });

  it("maps server error paths and splits lists", () => {
    expect(sqlParamIndex("kind.mode.params[2].name")).toBe(2);
    expect(sqlParamIndex("kind.mode.sql")).toBeNull();
    expect(splitList("users.email\n*.ssn, users.email,")).toEqual(["users.email", "*.ssn"]);
  });
});

describe("ms", () => {
  it("formats durations", () => {
    expect(ms(null)).toBe("–");
    expect(ms(0)).toBe("0 ms");
    expect(ms(999)).toBe("999 ms");
    expect(ms(1234)).toBe("1.23 s");
    expect(ms(12_345)).toBe("12.3 s");
  });
});

describe("calls route", () => {
  it("parses the calls tab", () => {
    expect(parseRoute("#/p/abc/calls")).toEqual({ name: "project", projectId: "abc", tab: "calls" });
  });
});

describe("approval defaults", () => {
  it("asks for approval by default only for DELETE", async () => {
    const { approvalDefault } = await import("./toolForm");
    expect(approvalDefault("DELETE")).toBe(true);
    expect(approvalDefault("GET")).toBe(false);
    expect(approvalDefault("POST")).toBe(false);
  });

  it("follows the method until the switch is flipped", async () => {
    const { buildToolInput, newForm } = await import("./toolForm");
    const form = { ...newForm(), name: "x", method: "DELETE" as const, base_url: "https://api.test", path_template: "/x" };
    const built = buildToolInput(form, []);
    expect(built.ok && built.input.require_approval).toBe(true);
    const off = buildToolInput({ ...form, requireApproval: false }, []);
    expect(off.ok && off.input.require_approval).toBe(false);
  });

  it("parses the approvals route", () => {
    expect(parseRoute("#/approvals")).toEqual({ name: "approvals" });
  });
});

describe("oauth auth configs", () => {
  it("describes them and builds the redirect URL", async () => {
    const { describeAuth, oauthRedirectUri } = await import("../pages/AuthTab");
    expect(oauthRedirectUri("http://127.0.0.1:7410")).toBe("http://127.0.0.1:7410/oauth/callback");
    expect(
      describeAuth({
        type: "oauth2_authorization_code",
        auth_url: "https://accounts.example.com/auth",
        token_url: "https://oauth.example.com/token",
        client_id: "x",
        scopes: [],
        client_auth: "body",
      }),
    ).toBe("OAuth2 sign-in with accounts.example.com");
    expect(
      describeAuth({ type: "oauth2_client_credentials", token_url: "https://auth.example.com/t", client_id: "x", scopes: [], client_auth: "basic" }),
    ).toBe("OAuth2 client credentials from auth.example.com");
  });
});

describe("consent route", () => {
  it("parses the OAuth consent page", () => {
    expect(parseRoute("#/oauth/consent/abc_123")).toEqual({ name: "consent", requestId: "abc_123" });
  });
});

describe("users route", () => {
  it("parses the users page", () => {
    expect(parseRoute("#/users")).toEqual({ name: "users" });
  });
});

describe("spec kinds", () => {
  it("labels every import format", async () => {
    const { specKindLabel } = await import("./specKind");
    expect(specKindLabel("openapi3", "3.1.0")).toBe("OpenAPI 3.1.0");
    expect(specKindLabel("swagger2")).toBe("Swagger 2.0");
    expect(specKindLabel("postman")).toBe("Postman collection");
    expect(specKindLabel("har")).toBe("HAR capture");
    expect(specKindLabel("curl")).toBe("curl commands");
    expect(specKindLabel("graphql")).toBe("GraphQL schema");
  });
});

describe("graphql form", () => {
  const doc = `# the user
query User($login: String!, $first: Int = 10 $tags: [ String! ]) {
  user(login: $login) { name }
}`;

  it("reads the operation header", async () => {
    const { declaredVariables, operationNameOf, operationOf } = await import("./graphqlForm");
    expect(operationOf(doc)).toBe("query");
    expect(operationOf("mutation Follow($id: ID!) { follow(id: $id) { ok } }")).toBe("mutation");
    expect(operationOf("{ viewer { login } }")).toBe("query");
    expect(operationNameOf(doc)).toBe("User");
    expect(operationNameOf("{ viewer { login } }")).toBeNull();
    expect(declaredVariables(doc)).toEqual([
      { name: "login", graphql_type: "String!" },
      { name: "first", graphql_type: "Int" },
      { name: "tags", graphql_type: "[String!]" },
    ]);
  });

  it("keeps descriptions and drops stale schemas when syncing", async () => {
    const { syncGraphqlVars } = await import("./graphqlForm");
    const synced = syncGraphqlVars(doc, [
      { name: "login", graphql_type: "String!", description: "GitHub login", schema: { type: "string", minLength: 1 } },
      { name: "first", graphql_type: "Float", description: "How many", schema: { type: "number" } },
      { name: "gone", graphql_type: "ID", description: "" },
    ]);
    expect(synced).toEqual([
      { name: "login", graphql_type: "String!", description: "GitHub login", schema: { type: "string", minLength: 1 } },
      { name: "first", graphql_type: "Int", description: "How many", schema: null },
      { name: "tags", graphql_type: "[String!]", description: "" },
    ]);
  });

  it("builds the payload; mutations need approval by default", async () => {
    const { buildGraphqlInput, newGraphqlForm } = await import("./graphqlForm");
    const form = { ...newGraphqlForm(), name: "follow", endpoint: " https://api.example.com/graphql ", document: "mutation Follow($id: ID!) { follow(id: $id) { ok } }" };
    const r = buildGraphqlInput(form);
    if (!r.ok || r.input.kind.type !== "graphql") throw new Error("expected a GraphQL payload");
    expect(r.input.require_approval).toBe(true);
    expect(r.input.kind).toEqual({
      type: "graphql",
      endpoint: "https://api.example.com/graphql",
      operation: "mutation",
      document: form.document,
      operation_name: "Follow",
      variables: [{ name: "id", graphql_type: "ID!", description: "" }],
      auth_ref: null,
      timeout_ms: 30000,
    });
    expect(buildGraphqlInput({ ...form, timeout: "soon" }).ok).toBe(false);
  });

  it("routes and hides mutations in read-only projects", async () => {
    const { hiddenReason } = await import("./visibility");
    expect(parseRoute("#/p/x/tools/new-graphql")).toEqual({ name: "tool", projectId: "x", toolId: null, kind: "graphql" });
    const mutation = { enabled: true, tags: [], kind: { type: "graphql", operation: "mutation" } } as unknown as Tool;
    const s = { read_only: true, include_tags: [], exclude_tags: [], include_paths: [], exclude_paths: [] } as unknown as ProjectSettings;
    expect(hiddenReason(mutation, s)).toBe("read_only");
    expect(hiddenReason({ ...mutation, kind: { type: "graphql", operation: "query" } } as unknown as Tool, s)).toBeNull();
  });
});

describe("nosql form", () => {
  it("finds placeholders and adds params for new ones", async () => {
    const { nosqlPlaceholders, syncNosqlParams } = await import("./nosqlForm");
    expect(nosqlPlaceholders('user:{{id}} {"q": "{{ q }}", "again": "{{id}}"}')).toEqual(["id", "q"]);
    const params = syncNosqlParams({
      target: "user:{{id}}",
      bodyText: '{ "x": "{{q}}" }',
      params: [{ name: "id", schema: { type: "integer" }, required: true, description: null }],
    });
    expect(params.map((p) => p.name)).toEqual(["id", "q"]);
    expect(params[0]!.schema.type).toBe("integer");
  });

  it("builds the payload; writes need approval by default; bad JSON is caught", async () => {
    const { buildNosqlInput, newNosqlForm } = await import("./nosqlForm");
    const form = {
      ...newNosqlForm("c1"),
      name: "remove_user",
      operation: "delete_many",
      target: " users ",
      bodyText: '{ "filter": { "email": "{{email}}" } }',
      params: [{ name: "email", schema: { type: "string" }, required: true, description: " " }],
    };
    const r = buildNosqlInput(form);
    if (!r.ok || r.input.kind.type !== "nosql") throw new Error("expected a NoSQL payload");
    expect(r.input.require_approval).toBe(true);
    expect(r.input.kind).toEqual({
      type: "nosql",
      connection_id: "c1",
      operation: "delete_many",
      target: "users",
      body: { filter: { email: "{{email}}" } },
      params: [{ name: "email", schema: { type: "string" }, required: true, description: null }],
    });
    const bad = buildNosqlInput({ ...form, bodyText: "{ nope" });
    expect(bad.ok).toBe(false);
    if (!bad.ok) expect(bad.fieldErrors["kind.body"]).toContain("Not valid JSON");
    const read = buildNosqlInput({ ...form, operation: "find" });
    expect(read.ok && read.input.require_approval).toBe(false);
  });

  it("routes and hides writes in read-only projects", async () => {
    const { hiddenReason } = await import("./visibility");
    expect(parseRoute("#/p/x/tools/new-nosql")).toEqual({ name: "tool", projectId: "x", toolId: null, kind: "nosql" });
    const s = { read_only: true, include_tags: [], exclude_tags: [], include_paths: [], exclude_paths: [] } as unknown as ProjectSettings;
    const tool = (operation: string) => ({ enabled: true, tags: [], kind: { type: "nosql", operation } }) as unknown as Tool;
    expect(hiddenReason(tool("DEL"), s)).toBe("read_only");
    expect(hiddenReason(tool("update_many"), s)).toBe("read_only");
    expect(hiddenReason(tool("find"), s)).toBeNull();
    expect(hiddenReason(tool("run_query"), s)).toBeNull();
  });
});
