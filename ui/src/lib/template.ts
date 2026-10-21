import type { Json, ParamDef } from "../types";

export type Segment = { kind: "text"; text: string } | { kind: "param"; name: string };

/** Splits `/users/{id}/posts` into text and `{param}` segments. Malformed braces stay text. */
export function splitTemplate(template: string): Segment[] {
  const out: Segment[] = [];
  const re = /\{([^{}]+)\}/g;
  let last = 0;
  for (const m of template.matchAll(re)) {
    const start = m.index ?? 0;
    if (start > last) out.push({ kind: "text", text: template.slice(last, start) });
    out.push({ kind: "param", name: m[1] ?? "" });
    last = start + m[0].length;
  }
  if (last < template.length) out.push({ kind: "text", text: template.slice(last) });
  return out;
}

export function placeholders(template: string): string[] {
  return splitTemplate(template).flatMap((s) => (s.kind === "param" ? [s.name] : []));
}

/** Adds a required path param for every `{placeholder}` that has none yet. */
export function syncPathParams(template: string, params: ParamDef[]): ParamDef[] {
  const have = new Set(params.filter((p) => p.location === "path").map((p) => p.name));
  const added = placeholders(template)
    .filter((name, i, all) => !have.has(name) && all.indexOf(name) === i)
    .map<ParamDef>((name) => ({
      name,
      location: "path",
      schema: { type: "string" },
      required: true,
      description: null,
      fixed_value: null,
    }));
  return added.length ? [...params, ...added] : params;
}

export type SchemaType = "string" | "integer" | "number" | "boolean" | "array" | "object";
export const SCHEMA_TYPES: SchemaType[] = ["string", "integer", "number", "boolean", "array", "object"];

export function schemaType(schema: { [k: string]: Json }): SchemaType | "custom" {
  const t = schema.type;
  return typeof t === "string" && (SCHEMA_TYPES as string[]).includes(t) ? (t as SchemaType) : "custom";
}

/**
 * Parses text typed into a value box according to a JSON Schema type.
 * Strings are taken literally; everything else must parse.
 */
export function parseTyped(text: string, type: SchemaType | "custom"): { value: Json } | { error: string } {
  switch (type) {
    case "string":
      return { value: text };
    case "integer": {
      const n = Number(text.trim());
      return text.trim() !== "" && Number.isInteger(n) ? { value: n } : { error: "Enter a whole number." };
    }
    case "number": {
      const n = Number(text.trim());
      return text.trim() !== "" && Number.isFinite(n) ? { value: n } : { error: "Enter a number." };
    }
    case "boolean":
      if (text === "true" || text === "false") return { value: text === "true" };
      return { error: "Enter true or false." };
    default:
      try {
        const v = JSON.parse(text) as Json;
        if (type === "array" && !Array.isArray(v)) return { error: "Enter a JSON array, like [1, 2]." };
        if (type === "object" && (v === null || typeof v !== "object" || Array.isArray(v)))
          return { error: 'Enter a JSON object, like {"a": 1}.' };
        return { value: v };
      } catch {
        return { error: "Enter valid JSON." };
      }
  }
}

/** Text shown in a value box for an existing value. */
export function typedToText(value: Json | null | undefined): string {
  if (value === null || value === undefined) return "";
  return typeof value === "string" ? value : JSON.stringify(value);
}

/** Mirrors the server's slugify: lowercase ASCII alphanumerics joined by single dashes. */
export function slugify(name: string): string {
  return name
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, "-")
    .replace(/^-+|-+$/g, "")
    .slice(0, 64)
    .replace(/-+$/, "");
}
