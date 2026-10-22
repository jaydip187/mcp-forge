//! Local `$ref` resolution for OpenAPI documents held as JSON values.

use serde_json::{Map, Value};

/// Follows chains of `$ref` (e.g. parameter → parameter) at most this many hops.
const MAX_HOPS: usize = 16;

/// Keys whose values are data, not schemas: never look for `$ref` inside them.
const DATA_KEYS: [&str; 5] = ["example", "examples", "default", "enum", "const"];

pub struct Resolver<'a> {
    root: &'a Value,
    /// How many `$ref`s may be expanded along one path before stopping.
    max_depth: usize,
}

impl<'a> Resolver<'a> {
    pub fn new(root: &'a Value, max_depth: usize) -> Self {
        Self { root, max_depth }
    }

    /// Looks up a local reference like `#/components/schemas/Pet`.
    pub fn lookup(&self, reference: &str) -> Option<&'a Value> {
        let pointer = reference.strip_prefix('#')?;
        let decoded = percent_decode(pointer);
        self.root.pointer(&decoded)
    }

    /// Follows `$ref` on a non-schema object (parameter, request body, path item).
    pub fn deref(&self, v: &'a Value) -> Result<&'a Value, String> {
        let mut cur = v;
        for _ in 0..MAX_HOPS {
            let Some(r) = cur.get("$ref").and_then(Value::as_str) else {
                return Ok(cur);
            };
            if !r.starts_with('#') {
                return Err(format!("external reference `{r}` isn't supported"));
            }
            cur = self
                .lookup(r)
                .ok_or_else(|| format!("reference `{r}` points nowhere"))?;
        }
        Err("reference chain is too long or circular".into())
    }

    /// Deep-copies a schema with every local `$ref` inlined. Cycles and refs
    /// beyond the depth cap are replaced by a short descriptive schema, so the
    /// result is always finite. OpenAPI 3.0 `nullable` becomes a type union.
    pub fn inline_schema(&self, v: &Value) -> Value {
        self.inline(v, &mut Vec::new())
    }

    fn inline(&self, v: &Value, stack: &mut Vec<String>) -> Value {
        match v {
            Value::Object(obj) => {
                if let Some(r) = obj.get("$ref").and_then(Value::as_str) {
                    return self.inline_ref(r, obj, stack);
                }
                let mut out = Map::with_capacity(obj.len());
                for (k, child) in obj {
                    let child = if DATA_KEYS.contains(&k.as_str()) {
                        child.clone()
                    } else {
                        self.inline(child, stack)
                    };
                    out.insert(k.clone(), child);
                }
                normalize_nullable(&mut out);
                Value::Object(out)
            }
            Value::Array(items) => {
                Value::Array(items.iter().map(|i| self.inline(i, stack)).collect())
            }
            other => other.clone(),
        }
    }

    fn inline_ref(&self, r: &str, obj: &Map<String, Value>, stack: &mut Vec<String>) -> Value {
        let name = r.rsplit('/').next().unwrap_or(r).to_owned();
        let stub = |description: String| {
            let mut m = Map::new();
            m.insert("description".into(), Value::String(description));
            Value::Object(m)
        };
        if !r.starts_with('#') {
            return stub(format!("External schema {r} (not resolved)."));
        }
        if stack.iter().any(|s| s == r) {
            let mut m = Map::new();
            m.insert("type".into(), Value::String("object".into()));
            m.insert(
                "description".into(),
                Value::String(format!("A nested {name} (recursive; not expanded).")),
            );
            return Value::Object(m);
        }
        if stack.len() >= self.max_depth {
            return stub(format!("A nested {name} (not expanded: nesting limit)."));
        }
        let Some(target) = self.lookup(r) else {
            return stub(format!("Unresolvable reference {r}."));
        };
        stack.push(r.to_owned());
        let mut resolved = self.inline(target, stack);
        stack.pop();
        // OpenAPI 3.1 allows keywords next to $ref (e.g. a local description); they win.
        if let Value::Object(m) = &mut resolved {
            for (k, v) in obj.iter().filter(|(k, _)| *k != "$ref") {
                let v = if DATA_KEYS.contains(&k.as_str()) {
                    v.clone()
                } else {
                    self.inline(v, stack)
                };
                m.insert(k.clone(), v);
            }
        }
        resolved
    }
}

