/** Human names for override / diff field keys (see core OVERRIDABLE_FIELDS). */
const LABELS: Record<string, string> = {
  name: "name",
  description: "description",
  enabled: "visibility",
  response_shape: "response shape",
  require_approval: "approval",
  "kind.method": "method",
  "kind.base_url": "base URL",
  "kind.path_template": "path",
  "kind.params": "parameters",
  "kind.body_schema": "body schema",
  "kind.auth_ref": "auth",
  "kind.timeout_ms": "timeout",
};

export function fieldLabel(field: string): string {
  return LABELS[field] ?? field;
}

/** "a, b and c" */
export function listLabels(fields: string[]): string {
  const names = fields.map(fieldLabel);
  if (names.length <= 1) return names.join("");
  return `${names.slice(0, -1).join(", ")} and ${names[names.length - 1]}`;
}

/**
 * A field path from a server error, in words for a list of problems:
 * `kind.base_url` is "Base URL", `kind.params[1].name` "Parameter 2 name".
 */
export function problemField(field: string): string {
  const words =
    LABELS[field] ??
    field
      .replace(/^kind\.(mode\.)?/, "")
      .replace(/(params|variables|arguments|steps)\[(\d+)\]/g, (_, list: string, i: string) => `${SINGULAR[list] ?? list} ${Number(i) + 1}`)
      .replace(/\[(\d+)\]/g, (_, i: string) => ` ${Number(i) + 1}`)
      .replace(/[._]/g, " ")
      .replace(/\burl\b/g, "URL")
      .replace(/\bsql\b/g, "SQL")
      .replace(/\bdsn ref\b/, "connection string reference")
      .replace(/\bsecret ref\b/, "secret reference");
  return words.charAt(0).toUpperCase() + words.slice(1);
}

const SINGULAR: Record<string, string> = { params: "parameter", variables: "variable", arguments: "argument", steps: "step" };
