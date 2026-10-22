//! Which fields of an imported tool the user has edited, and how re-import
//! combines fresh spec values with those edits.

use serde_json::Value;

use crate::tool::{Overrides, ToolDef, ToolInput, ToolKind};

/// Fields that can be overridden, as recorded in `Overrides::fields`.
/// Params are tracked as one unit (`kind.params`).
pub const OVERRIDABLE_FIELDS: [&str; 11] = [
    "name",
    "description",
    "enabled",
    "response_shape",
    "kind.method",
    "kind.base_url",
    "kind.path_template",
    "kind.params",
    "kind.body_schema",
    "kind.auth_ref",
    "kind.timeout_ms",
];

impl ToolDef {
    /// The user-editable part of the tool.
    pub fn input(&self) -> ToolInput {
        ToolInput {
            name: self.name.clone(),
            description: self.description.clone(),
            kind: self.kind.clone(),
            enabled: self.enabled,
            response_shape: self.response_shape.clone(),
        }
    }
}

fn field_value(t: &ToolInput, field: &str) -> Value {
    let ToolKind::Http(h) = &t.kind;
    let v = match field {
        "name" => serde_json::to_value(&t.name),
        "description" => serde_json::to_value(&t.description),
        "enabled" => serde_json::to_value(t.enabled),
        "response_shape" => serde_json::to_value(&t.response_shape),
        "kind.method" => serde_json::to_value(h.method),
        "kind.base_url" => serde_json::to_value(&h.base_url),
        "kind.path_template" => serde_json::to_value(&h.path_template),
        "kind.params" => serde_json::to_value(&h.params),
        "kind.body_schema" => serde_json::to_value(&h.body_schema),
        "kind.auth_ref" => serde_json::to_value(h.auth_ref),
        "kind.timeout_ms" => serde_json::to_value(h.timeout_ms),
        other => unreachable!("unknown overridable field {other}"),
    };
    v.expect("IR fields serialize")
}

fn copy_field(from: &ToolInput, to: &mut ToolInput, field: &str) {
    let ToolKind::Http(src) = &from.kind;
    let ToolKind::Http(dst) = &mut to.kind;
    match field {
        "name" => to.name = from.name.clone(),
        "description" => to.description = from.description.clone(),
        "enabled" => to.enabled = from.enabled,
        "response_shape" => to.response_shape = from.response_shape.clone(),
        "kind.method" => dst.method = src.method,
        "kind.base_url" => dst.base_url = src.base_url.clone(),
        "kind.path_template" => dst.path_template = src.path_template.clone(),
        "kind.params" => dst.params = src.params.clone(),
        "kind.body_schema" => dst.body_schema = src.body_schema.clone(),
        "kind.auth_ref" => dst.auth_ref = src.auth_ref,
        "kind.timeout_ms" => dst.timeout_ms = src.timeout_ms,
        _ => {}
    }
}

/// Fields whose values differ between two versions of a tool.
pub fn changed_fields(old: &ToolInput, new: &ToolInput) -> Vec<&'static str> {
    OVERRIDABLE_FIELDS
        .iter()
        .copied()
        .filter(|f| field_value(old, f) != field_value(new, f))
        .collect()
}

/// Re-import merge: start from the spec's fresh `base`, then put back every
/// field the user overrode on `current`.
///
/// `kind.path_template` and `kind.params` travel together: path params must
/// match the template, so overriding either keeps both.
pub fn merge_overrides(current: &ToolInput, overrides: &Overrides, base: &ToolInput) -> ToolInput {
    let mut merged = base.clone();
    let has = |f: &str| overrides.fields.contains(f);
    for f in OVERRIDABLE_FIELDS {
        let coupled = matches!(f, "kind.params" | "kind.path_template")
            && (has("kind.params") || has("kind.path_template"));
        if has(f) || coupled {
            copy_field(current, &mut merged, f);
        }
    }
    merged
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn tool(desc: &str, path: &str) -> ToolInput {
        serde_json::from_value(json!({
            "name": "get_user",
            "description": desc,
            "kind": {
                "type": "http", "method": "GET", "base_url": "https://x.io",
                "path_template": path,
                "params": [{ "name": "id", "location": "path", "required": true }]
            }
        }))
        .unwrap()
    }

    #[test]
    fn detects_changes() {
        let a = tool("one", "/u/{id}");
        let mut b = a.clone();
        assert!(changed_fields(&a, &b).is_empty());
        b.description = "two".into();
        b.enabled = false;
        let ToolKind::Http(h) = &mut b.kind;
        h.timeout_ms = 5;
        assert_eq!(
            changed_fields(&a, &b),
            ["description", "enabled", "kind.timeout_ms"]
        );
    }

    #[test]
    fn merge_keeps_overridden_fields_only() {
        let mut current = tool("my words", "/u/{id}");
        current.enabled = false;
        let base = tool("spec words v2", "/users/{id}");
        let mut ov = Overrides::default();
        ov.fields.insert("description".into());

        let merged = merge_overrides(&current, &ov, &base);
        assert_eq!(merged.description, "my words");
        assert!(merged.enabled, "enabled wasn't overridden, spec value wins");
        let ToolKind::Http(h) = &merged.kind;
        assert_eq!(h.path_template, "/users/{id}");
    }

    #[test]
    fn params_and_path_travel_together() {
        let current = tool("d", "/u/{id}");
        let base = tool("d", "/users/{user}");
        let mut ov = Overrides::default();
        ov.fields.insert("kind.params".into());
        let merged = merge_overrides(&current, &ov, &base);
        let ToolKind::Http(h) = &merged.kind;
        assert_eq!(h.path_template, "/u/{id}");
        merged.validate().unwrap();
    }
}
