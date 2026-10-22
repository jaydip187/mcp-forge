/** Human names for override / diff field keys (see core OVERRIDABLE_FIELDS). */
const LABELS: Record<string, string> = {
  name: "name",
  description: "description",
  enabled: "visibility",
  response_shape: "response shape",
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
