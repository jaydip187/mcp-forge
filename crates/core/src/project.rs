use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::validate::ValidationErrors;

pub const MAX_PROJECT_NAME_LEN: usize = 100;
pub const MAX_SLUG_LEN: usize = 64;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Project {
    pub id: Uuid,
    pub name: String,
    /// URL-safe identifier used in `/mcp/{slug}` and `stdio --project`.
    pub slug: String,
    /// Curation and response shaping; changed via the settings endpoint.
    #[serde(default)]
    pub settings: crate::settings::ProjectSettings,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Create/replace payload. `slug` defaults to `slugify(name)`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectInput {
    pub name: String,
    #[serde(default)]
    pub slug: Option<String>,
}

impl ProjectInput {
    /// Validates and returns `(name, slug)` with the slug resolved.
    pub fn validate(&self) -> Result<(String, String), ValidationErrors> {
        let mut errs = ValidationErrors::default();
        let name = self.name.trim().to_owned();
        if name.is_empty() || name.chars().count() > MAX_PROJECT_NAME_LEN {
            errs.push(
                "name",
                format!("must be 1–{MAX_PROJECT_NAME_LEN} characters"),
            );
        }
        let slug = match &self.slug {
            Some(s) => s.clone(),
            None => slugify(&name),
        };
        if !is_valid_slug(&slug) {
            let msg = if self.slug.is_none() {
                "could not derive a slug from the name; provide one (lowercase letters, digits, single dashes)"
            } else {
                "must be lowercase letters and digits separated by single dashes, at most 64 characters"
            };
            errs.push("slug", msg);
        }
        errs.into_result().map(|()| (name, slug))
    }
}

/// `[a-z0-9]+(-[a-z0-9]+)*`, at most 64 chars.
pub fn is_valid_slug(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= MAX_SLUG_LEN
        && s.split('-').all(|part| {
            !part.is_empty()
                && part
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
        })
}

/// Lowercases ASCII alphanumerics and collapses everything else into single dashes.
pub fn slugify(name: &str) -> String {
    let mut out = String::new();
    for c in name.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if !out.is_empty() && !out.ends_with('-') {
            out.push('-');
        }
    }
    let mut out = out.trim_end_matches('-').to_owned();
    if out.len() > MAX_SLUG_LEN {
        out.truncate(MAX_SLUG_LEN);
        out = out.trim_end_matches('-').to_owned();
    }
    out
}

#[cfg(test)]
#[path = "tests/project.rs"]
mod tests;
