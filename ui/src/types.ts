// Mirrors the Rust IR in crates/core. Keep in sync.

export type HttpMethod = "GET" | "POST" | "PUT" | "PATCH" | "DELETE" | "HEAD" | "OPTIONS";
export const METHODS: HttpMethod[] = ["GET", "POST", "PUT", "PATCH", "DELETE", "HEAD", "OPTIONS"];

export type ParamLocation = "path" | "query" | "header" | "body";
export const LOCATIONS: ParamLocation[] = ["path", "query", "header", "body"];

export type Json = null | boolean | number | string | Json[] | { [k: string]: Json };
export type JsonObject = { [k: string]: Json };

export interface ProjectSettings {
  read_only: boolean;
  include_tags: string[];
  exclude_tags: string[];
  include_paths: string[];
  exclude_paths: string[];
  strip_nulls: boolean;
  max_response_chars: number;
  tool_search: boolean;
}

export interface Project {
  id: string;
  name: string;
  slug: string;
  settings: ProjectSettings;
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

export type SqlParamType = "string" | "integer" | "number" | "boolean";
export const SQL_PARAM_TYPES: SqlParamType[] = ["string", "integer", "number", "boolean"];

export interface SqlParam {
  /** Placeholder name: `:name` in the SQL. */
  name: string;
  schema: JsonObject;
  required: boolean;
  description: string | null;
}

export type ExplorationMode = "list_tables" | "describe_table" | "run_sql";
export const EXPLORATION_MODES: ExplorationMode[] = ["list_tables", "describe_table", "run_sql"];

export type SqlMode = { type: "query"; sql: string; params: SqlParam[] } | { type: ExplorationMode };

export interface SqlSpec {
  type: "sql";
  connection_id: string;
  mode: SqlMode;
}

export type ToolKind = HttpSpec | SqlSpec;

export interface ToolInput {
  name: string;
  description: string;
  kind: ToolKind;
  enabled: boolean;
  tags: string[];
  response_shape: string | null;
}

export type ToolSource =
  | { type: "manual" }
  | { type: "open_api" | "swagger"; spec_id: string; op_id: string };

export interface Tool extends ToolInput {
  id: string;
  project_id: string;
  source: ToolSource;
  /** Fields the user edited; kept on re-import. */
  overrides: { fields: string[] };
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

export type Driver = "sqlite" | "postgres" | "mysql";
export const DRIVERS: Driver[] = ["postgres", "mysql", "sqlite"];
export const DRIVER_LABEL: Record<Driver, string> = { postgres: "PostgreSQL", mysql: "MySQL", sqlite: "SQLite" };

export type AccessMode = "read_only" | "curated_write" | "full_access";

export interface DbConnectionInput {
  name: string;
  driver: Driver;
  /** env:NAME or keyring:NAME holding the connection string. */
  dsn_ref: string;
  access_mode: AccessMode;
  row_limit: number;
  timeout_ms: number;
  max_affected_rows: number;
  allow_ddl: boolean;
  table_allowlist: string[];
  /** `table.column`, `schema.table.column` or `*.column`. */
  column_masks: string[];
}

export interface DbConnection extends DbConnectionInput {
  id: string;
  project_id: string;
  created_at: string;
  updated_at: string;
}

export interface ConnectionReport {
  ok: boolean;
  server_version: string | null;
  user: string | null;
  can_write: boolean | null;
  write_privileges: string[];
  warnings: string[];
  error: string | null;
  elapsed_ms: number;
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

interface TestRunBase {
  agent_view: { is_error: boolean; text: string };
  shaped: string | null;
}

export interface HttpTestRun extends TestRunBase {
  kind: "http";
  request: { method: string; url: string } | null;
  response: {
    status: number;
    status_line: string;
    content_type: string | null;
    body: string;
    truncated: boolean;
  } | null;
  error: { kind: "invalid_args" | "config" | "request"; message: string; fields?: FieldError[] } | null;
}

export interface SqlTestRun extends TestRunBase {
  kind: "sql";
  request: { sql: string; params: Json[] } | null;
  response: {
    columns: string[];
    rows: Json[][];
    row_count: number;
    truncated: boolean;
    masked_columns: string[];
    elapsed_ms: number;
    write: { class: "write" | "ddl"; affected_rows: number; committed: boolean; dry_run: boolean } | null;
  } | null;
  error: {
    kind: "invalid_args" | "rejected" | "config" | "connect" | "timeout" | "database" | "too_many_rows";
    message: string;
    fields?: FieldError[];
  } | null;
}

export type TestRun = HttpTestRun | SqlTestRun;

export interface User {
  id: string;
  username: string;
  created_at: string;
}

export interface AuthStatus {
  login_required: boolean;
  setup_required: boolean;
  user: User | null;
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
    /** Version as written: "3.1.0", or "2.0" for Swagger. */
    openapi: string;
    kind: "openapi3" | "swagger2";
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
  options: { base_url?: string | null; auth_ref?: string | null };
  created_at: string;
  updated_at: string | null;
}

export interface ReimportPreview {
  spec: SpecRecord;
  info: ImportPreview["info"];
  base_url: string | null;
  auth_ref: string | null;
  changed: { tool_id: string; name: string; new_name: string; key: string; fields: string[]; kept: string[] }[];
  unchanged: number;
  blocked: { tool_id: string; name: string; key: string; warnings: string[] }[];
  added: ImportOperation[];
  removed: { tool_id: string; name: string; op_id: string | null }[];
  content: string | null;
  source_url: string | null;
}

export interface BundleReport {
  auth_created: string[];
  auth_updated: string[];
  auth_kept: string[];
  connections_created: string[];
  connections_updated: string[];
  connections_kept: string[];
  tools_created: string[];
  tools_updated: string[];
  tools_skipped: string[];
}

export interface AuditEntry {
  id: string;
  project_id: string;
  ts: string;
  tool_id: string | null;
  tool_name: string;
  connection_id: string | null;
  connection_name: string;
  origin: "mcp" | "console";
  statement_class: "write" | "ddl";
  sql: string;
  params: Json[];
  rows_affected: number | null;
  outcome: "committed" | "rolled_back" | "dry_run" | "failed";
  error: string | null;
}
