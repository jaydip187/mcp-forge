//! Canonical tool IR types and validation.
//!
//! Importers produce these types; the store persists them; the UI and MCP server
//! read only these. MCP input schemas are derived from them, never stored.

mod args;
mod audit;
mod auth;
mod calls;
mod context;
mod db;
mod graphql;
mod nosql;
mod overrides;
mod project;
mod schema;
mod secret;
mod settings;
mod tool;
mod validate;

pub use args::check_args;
pub use audit::{AuditEntry, AuditOutcome};
pub use auth::{ApiKeyLocation, AuthConfig, AuthConfigInput, AuthKind, OAuthClientAuth};
pub use calls::{
    is_secret_name, logged_arguments, logged_error, ApprovalDecision, ApprovalRecord, CallEntry,
    CallStatus, MAX_CALLS_PER_PROJECT, MAX_LOGGED_ARGS_CHARS, MAX_LOGGED_ERROR_CHARS, REDACTED,
};
pub use context::{
    PromptArg, PromptDef, PromptInput, ResourceDef, ResourceInput, MAX_PROMPT_BYTES,
    MAX_RESOURCE_BYTES, RESOURCE_MIME_TYPES, RESOURCE_URI_PREFIX,
};
pub use db::{
    is_valid_sql_param_name, AccessMode, ColumnMask, DbConnection, DbConnectionInput, Driver,
    SqlMode, SqlParam, SqlSpec, DEFAULT_ROW_LIMIT, DEFAULT_SQL_TIMEOUT_MS, MAX_ROW_LIMIT,
    PATTERN_ARG, SQL_ARG, SQL_PARAM_TYPES, TABLE_ARG,
};
pub use graphql::{schema_for_type, GraphqlOperation, GraphqlSpec, GraphqlVar};
pub use nosql::{
    nosql_operations, op_class, render_target, render_value, NosqlSpec, OpClass, DESCRIBE_TARGET,
    EXPLORATION_OPS, LIST_TARGETS, NOSQL_BODY_ARG, NOSQL_OPERATION_ARG, NOSQL_PATTERN_ARG,
    NOSQL_TARGET_ARG, RUN_QUERY,
};
pub use overrides::{changed_fields, merge_overrides, OVERRIDABLE_FIELDS};
pub use project::{slugify, Project, ProjectInput};
pub use schema::{ArgBinding, ArgTarget, DerivedInput, BODY_ARG};
pub use secret::{SecretRef, SecretResolver};
pub use settings::{
    path_matches, Hidden, ProjectSettings, DEFAULT_APPROVAL_TIMEOUT_SECS, DEFAULT_CALL_LOG_DAYS,
    DEFAULT_MAX_RESPONSE_CHARS, MAX_APPROVAL_TIMEOUT_SECS, MAX_CALL_LOG_DAYS,
    MIN_APPROVAL_TIMEOUT_SECS,
};
pub use tool::{
    is_valid_param_name, path_placeholders, HttpMethod, HttpSpec, Overrides, ParamDef,
    ParamLocation, Source, ToolDef, ToolInput, ToolKind, DEFAULT_TIMEOUT_MS, MAX_TAGS,
    MAX_TIMEOUT_MS,
};
pub use validate::{is_valid_name, FieldError, ValidationErrors, MAX_NAME_LEN};
