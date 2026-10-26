//! Per-project curation: which tools agents see, and how responses are shaped.

use serde::{Deserialize, Serialize};

use crate::tool::{HttpMethod, ToolDef, ToolKind};
use crate::validate::ValidationErrors;

pub const DEFAULT_MAX_RESPONSE_CHARS: u32 = 50_000;
pub const MIN_MAX_RESPONSE_CHARS: u32 = 500;
pub const MAX_MAX_RESPONSE_CHARS: u32 = 2_000_000;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ProjectSettings {
    /// Expose only GET, HEAD and OPTIONS tools.
    pub read_only: bool,
    /// If not empty, expose only tools with at least one of these tags.
    pub include_tags: Vec<String>,
    /// Hide tools with any of these tags.
    pub exclude_tags: Vec<String>,
    /// If not empty, expose only tools whose path matches one of these globs.
    pub include_paths: Vec<String>,
    /// Hide tools whose path matches any of these globs.
    pub exclude_paths: Vec<String>,
    /// Remove `null` fields from JSON responses.
    pub strip_nulls: bool,
    /// Longest response (in characters) handed to the agent.
    pub max_response_chars: u32,
    /// List only `search_tools` and `call_tool`; agents find tools by searching.
    pub tool_search: bool,
}

impl Default for ProjectSettings {
    fn default() -> Self {
        Self {
            read_only: false,
            include_tags: vec![],
            exclude_tags: vec![],
            include_paths: vec![],
            exclude_paths: vec![],
            strip_nulls: false,
            max_response_chars: DEFAULT_MAX_RESPONSE_CHARS,
            tool_search: false,
        }
    }
}

/// Why a tool is hidden from agents, if it is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Hidden {
    Disabled,
    ReadOnly,
    Tag,
    Path,
}

impl ProjectSettings {
    pub fn validate(&self) -> Result<(), ValidationErrors> {
        let mut errs = ValidationErrors::default();
        if !(MIN_MAX_RESPONSE_CHARS..=MAX_MAX_RESPONSE_CHARS).contains(&self.max_response_chars) {
            errs.push(
                "max_response_chars",
                format!("must be between {MIN_MAX_RESPONSE_CHARS} and {MAX_MAX_RESPONSE_CHARS}"),
            );
        }
        for (field, list) in [
            ("include_tags", &self.include_tags),
            ("exclude_tags", &self.exclude_tags),
            ("include_paths", &self.include_paths),
            ("exclude_paths", &self.exclude_paths),
        ] {
            if list.len() > 100 {
                errs.push(field, "at most 100 entries");
            }
            for (i, v) in list.iter().enumerate() {
                if v.trim().is_empty() || v.len() > 256 {
                    errs.push(format!("{field}[{i}]"), "must be 1–256 characters");
                }
            }
        }
        errs.into_result()
    }

    /// `None` if agents can see the tool, else the first reason it's hidden.
    pub fn hidden_reason(&self, tool: &ToolDef) -> Option<Hidden> {
        if !tool.enabled {
            return Some(Hidden::Disabled);
        }
        let http = match &tool.kind {
            ToolKind::Http(h) => Some(h),
            // Curated queries only read.
            ToolKind::Sql(_) => None,
        };
        if self.read_only
            && http.is_some_and(|h| {
                !matches!(
                    h.method,
                    HttpMethod::Get | HttpMethod::Head | HttpMethod::Options
                )
            })
        {
            return Some(Hidden::ReadOnly);
        }
        let has_tag = |list: &[String]| {
            tool.tags
                .iter()
                .any(|t| list.iter().any(|l| l.eq_ignore_ascii_case(t)))
        };
        if (!self.include_tags.is_empty() && !has_tag(&self.include_tags))
            || has_tag(&self.exclude_tags)
        {
            return Some(Hidden::Tag);
        }
        // Path filters only concern HTTP tools.
        if let Some(h) = http {
            let matches = |list: &[String]| list.iter().any(|g| path_matches(g, &h.path_template));
            if (!self.include_paths.is_empty() && !matches(&self.include_paths))
                || matches(&self.exclude_paths)
            {
                return Some(Hidden::Path);
            }
        }
        None
    }

    pub fn exposes(&self, tool: &ToolDef) -> bool {
        self.hidden_reason(tool).is_none()
    }
}

/// Path filter, same rules as the UI's import filter: with `*`, `**` or `?`
/// it's a whole-path glob (`*`/`?` within one segment, `**` across);
/// without wildcards it's a case-insensitive substring match.
pub fn path_matches(pattern: &str, path: &str) -> bool {
    let p = pattern.trim();
    if p.is_empty() {
        return true;
    }
    if !p.contains(['*', '?']) {
        return path.to_lowercase().contains(&p.to_lowercase());
    }
    let pat: Vec<char> = p.to_lowercase().chars().collect();
    let s: Vec<char> = path.to_lowercase().chars().collect();
    glob(&pat, &s)
}

fn glob(p: &[char], s: &[char]) -> bool {
    match p.first() {
        None => s.is_empty(),
        Some('*') if p.get(1) == Some(&'*') => (0..=s.len()).any(|i| glob(&p[2..], &s[i..])),
        Some('*') => {
            let seg_end = s.iter().position(|c| *c == '/').unwrap_or(s.len());
            (0..=seg_end).any(|i| glob(&p[1..], &s[i..]))
        }
        Some('?') => s.first().is_some_and(|c| *c != '/') && glob(&p[1..], &s[1..]),
        Some(c) => s.first() == Some(c) && glob(&p[1..], &s[1..]),
    }
}

#[cfg(test)]
#[path = "../tests/unit/settings.rs"]
mod tests;
