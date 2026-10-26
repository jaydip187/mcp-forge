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
