import { describe, expect, it } from "vitest";

import { buildArgs, prettyBody } from "./args";
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
