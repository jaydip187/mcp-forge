// Mirrors the Rust IR in crates/core. Keep in sync.

export type HttpMethod = "GET" | "POST" | "PUT" | "PATCH" | "DELETE" | "HEAD" | "OPTIONS";
export const METHODS: HttpMethod[] = ["GET", "POST", "PUT", "PATCH", "DELETE", "HEAD", "OPTIONS"];

export type ParamLocation = "path" | "query" | "header" | "body";
export const LOCATIONS: ParamLocation[] = ["path", "query", "header", "body"];

export type Json = null | boolean | number | string | Json[] | { [k: string]: Json };
export type JsonObject = { [k: string]: Json };

export interface Project {
  id: string;
  name: string;
  slug: string;
  created_at: string;
  updated_at: string;
}

export interface ParamDef {
  name: string;
  location: ParamLocation;
  schema: JsonObject;
  required: boolean;
  description: string | null;
  fixed_value: Json | null;
  /** Upstream name when it differs from `name` (set by importers). */
  wire_name?: string;
}

export interface HttpSpec {
  type: "http";
  method: HttpMethod;
  base_url: string;
  path_template: string;
  params: ParamDef[];
  body_schema: JsonObject | null;
  auth_ref: string | null;
  timeout_ms: number;
}

export interface ToolInput {
  name: string;
  description: string;
  kind: HttpSpec;
  enabled: boolean;
  response_shape: string | null;
}

export interface Tool extends ToolInput {
  id: string;
  project_id: string;
  source: { type: string };
  created_at: string;
  updated_at: string;
}

export type AuthKind =
  | { type: "bearer" }
  | { type: "basic"; username: string }
  | { type: "api_key"; location: "header" | "query"; name: string };

export interface AuthConfigInput {
  name: string;
  kind: AuthKind;
  secret_ref: string;
}

export interface AuthConfig extends AuthConfigInput {
  id: string;
  project_id: string;
  created_at: string;
  updated_at: string;
}

export interface FieldError {
  field: string;
  message: string;
}

export interface McpToolView {
  name: string;
  description?: string;
  inputSchema: JsonObject & { properties?: Record<string, JsonObject>; required?: string[] };
  annotations?: { readOnlyHint?: boolean; destructiveHint?: boolean };
}

export interface TestRun {
  request: { method: string; url: string } | null;
  response: {
    status: number;
    status_line: string;
    content_type: string | null;
    body: string;
    truncated: boolean;
  } | null;
  error: { kind: "invalid_args" | "config" | "request"; message: string; fields?: FieldError[] } | null;
  agent_view: { is_error: boolean; text: string };
  shaped: string | null;
}

export interface ServerInfo {
  version: string;
  exe: string | null;
  data_dir: string | null;
  auth_required: boolean;
}

export interface ImportOperation {
  key: string;
  operation_id: string | null;
  method: HttpMethod;
  path: string;
  name: string;
  tags: string[];
  summary: string | null;
  deprecated: boolean;
  needs_auth: boolean;
  param_count: number;
  base_url: string;
  supported: boolean;
  warnings: string[];
}

export interface ImportPreview {
  info: {
    title: string;
    api_version: string;
    openapi: string;
    servers: string[];
    base_url: string | null;
    security_schemes: { name: string; description: string; suggested: AuthKind | null }[];
  };
  operations: ImportOperation[];
  any_auth: boolean;
  /** Present when the server fetched the spec from a URL. */
  content: string | null;
  source_url: string | null;
}

export interface SpecRecord {
  id: string;
  kind: string;
  title: string;
  version: string;
  source_url: string | null;
  filename: string | null;
  created_at: string;
}
