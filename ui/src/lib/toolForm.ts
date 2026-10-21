import { parseTyped, schemaType } from "./template";
import type { HttpMethod, Json, JsonObject, ParamDef, Tool, ToolInput } from "../types";

export interface ToolForm {
  name: string;
  description: string;
  enabled: boolean;
  method: HttpMethod;
  base_url: string;
  path_template: string;
  auth_ref: string | null;
  timeout: string;
  bodyMode: "fields" | "schema";
  bodySchemaText: string;
  response_shape: string | null;
}

/** Text state behind one param row (see ParamsEditor). */
export interface RowText {
  def: ParamDef;
  fixedOn: boolean;
  fixedText: string;
  schemaOpen: boolean;
  schemaText: string;
}

export function newForm(): ToolForm {
  return {
    name: "",
    description: "",
    enabled: true,
    method: "GET",
    base_url: "",
    path_template: "",
    auth_ref: null,
    timeout: "30000",
    bodyMode: "fields",
    bodySchemaText: '{\n  "type": "object",\n  "properties": {}\n}',
    response_shape: null,
  };
}

export function formFromTool(t: Tool): ToolForm {
  return {
    name: t.name,
    description: t.description,
    enabled: t.enabled,
    method: t.kind.method,
    base_url: t.kind.base_url,
    path_template: t.kind.path_template,
    auth_ref: t.kind.auth_ref,
    timeout: String(t.kind.timeout_ms),
    bodyMode: t.kind.body_schema ? "schema" : "fields",
    bodySchemaText: t.kind.body_schema ? JSON.stringify(t.kind.body_schema, null, 2) : newForm().bodySchemaText,
    response_shape: t.response_shape,
  };
}

export type BuildResult =
  | { ok: true; input: ToolInput }
  | { ok: false; fieldErrors: Record<string, string>; rowErrors: Record<number, string> };

/** Builds the API payload, catching what the browser can check before the server does. */
export function buildToolInput(form: ToolForm, rows: RowText[]): BuildResult {
  const fieldErrors: Record<string, string> = {};
  const rowErrors: Record<number, string> = {};

  const params: ParamDef[] = rows.map((r, i) => {
    let schema = r.def.schema;
    if (r.schemaOpen) {
      try {
        const parsed = JSON.parse(r.schemaText) as Json;
        if (parsed && typeof parsed === "object" && !Array.isArray(parsed)) schema = parsed;
        else rowErrors[i] = "The schema must be a JSON object.";
      } catch {
        rowErrors[i] = "The schema isn't valid JSON.";
      }
    }
    let fixed: Json | null = null;
    if (r.fixedOn) {
      const parsed = parseTyped(r.fixedText, schemaType(schema));
      if ("error" in parsed) rowErrors[i] ??= `Fixed value: ${parsed.error}`;
      else fixed = parsed.value;
    }
    return { ...r.def, name: r.def.name.trim(), schema, fixed_value: fixed, description: r.def.description?.trim() || null };
  });

  let body_schema: JsonObject | null = null;
  if (form.bodyMode === "schema") {
    try {
      const parsed = JSON.parse(form.bodySchemaText) as Json;
      if (parsed && typeof parsed === "object" && !Array.isArray(parsed)) body_schema = parsed;
      else fieldErrors["kind.body_schema"] = "The body schema must be a JSON object.";
    } catch {
      fieldErrors["kind.body_schema"] = "The body schema isn't valid JSON.";
    }
  }

  const timeout = Number(form.timeout);
  if (!Number.isInteger(timeout) || timeout < 1) fieldErrors["kind.timeout_ms"] = "Enter a whole number of milliseconds.";

  if (Object.keys(fieldErrors).length || Object.keys(rowErrors).length) return { ok: false, fieldErrors, rowErrors };
  return {
    ok: true,
    input: {
      name: form.name.trim(),
      description: form.description,
      enabled: form.enabled,
      response_shape: form.response_shape,
      kind: {
        type: "http",
        method: form.method,
        base_url: form.base_url.trim(),
        path_template: form.path_template.trim(),
        params,
        body_schema,
        auth_ref: form.auth_ref,
        timeout_ms: timeout,
      },
    },
  };
}

/** Index of the param a server error path like `kind.params[2].name` refers to. */
export function paramIndex(field: string): number | null {
  const m = /^kind\.params\[(\d+)\]/.exec(field);
  return m ? Number(m[1]) : null;
}
