import { splitTags } from "./toolForm";
import type { GraphqlOperation, GraphqlVar, Tool, ToolInput } from "../types";

export interface GraphqlForm {
  name: string;
  description: string;
  enabled: boolean;
  /** Comma-separated, as typed. */
  tagsText: string;
  endpoint: string;
  document: string;
  /** Variables as last synced from the document, with their descriptions. */
  variables: GraphqlVar[];
  auth_ref: string | null;
  /** As typed. */
  timeout: string;
  /** jq expression, as typed. */
  responseShape: string;
  /** `null` until the user flips the switch: then it follows the operation. */
  requireApproval: boolean | null;
}

export function newGraphqlForm(): GraphqlForm {
  return {
    name: "",
    description: "",
    enabled: true,
    tagsText: "",
    endpoint: "",
    document: "",
    variables: [],
    auth_ref: null,
    timeout: "30000",
    responseShape: "",
    requireApproval: null,
  };
}

export function graphqlFormFromTool(t: Tool): GraphqlForm {
  if (t.kind.type !== "graphql") throw new Error("graphqlFormFromTool needs a GraphQL tool");
  return {
    name: t.name,
    description: t.description,
    enabled: t.enabled,
    tagsText: t.tags.join(", "),
    endpoint: t.kind.endpoint,
    document: t.kind.document,
    variables: t.kind.variables,
    auth_ref: t.kind.auth_ref,
    timeout: String(t.kind.timeout_ms),
    responseShape: t.response_shape ?? "",
    requireApproval: t.require_approval,
  };
}

/** The part of the document before its selection set: `query Name($a: T)`. */
function header(document: string): string {
  const doc = document.replace(/#[^\n]*/g, " ").trim();
  const brace = doc.indexOf("{");
  return brace === -1 ? doc : doc.slice(0, brace);
}

/** `mutation` when the document starts with it; otherwise a query. */
export function operationOf(document: string): GraphqlOperation {
  return /^mutation\b/.test(header(document).trim()) ? "mutation" : "query";
}

/** The operation's name, e.g. `User` in `query User($login: String!)`. */
export function operationNameOf(document: string): string | null {
  return /^(?:query|mutation)\s+([A-Za-z_]\w*)/.exec(header(document).trim())?.[1] ?? null;
}

/** `$name: Type` declarations of the operation, in order. */
export function declaredVariables(document: string): { name: string; graphql_type: string }[] {
  const out: { name: string; graphql_type: string }[] = [];
  for (const m of header(document).matchAll(/\$([A-Za-z_]\w*)\s*:\s*([[\]\w!\s]+)/g)) {
    const graphql_type = m[2]!.replace(/\s+/g, "");
    if (!out.some((v) => v.name === m[1])) out.push({ name: m[1]!, graphql_type });
  }
  return out;
}

/** Variables from the document, keeping descriptions and schemas of ones that stay. */
export function syncGraphqlVars(document: string, vars: GraphqlVar[]): GraphqlVar[] {
  return declaredVariables(document).map((d) => {
    const old = vars.find((v) => v.name === d.name);
    if (!old) return { ...d, description: "" };
    // An imported schema describes the old type; drop it when the type changes.
    return old.graphql_type === d.graphql_type ? old : { ...old, graphql_type: d.graphql_type, schema: null };
  });
}

export type GraphqlBuildResult = { ok: true; input: ToolInput } | { ok: false; fieldErrors: Record<string, string> };

export function buildGraphqlInput(form: GraphqlForm): GraphqlBuildResult {
  const fieldErrors: Record<string, string> = {};
  const timeout = Number(form.timeout);
  if (!Number.isInteger(timeout)) fieldErrors["kind.timeout_ms"] = "Enter a whole number of milliseconds.";
  if (Object.keys(fieldErrors).length) return { ok: false, fieldErrors };
  const operation = operationOf(form.document);
  return {
    ok: true,
    input: {
      name: form.name.trim(),
      description: form.description,
      enabled: form.enabled,
      tags: splitTags(form.tagsText),
      response_shape: form.responseShape.trim() || null,
      require_approval: form.requireApproval ?? operation === "mutation",
      kind: {
        type: "graphql",
        endpoint: form.endpoint.trim(),
        operation,
        document: form.document,
        operation_name: operationNameOf(form.document),
        variables: syncGraphqlVars(form.document, form.variables).map((v) => ({ ...v, description: v.description.trim() })),
        auth_ref: form.auth_ref,
        timeout_ms: timeout,
      },
    },
  };
}

/** Index of the variable a server error path like `kind.variables[2].name` refers to. */
export function graphqlVarIndex(field: string): number | null {
  const m = /^kind\.variables\[(\d+)\]/.exec(field);
  return m ? Number(m[1]) : null;
}
