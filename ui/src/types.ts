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
  call_log_days: number;
  approval_timeout_secs: number;
  require_key: boolean;
}

/** What the consent page shows about an app asking to connect. */
export interface ConsentInfo {
  client_name: string;
  redirect_host: string;
  project: { id: string; name: string; slug: string };
}

/** An app connected through sign-in (OAuth), as listed on the Connect tab. */
export interface ConnectedApp {
  id: string;
  client_id: string;
  client_name: string;
  project_id: string;
  approved_by: string | null;
  created_at: string;
  last_used_at: string | null;
  access_expires_at: string;
  refresh_expires_at: string;
}

/** An access key for a project's /mcp endpoint (never the key itself). */
export interface ApiKey {
  id: string;
  project_id: string;
  name: string;
  /** First characters of the key, to tell keys apart. */
  prefix: string;
  /** Allowed tools; null = all tools agents can see. */
  tools: string[] | null;
  expires_at: string | null;
  created_at: string;
  created_by: string | null;
  last_used_at: string | null;
}

export interface ApiKeyInput {
  name: string;
  tools: string[] | null;
  expires_at: string | null;
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

export interface GraphqlVar {
  /** Without the `$`. */
  name: string;
  /** As written in the document, e.g. `ID!` or `[String!]`. */
  graphql_type: string;
  description: string;
  /** Argument schema (set by the importer); derived from the type when absent. */
  schema?: JsonObject | null;
}

export type GraphqlOperation = "query" | "mutation";

export interface GraphqlSpec {
  type: "graphql";
  endpoint: string;
  operation: GraphqlOperation;
  document: string;
  operation_name: string | null;
  variables: GraphqlVar[];
  auth_ref: string | null;
  timeout_ms: number;
}

export interface NosqlSpec {
  type: "nosql";
  connection_id: string;
  /** An engine operation (`find`, `HGETALL`, `Query`, `search`, …) or an exploration tool. */
  operation: string;
  /** Collection, key, table or index; `{{param}}` templates allowed. */
  target: string;
  /** Operation body; `{{param}}` templates allowed in values. */
  body: Json;
  params: SqlParam[];
}

export type ToolKind = HttpSpec | SqlSpec | GraphqlSpec | NosqlSpec;

export interface ToolInput {
  name: string;
  description: string;
  kind: ToolKind;
  enabled: boolean;
  tags: string[];
  response_shape: string | null;
  require_approval: boolean;
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

export type OAuthClientAuth = "body" | "basic";

export type AuthKind =
  | { type: "bearer" }
  | { type: "basic"; username: string }
  | { type: "api_key"; location: "header" | "query"; name: string }
  | {
      type: "oauth2_client_credentials";
      token_url: string;
      client_id: string;
      scopes: string[];
      audience?: string | null;
      client_auth: OAuthClientAuth;
    }
  | {
      type: "oauth2_authorization_code";
      auth_url: string;
      token_url: string;
      client_id: string;
      scopes: string[];
      client_auth: OAuthClientAuth;
    };

/** A connected authorization-code config (tokens stay on the server). */
export interface OAuthStatus {
  auth_config_id: string;
  connected_at: string;
  connected_by: string | null;
  expires_at: string | null;
  scope: string | null;
  can_refresh: boolean;
}

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

export type Driver =
  | "sqlite"
  | "postgres"
  | "mysql"
  | "duckdb"
  | "mssql"
  | "clickhouse"
  | "mongodb"
  | "redis"
  | "dynamodb"
  | "elasticsearch";
// DuckDB is deferred: the server is built without it for now.
export const DRIVERS: Driver[] = [
  "postgres",
  "mysql",
  "mssql",
  "sqlite",
  "clickhouse",
  "mongodb",
  "redis",
  "dynamodb",
  "elasticsearch",
];
/** These run NoSQL tools; the others run SQL tools. */
export const NOSQL_DRIVERS: Driver[] = ["mongodb", "redis", "dynamodb", "elasticsearch"];
export const isSqlDriver = (d: Driver) => !NOSQL_DRIVERS.includes(d);
export const DRIVER_LABEL: Record<Driver, string> = {
  postgres: "PostgreSQL",
  mysql: "MySQL",
  sqlite: "SQLite",
  duckdb: "DuckDB",
  mssql: "SQL Server",
  clickhouse: "ClickHouse",
  mongodb: "MongoDB",
  redis: "Redis",
  dynamodb: "DynamoDB",
  elasticsearch: "Elasticsearch",
};

/** Operations per NoSQL engine, mirroring `forge_core::nosql_operations`. */
export const NOSQL_OPERATIONS: Record<string, { name: string; write: boolean }[]> = {
  mongodb: [
    ...["find", "find_one", "aggregate", "count", "distinct"].map((name) => ({ name, write: false })),
    ...["insert_one", "update_one", "update_many", "delete_one", "delete_many"].map((name) => ({ name, write: true })),
  ],
  redis: [
    ...["GET", "MGET", "STRLEN", "EXISTS", "TYPE", "TTL", "HGET", "HMGET", "HGETALL", "HKEYS", "HLEN", "LRANGE", "LLEN",
      "LINDEX", "SMEMBERS", "SISMEMBER", "SCARD", "ZRANGE", "ZREVRANGE", "ZRANGEBYSCORE", "ZSCORE", "ZCARD", "XRANGE",
      "XREVRANGE", "XLEN"].map((name) => ({ name, write: false })),
    ...["SET", "DEL", "EXPIRE", "INCR", "INCRBY", "HSET", "HDEL", "HINCRBY", "LPUSH", "RPUSH", "SADD", "SREM", "ZADD",
      "ZREM"].map((name) => ({ name, write: true })),
  ],
  dynamodb: [
    ...["GetItem", "Query", "Scan"].map((name) => ({ name, write: false })),
    ...["PutItem", "UpdateItem", "DeleteItem"].map((name) => ({ name, write: true })),
  ],
  elasticsearch: [
    ...["search", "count", "get"].map((name) => ({ name, write: false })),
    ...["index", "update", "delete"].map((name) => ({ name, write: true })),
  ],
};

/** Whether a NoSQL operation writes (names don't collide across engines). */
export function nosqlWrites(op: string): boolean {
  return Object.values(NOSQL_OPERATIONS).some((ops) => ops.some((o) => o.write && o.name.toLowerCase() === op.toLowerCase()));
}

export const NOSQL_EXPLORATION = ["list_targets", "describe_target", "run_query"] as const;
/** ClickHouse has no transactions, so writes can't be capped or undone. */
export const READ_ONLY_DRIVERS: Driver[] = ["clickhouse"];

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
  /** DuckDB: absolute folders whose files queries may read. */
  file_allowlist: string[];
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
  /** GraphQL tools run as one HTTP POST. */
  kind: "http" | "graphql";
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

export interface NosqlTestRun extends TestRunBase {
  kind: "nosql";
  request: { operation: string; target?: string; body?: Json } | null;
  response: {
    documents: Json[];
    count: number;
    truncated: boolean;
    summary: Json | null;
    masked_fields: string[];
    elapsed_ms: number;
    write: { affected_rows: number; committed: boolean; dry_run: boolean } | null;
  } | null;
  error: SqlTestRun["error"];
}

export type TestRun = HttpTestRun | SqlTestRun | NosqlTestRun;

export type Role = "admin" | "member";
export type Access = "read" | "write";

export interface User {
  id: string;
  username: string;
  role: Role;
  created_at: string;
}

export interface AuthStatus {
  login_required: boolean;
  setup_required: boolean;
  user: User | null;
  /** Members' per-project access; null for admins. */
  access: Record<string, Access> | null;
}

/** An account as the Users page lists it. */
export interface UserView extends User {
  access: { project_id: string; access: Access }[];
}

/** Who can see a project (its People card). */
export interface ProjectPerson {
  user: User;
  /** null for admins. */
  access: Access | null;
}

export interface ServerInfo {
  version: string;
  exe: string | null;
  data_dir: string | null;
  auth_required: boolean;
}

export interface ImportOperation {
  key: string;
  /** `graphql` operations are queries and mutations; `method` is then POST. */
  kind: "http" | "graphql";
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
    kind: "openapi3" | "swagger2" | "postman" | "har" | "curl" | "graphql";
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

export type CallStatus = "ok" | "error" | "unknown_tool" | "not_approved";

export interface ApprovalRecord {
  decision: "approved" | "denied" | "timed_out" | "cancelled";
  decided_by: string | null;
  wait_ms: number;
}

export interface PendingApproval {
  id: string;
  project_id: string;
  project_name: string;
  project_slug: string;
  tool_id: string;
  tool_name: string;
  client: string | null;
  arguments: Json;
  created_at: string;
  expires_at: string;
  asked_client: boolean;
}

export interface CallEntry {
  id: string;
  project_id: string;
  ts: string;
  tool_id: string | null;
  tool_name: string;
  origin: "mcp" | "console";
  client: string | null;
  arguments: Json;
  status: CallStatus;
  duration_ms: number;
  response_chars: number;
  upstream_status: number | null;
  error: string | null;
  approval: ApprovalRecord | null;
  api_key: string | null;
}

export type CallWindow = "24h" | "7d" | "30d";

export interface CallFilter {
  tool?: string;
  status?: CallStatus;
  origin?: "mcp" | "console";
}

export interface ToolCallSummary {
  tool_name: string;
  calls: number;
  errors: number;
  p50_ms: number | null;
  p95_ms: number | null;
  last_call: string;
}

export interface CallSummary {
  window: CallWindow;
  since: string;
  bucket: "hour" | "day";
  total: number;
  errors: number;
  p50_ms: number | null;
  p95_ms: number | null;
  tools: ToolCallSummary[];
  buckets: { start: string; ok: number; errors: number }[];
}
