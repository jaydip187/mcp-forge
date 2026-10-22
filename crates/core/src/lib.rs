//! Canonical tool IR types and validation.
//!
//! Importers produce these types; the store persists them; the UI and MCP server
//! read only these. MCP input schemas are derived from them, never stored.

mod auth;
mod overrides;
mod project;
mod schema;
mod secret;
mod settings;
mod tool;
mod validate;

pub use auth::{ApiKeyLocation, AuthConfig, AuthConfigInput, AuthKind};
pub use overrides::{changed_fields, merge_overrides, OVERRIDABLE_FIELDS};
pub use project::{slugify, Project, ProjectInput};
pub use schema::{ArgBinding, ArgTarget, DerivedInput, BODY_ARG};
pub use secret::{SecretRef, SecretResolver};
pub use settings::{path_matches, Hidden, ProjectSettings, DEFAULT_MAX_RESPONSE_CHARS};
pub use tool::{
    is_valid_param_name, path_placeholders, HttpMethod, HttpSpec, Overrides, ParamDef,
    ParamLocation, Source, ToolDef, ToolInput, ToolKind, DEFAULT_TIMEOUT_MS, MAX_TAGS,
    MAX_TIMEOUT_MS,
};
pub use validate::{is_valid_name, FieldError, ValidationErrors, MAX_NAME_LEN};