/// `{"type": "string", "nullable": true}` → `{"type": ["string", "null"]}`.
fn normalize_nullable(m: &mut Map<String, Value>) {
    if m.get("nullable") != Some(&Value::Bool(true)) {
        m.remove("nullable");
        return;
    }
    m.remove("nullable");
    if let Some(Value::String(t)) = m.get("type").cloned() {
        m.insert(
            "type".into(),
            Value::Array(vec![Value::String(t), Value::String("null".into())]),
        );
    }
}

/// JSON-pointer fragments in refs may be percent-encoded (`%7B` for `{`).
fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap_or("");
            if let Ok(b) = u8::from_str_radix(hex, 16) {
                out.push(b);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn doc() -> Value {
        json!({
            "components": {
                "schemas": {
                    "Pet": { "type": "object", "properties": {
                        "id": { "type": "integer" },
                        "owner": { "$ref": "#/components/schemas/Person" },
                        "tag": { "$ref": "#/components/schemas/Tag", "description": "local wins" }
                    }},
                    "Person": { "type": "object", "properties": {
                        "pets": { "type": "array", "items": { "$ref": "#/components/schemas/Pet" } },
                        "nick": { "type": "string", "nullable": true }
                    }},
                    "Tag": { "type": "string", "description": "remote" },
                    "Deep1": { "properties": { "a": { "$ref": "#/components/schemas/Deep2" } } },
                    "Deep2": { "properties": { "b": { "$ref": "#/components/schemas/Deep3" } } },
                    "Deep3": { "properties": { "c": { "type": "string" } } }
                },
                "parameters": {
                    "A": { "$ref": "#/components/parameters/B" },
                    "B": { "name": "b", "in": "query" },
                    "Loop": { "$ref": "#/components/parameters/Loop" }
                }
            },
            "paths": { "/a~1b/{x}": { "get": {} } }
        })
    }

    #[test]
    fn inlines_and_marks_cycles() {
        let d = doc();
        let r = Resolver::new(&d, 8);
        let pet = r.inline_schema(&json!({ "$ref": "#/components/schemas/Pet" }));
        let owner = &pet["properties"]["owner"];
        assert_eq!(owner["type"], "object");
        let inner_pet = &owner["properties"]["pets"]["items"];
        assert_eq!(inner_pet["type"], "object");
        assert!(inner_pet["description"]
            .as_str()
            .unwrap()
            .contains("recursive"));
        assert_eq!(pet["properties"]["tag"]["description"], "local wins");
        assert_eq!(
            owner["properties"]["nick"]["type"],
            json!(["string", "null"])
        );
        assert!(serde_json::to_string(&pet).unwrap().len() < 2000);
    }

    #[test]
    fn depth_cap() {
        let d = doc();
        let r = Resolver::new(&d, 2);
        let s = r.inline_schema(&json!({ "$ref": "#/components/schemas/Deep1" }));
        let b = &s["properties"]["a"]["properties"]["b"];
        assert!(
            b["description"].as_str().unwrap().contains("nesting limit"),
            "{s}"
        );
    }

    #[test]
    fn deref_chains_and_errors() {
        let d = doc();
        let r = Resolver::new(&d, 8);
        let p = r.deref(&d["components"]["parameters"]["A"]).unwrap();
        assert_eq!(p["name"], "b");
        assert!(r.deref(&d["components"]["parameters"]["Loop"]).is_err());
        assert!(r.deref(&json!({ "$ref": "other.yaml#/X" })).is_err());
        assert!(r.deref(&json!({ "$ref": "#/nope" })).is_err());
        // Key "/a~1b/{x}": "/" escapes to ~1, "~" to ~0, braces percent-encoded.
        assert!(r.lookup("#/paths/~1a~01b~1%7Bx%7D").is_some());
        assert!(r.lookup("#/paths/~1a~1b~1%7Bx%7D").is_none());
    }

    #[test]
    fn examples_are_not_resolved() {
        let d = doc();
        let r = Resolver::new(&d, 8);
        let s = r.inline_schema(&json!({ "type": "object", "example": { "$ref": "keep me" } }));
        assert_eq!(s["example"]["$ref"], "keep me");
    }
}
