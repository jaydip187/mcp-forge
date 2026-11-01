import { splitTags } from "./toolForm";
import { nosqlWrites, type Driver, type Json, type SqlParam, type Tool, type ToolInput } from "../types";

export interface NosqlForm {
  name: string;
  description: string;
  enabled: boolean;
  /** Comma-separated, as typed. */
  tagsText: string;
  connection_id: string;
  operation: string;
  target: string;
  /** The body as typed JSON. */
  bodyText: string;
  params: SqlParam[];
  /** jq expression, as typed. */
  responseShape: string;
  /** `null` until the user flips the switch: then it follows the operation. */
  requireApproval: boolean | null;
}

export function newNosqlForm(connectionId = ""): NosqlForm {
  return {
    name: "",
    description: "",
    enabled: true,
    tagsText: "",
    connection_id: connectionId,
    operation: "",
    target: "",
    bodyText: "",
    params: [],
    responseShape: "",
    requireApproval: null,
  };
}

export function nosqlFormFromTool(t: Tool): NosqlForm {
  if (t.kind.type !== "nosql") throw new Error("nosqlFormFromTool needs a NoSQL tool");
  return {
    name: t.name,
    description: t.description,
    enabled: t.enabled,
    tagsText: t.tags.join(", "),
    connection_id: t.kind.connection_id,
    operation: t.kind.operation,
    target: t.kind.target,
    bodyText: t.kind.body === null ? "" : JSON.stringify(t.kind.body, null, 2),
    params: t.kind.params,
    responseShape: t.response_shape ?? "",
    requireApproval: t.require_approval,
  };
}

/** `{{name}}` placeholders in order of first use. */
export function nosqlPlaceholders(text: string): string[] {
  const out: string[] = [];
  for (const m of text.matchAll(/\{\{\s*([A-Za-z_][A-Za-z0-9_]*)\s*\}\}/g)) {
    if (!out.includes(m[1]!)) out.push(m[1]!);
  }
  return out;
}

/** Adds a string param for every placeholder that has none yet. */
export function syncNosqlParams(form: Pick<NosqlForm, "target" | "bodyText" | "params">): SqlParam[] {
  const have = new Set(form.params.map((p) => p.name));
  const added = nosqlPlaceholders(`${form.target}\n${form.bodyText}`)
    .filter((n) => !have.has(n))
    .map<SqlParam>((name) => ({ name, schema: { type: "string" }, required: true, description: null }));
  return added.length ? [...form.params, ...added] : form.params;
}

/** What the body looks like for an operation (shown as a placeholder). */
export function bodyExample(driver: Driver | undefined, op: string): string {
  switch (driver) {
    case "mongodb":
      if (op === "aggregate") return '{\n  "pipeline": [\n    { "$match": { "status": "{{status}}" } },\n    { "$group": { "_id": "$region", "total": { "$sum": "$amount" } } }\n  ]\n}';
      if (op === "insert_one") return '{ "document": { "name": "{{name}}" } }';
      if (op.startsWith("update")) return '{ "filter": { "_id": { "$oid": "{{id}}" } }, "update": { "$set": { "status": "{{status}}" } } }';
      if (op.startsWith("delete")) return '{ "filter": { "_id": { "$oid": "{{id}}" } } }';
      if (op === "distinct") return '{ "field": "region", "filter": {} }';
      return '{\n  "filter": { "email": "{{email}}" },\n  "projection": { "name": 1, "email": 1 },\n  "sort": { "created": -1 },\n  "limit": 20\n}';
    case "redis":
      if (op === "LRANGE" || op === "ZRANGE" || op === "ZREVRANGE") return "[0, 49]";
      if (op === "HGET") return '["{{field}}"]';
      if (op === "SET") return '["{{value}}", "EX", 3600]';
      return "[]";
    case "dynamodb":
      if (op === "GetItem" || op === "DeleteItem") return '{ "Key": { "id": "{{id}}" } }';
      if (op === "PutItem") return '{ "Item": { "id": "{{id}}", "name": "{{name}}" } }';
      if (op === "UpdateItem")
        return '{\n  "Key": { "id": "{{id}}" },\n  "UpdateExpression": "SET #s = :s",\n  "ExpressionAttributeNames": { "#s": "status" },\n  "ExpressionAttributeValues": { ":s": "{{status}}" }\n}';
      return '{\n  "KeyConditionExpression": "customer_id = :c",\n  "ExpressionAttributeValues": { ":c": "{{customer_id}}" }\n}';
    case "elasticsearch":
      if (op === "get" || op === "delete") return '{ "id": "{{id}}" }';
      if (op === "index") return '{ "id": "{{id}}", "document": { "title": "{{title}}" } }';
      if (op === "update") return '{ "id": "{{id}}", "doc": { "status": "{{status}}" } }';
      if (op === "count") return '{ "query": { "term": { "status": "{{status}}" } } }';
      return '{\n  "query": { "match": { "title": "{{q}}" } },\n  "size": 20\n}';
    default:
      return "{}";
  }
}

/** What the target names, per engine. */
export function targetLabel(driver: Driver | undefined): string {
  switch (driver) {
    case "mongodb":
      return "Collection";
    case "redis":
      return "Key";
    case "dynamodb":
      return "Table";
    case "elasticsearch":
      return "Index";
    default:
      return "Target";
  }
}

export type NosqlBuildResult = { ok: true; input: ToolInput } | { ok: false; fieldErrors: Record<string, string> };

export function buildNosqlInput(form: NosqlForm): NosqlBuildResult {
  const fieldErrors: Record<string, string> = {};
  if (!form.connection_id) fieldErrors["kind.connection_id"] = "Pick the database this runs on.";
  if (!form.operation) fieldErrors["kind.operation"] = "Pick an operation.";
  let body: Json = null;
  if (form.bodyText.trim()) {
    try {
      body = JSON.parse(form.bodyText) as Json;
    } catch (e) {
      fieldErrors["kind.body"] = `Not valid JSON: ${e instanceof Error ? e.message : String(e)}`;
    }
  }
  if (Object.keys(fieldErrors).length) return { ok: false, fieldErrors };
  return {
    ok: true,
    input: {
      name: form.name.trim(),
      description: form.description,
      enabled: form.enabled,
      tags: splitTags(form.tagsText),
      response_shape: form.responseShape.trim() || null,
      require_approval: form.requireApproval ?? nosqlWrites(form.operation),
      kind: {
        type: "nosql",
        connection_id: form.connection_id,
        operation: form.operation,
        target: form.target.trim(),
        body,
        params: form.params.map((p) => ({ ...p, name: p.name.trim(), description: p.description?.trim() || null })),
      },
    },
  };
}

/** Index of the param a server error path like `kind.params[2].name` refers to. */
export function nosqlParamIndex(field: string): number | null {
  const m = /^kind\.params\[(\d+)\]/.exec(field);
  return m ? Number(m[1]) : null;
}
