import { splitTags } from "./toolForm";
import type { ExplorationMode, Json, JsonObject, SqlParam, SqlParamType, Tool, ToolInput } from "../types";

export interface SqlForm {
  name: string;
  description: string;
  enabled: boolean;
  /** Comma-separated, as typed. */
  tagsText: string;
  connection_id: string;
  /** `query` for curated SQL; exploration tools have no SQL or params. */
  mode: "query" | ExplorationMode;
  sql: string;
  params: SqlParam[];
  /** jq expression, as typed. */
  responseShape: string;
}

export function newSqlForm(connectionId = ""): SqlForm {
  return {
    name: "",
    description: "",
    enabled: true,
    tagsText: "",
    connection_id: connectionId,
    mode: "query",
    sql: "",
    params: [],
    responseShape: "",
  };
}

export function sqlFormFromTool(t: Tool): SqlForm {
  if (t.kind.type !== "sql") throw new Error("sqlFormFromTool needs a SQL tool");
  return {
    name: t.name,
    description: t.description,
    enabled: t.enabled,
    tagsText: t.tags.join(", "),
    connection_id: t.kind.connection_id,
    mode: t.kind.mode.type,
    sql: t.kind.mode.type === "query" ? t.kind.mode.sql : "",
    params: t.kind.mode.type === "query" ? t.kind.mode.params : [],
    responseShape: t.response_shape ?? "",
  };
}

/**
 * `:name` placeholders in order of first use, skipping strings, quoted
 * identifiers, comments and `::` casts. Mirrors the server's tokenizer
 * closely enough for the editor; the server has the final say.
 */
export function sqlPlaceholders(sql: string): string[] {
  const out: string[] = [];
  let i = 0;
  while (i < sql.length) {
    const c = sql[i]!;
    const next = sql[i + 1];
    if (c === "'" || c === '"' || c === "`") {
      const end = sql.indexOf(c, i + 1);
      i = end === -1 ? sql.length : end + 1;
    } else if (c === "-" && next === "-") {
      const end = sql.indexOf("\n", i);
      i = end === -1 ? sql.length : end + 1;
    } else if (c === "/" && next === "*") {
      const end = sql.indexOf("*/", i + 2);
      i = end === -1 ? sql.length : end + 2;
    } else if (c === ":" && next === ":") {
      i += 2;
    } else if (c === ":") {
      const m = /^[A-Za-z_][A-Za-z0-9_]*/.exec(sql.slice(i + 1));
      if (m && !out.includes(m[0])) out.push(m[0]);
      i += 1 + (m?.[0].length ?? 0);
    } else {
      i += 1;
    }
  }
  return out;
}

/** Adds a string param for every placeholder that has none yet. */
export function syncSqlParams(sql: string, params: SqlParam[]): SqlParam[] {
  const have = new Set(params.map((p) => p.name));
  const added = sqlPlaceholders(sql)
    .filter((n) => !have.has(n))
    .map<SqlParam>((name) => ({ name, schema: { type: "string" }, required: true, description: null }));
  return added.length ? [...params, ...added] : params;
}

export function paramType(p: SqlParam): SqlParamType {
  const t = p.schema.type;
  return t === "integer" || t === "number" || t === "boolean" ? t : "string";
}

export type SqlBuildResult = { ok: true; input: ToolInput } | { ok: false; fieldErrors: Record<string, string> };

export function buildSqlInput(form: SqlForm): SqlBuildResult {
  const fieldErrors: Record<string, string> = {};
  if (!form.connection_id) fieldErrors["kind.connection_id"] = "Pick the database this query runs on.";
  if (Object.keys(fieldErrors).length) return { ok: false, fieldErrors };
  return {
    ok: true,
    input: {
      name: form.name.trim(),
      description: form.description,
      enabled: form.enabled,
      tags: splitTags(form.tagsText),
      response_shape: form.responseShape.trim() || null,
      kind: {
        type: "sql",
        connection_id: form.connection_id,
        mode:
          form.mode === "query"
            ? {
                type: "query",
                sql: form.sql,
                params: form.params.map((p) => ({ ...p, name: p.name.trim(), description: p.description?.trim() || null })),
              }
            : { type: form.mode },
      },
    },
  };
}

/** Index of the param a server error path like `kind.mode.params[2].name` refers to. */
export function sqlParamIndex(field: string): number | null {
  const m = /^kind\.mode\.params\[(\d+)\]/.exec(field);
  return m ? Number(m[1]) : null;
}

/** Splits a textarea of masks or table names on commas and newlines. */
export function splitList(text: string): string[] {
  const out: string[] = [];
  for (const t of text.split(/[,\n]/).map((x) => x.trim())) {
    if (t && !out.includes(t)) out.push(t);
  }
  return out;
}

export const EXPLORATION_LABEL: Record<ExplorationMode, { title: string; blurb: string }> = {
  list_tables: { title: "List tables", blurb: "Lists the tables and views the agent may query." },
  describe_table: { title: "Describe table", blurb: "Shows one table's columns, types, keys and which are masked." },
  run_sql: {
    title: "Run SQL",
    blurb: "Lets the agent write its own read-only SELECT against allowed tables. Most flexible, so most to review.",
  },
};

/**
 * Rough guess whether SQL changes data, for labels only (the server
 * classifies statements properly and enforces the rules).
 */
export function looksLikeWrite(sql: string): boolean {
  const code = sql
    .replace(/--[^\n]*/g, " ")
    .replace(/\/\*[\s\S]*?\*\//g, " ")
    .replace(/'(?:[^']|'')*'/g, "''");
  return /\b(insert|update|delete|merge|create|alter|drop|truncate)\b/i.test(code);
}

/** A param's allowed values as typed text ("a, b"). */
export function enumText(p: SqlParam): string {
  const e = p.schema.enum;
  return Array.isArray(e) ? e.map((v) => String(v)).join(", ") : "";
}

/** Sets (or clears) `enum` from comma-separated text, typed by the param type. */
export function withEnum(schema: JsonObject, text: string, type: SqlParamType): JsonObject {
  const { enum: _, ...rest } = schema;
  const parts = text.split(",").map((x) => x.trim()).filter(Boolean);
  if (parts.length === 0) return rest;
  const values: Json[] = parts.map((v) =>
    type === "integer" || type === "number" ? (Number.isFinite(Number(v)) ? Number(v) : v) : type === "boolean" ? v === "true" : v,
  );
  return { ...rest, enum: values };
}
