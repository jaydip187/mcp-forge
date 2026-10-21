import { parseTyped, schemaType } from "./template";
import type { JsonObject } from "../types";

/**
 * Turns the test console's text boxes into tool arguments. Empty boxes are
 * left out; everything else is parsed by its schema type.
 */
export function buildArgs(
  properties: Record<string, JsonObject>,
  values: Record<string, string>,
): { args: JsonObject; errors: Record<string, string> } {
  const args: JsonObject = {};
  const errors: Record<string, string> = {};
  for (const [name, schema] of Object.entries(properties)) {
    const text = values[name] ?? "";
    if (text === "") continue;
    const parsed = parseTyped(text, schemaType(schema));
    if ("error" in parsed) errors[name] = parsed.error;
    else args[name] = parsed.value;
  }
  return { args, errors };
}

/** Pretty-prints JSON bodies; anything else is returned unchanged. */
export function prettyBody(body: string, contentType: string | null): string {
  const looksJson = (contentType ?? "").includes("json") || /^\s*[[{]/.test(body);
  if (!looksJson) return body;
  try {
    return JSON.stringify(JSON.parse(body), null, 2);
  } catch {
    return body;
  }
}
