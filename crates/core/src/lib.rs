//! Canonical tool IR types and validation.
//!
//! Importers produce these types; the store persists them; the UI and MCP server
//! read only these. MCP input schemas are derived from them, never stored.

mod auth;
mod project;
mod schema;
mod secret;
mod tool;
mod validate;

pub use auth::{ApiKeyLocation, AuthConfig, AuthConfigInput, AuthKind};
pub use project::{slugify, Project, ProjectInput};
pub use schema::{ArgBinding, ArgTarget, DerivedInput, BODY_ARG};
pub use secret::{SecretRef, SecretResolver};
pub use tool::{
    is_valid_param_name, path_placeholders, HttpMethod, HttpSpec, Overrides, ParamDef,
    ParamLocation, Source, ToolDef, ToolInput, ToolKind, DEFAULT_TIMEOUT_MS, MAX_TIMEOUT_MS,
};
pub use validate::{is_valid_name, FieldError, ValidationErrors, MAX_NAME_LEN};
