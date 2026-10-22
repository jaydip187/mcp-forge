use forge_core::{ArgTarget, HttpMethod, ParamLocation, ToolKind};
use forge_importers::openapi3::{import, BuiltOperation, Imported, Options};
use forge_importers::parse_document;
use serde_json::{json, Value};

fn petstore() -> Value {
    parse_document(include_str!("fixtures/petstore3.json")).unwrap()
}

fn op<'a>(imported: &'a Imported, key: &str) -> &'a BuiltOperation {
    imported
        .operations
        .iter()
        .find(|o| o.key == key)
        .unwrap_or_else(|| panic!("no operation {key}"))
}

fn args(o: &BuiltOperation) -> Vec<String> {
    let ToolKind::Http(h) = &o.tool.kind;
    h.derive_input()
        .bindings
        .into_iter()
        .map(|b| b.arg)
        .collect()
}

#[test]
fn petstore_imports_cleanly() {
    let opts = Options {
        source_url: Some("https://petstore3.swagger.io/api/v3/openapi.json".into()),
        ..Default::default()
    };
    let imported = import(&petstore(), &opts).unwrap();
    assert_eq!(imported.info.openapi, "3.0.4");
    assert_eq!(imported.info.servers, ["/api/v3"]);
    assert_eq!(
        imported.info.base_url.as_deref(),
        Some("https://petstore3.swagger.io/api/v3")
    );
    assert_eq!(imported.operations.len(), 19);

    let unsupported: Vec<_> = imported
        .operations
        .iter()
        .filter(|o| !o.supported)
        .map(|o| (&o.key, &o.warnings))
        .collect();
    assert!(unsupported.is_empty(), "{unsupported:#?}");
    for o in &imported.operations {
        o.tool
            .validate()
            .unwrap_or_else(|e| panic!("{}: {e}", o.key));
    }

    // operationId names.
    let get_pet = op(&imported, "GET /pet/{petId}");
    assert_eq!(get_pet.tool.name, "getPetById");
    assert_eq!(get_pet.tags, ["pet"]);
    assert!(get_pet.needs_auth);
    let ToolKind::Http(h) = &get_pet.tool.kind;
    assert_eq!(h.method, HttpMethod::Get);
    assert_eq!(h.base_url, "https://petstore3.swagger.io/api/v3");
    assert_eq!(h.params[0].schema["type"], "integer");

    // JSON body flattened into fields, with $ref'd Category inlined.
    let add = op(&imported, "POST /pet");
    let ToolKind::Http(h) = &add.tool.kind;
    let body: Vec<_> = h
        .params
        .iter()
        .filter(|p| p.location == ParamLocation::Body)
        .collect();
    let names: Vec<_> = body.iter().map(|p| p.name.as_str()).collect();
    assert!(
        names.contains(&"name") && names.contains(&"photoUrls") && names.contains(&"category"),
        "{names:?}"
    );
    let required: Vec<_> = body
        .iter()
        .filter(|p| p.required)
        .map(|p| p.name.as_str())
        .collect();
    assert_eq!(required.len(), 2, "{required:?}"); // name, photoUrls
    let category = body.iter().find(|p| p.name == "category").unwrap();
    assert_eq!(category.schema["type"], "object");
    assert!(category.schema["properties"]["name"].is_object());

    // Optional non-JSON body (octet-stream upload) is skipped with a warning, not fatal.
    let upload = op(&imported, "POST /pet/{petId}/uploadImage");
    assert!(upload.supported);
    assert!(
        upload.warnings.iter().any(|w| w.contains("octet-stream")),
        "{:?}",
        upload.warnings
    );
}

#[test]
fn relative_server_without_source_needs_base_url() {
    let imported = import(&petstore(), &Options::default()).unwrap();
    assert_eq!(imported.info.base_url, None);
    assert!(imported.operations.iter().all(|o| !o.supported));
    assert!(imported.operations[0].warnings[0].contains("no base URL"));

    let with_base = import(
        &petstore(),
        &Options {
            base_url: Some("https://pets.example.com/api/v3/".into()),
            ..Default::default()
        },
    )
    .unwrap();
    assert!(with_base.operations.iter().all(|o| o.supported));
    let ToolKind::Http(h) = &with_base.operations[0].tool.kind;
    assert_eq!(h.base_url, "https://pets.example.com/api/v3");
}

fn synthetic() -> Value {
    json!({
        "openapi": "3.1.0",
        "info": { "title": "Synthetic", "version": "1" },
        "servers": [{ "url": "https://{region}.api.example.com/v{ver}", "variables": {
            "region": { "default": "eu" }, "ver": { "default": "2" }
        }}],
        "security": [{ "key": [] }],
        "components": {
            "securitySchemes": {
                "key": { "type": "apiKey", "in": "header", "name": "X-API-Key" },
                "oauth": { "type": "oauth2", "flows": {} }
            },
            "parameters": {
                "Id": { "name": "id", "in": "path", "required": true, "schema": { "$ref": "#/components/schemas/Id" } }
            },
            "schemas": {
                "Id": { "type": "string", "format": "uuid" },
                "Node": { "type": "object", "properties": {
                    "id": { "type": "string" },
                    "label": { "type": ["string", "null"] },
                    "children": { "type": "array", "items": { "$ref": "#/components/schemas/Node" } },
                    "created": { "type": "string", "readOnly": true }
                }, "required": ["id"] },
                "Shape": { "oneOf": [ { "$ref": "#/components/schemas/Node" }, { "type": "string" } ] }
            }
        },
        "paths": {
            "/nodes/{id}": {
                "parameters": [{ "$ref": "#/components/parameters/Id" }],
                "put": {
                    "operationId": "nodes/update",
                    "requestBody": { "required": true, "content": {
                        "application/json": { "schema": { "$ref": "#/components/schemas/Node" } }
                    }}
                },
                "get": {
                    "security": [],
                    "parameters": [
                        { "name": "filter[name]", "in": "query", "schema": { "type": "string" } },
                        { "name": "$top", "in": "query", "schema": { "type": "integer" } },
                        { "name": "Accept", "in": "header", "schema": { "type": "string" } }
                    ]
                }
            },
            "/shapes": {
                "post": {
                    "operationId": "nodes/update",
                    "requestBody": { "content": { "application/vnd.api+json": {
                        "schema": { "$ref": "#/components/schemas/Shape" }
                    }}}
                }
            },
            "/upload": {
                "post": {
                    "requestBody": { "required": true, "content": { "multipart/form-data": { "schema": { "type": "object" } } } }
                }
            },
            "/session": {
                "get": { "parameters": [{ "name": "sid", "in": "cookie", "required": true }] }
            },
            "/loose/{a}/{b}": {
                "get": { "parameters": [{ "name": "a", "in": "path", "required": true }] }
            }
        }
    })
}

#[test]
fn synthetic_covers_refs_collisions_naming() {
    let imported = import(&synthetic(), &Options::default()).unwrap();
    assert_eq!(
        imported.info.base_url.as_deref(),
        Some("https://eu.api.example.com/v2")
    );
    let schemes: Vec<_> = imported
        .info
        .security_schemes
        .iter()
        .map(|s| s.description.as_str())
        .collect();
    assert!(
        schemes.contains(&"API key in header X-API-Key"),
        "{schemes:?}"
    );

    // Collision: path `id` vs body `id` → prefixed only for those two.
    let update = op(&imported, "PUT /nodes/{id}");
    assert!(update.supported, "{:?}", update.warnings);
    assert_eq!(update.tool.name, "nodes_update");
    assert!(update.needs_auth);
    let a = args(update);
    assert!(
        a.contains(&"path_id".into()) && a.contains(&"body_id".into()),
        "{a:?}"
    );
    assert!(a.contains(&"label".into()) && a.contains(&"children".into()));
    assert!(
        !a.contains(&"created".into()),
        "readOnly fields aren't sent"
    );
    let ToolKind::Http(h) = &update.tool.kind;
    let children = h.params.iter().find(|p| p.name == "children").unwrap();
    // Node is already being expanded, so its self-reference becomes a stub.
    assert!(children.schema["items"]["description"]
        .as_str()
        .unwrap()
        .contains("recursive"));
    assert_eq!(h.params[0].schema["format"], "uuid"); // path param via $ref'd parameter + schema

    // Wire names for unsafe query names; reserved headers dropped; `security: []` → no auth.
    let get = op(&imported, "GET /nodes/{id}");
    assert!(!get.needs_auth);
    let ToolKind::Http(h) = &get.tool.kind;
    let q: Vec<_> = h
        .params
        .iter()
        .filter(|p| p.location == ParamLocation::Query)
        .map(|p| (p.name.as_str(), p.wire()))
        .collect();
    assert_eq!(q, [("filter_name", "filter[name]"), ("top", "$top")]);
    assert!(h.params.iter().all(|p| p.location != ParamLocation::Header));
    assert_eq!(get.tool.name, "get_nodes_id");

    // Duplicate operationId → deduped; composed +json schema → whole body arg.
    let shapes = op(&imported, "POST /shapes");
    assert_eq!(shapes.tool.name, "nodes_update_2");
    let ToolKind::Http(h) = &shapes.tool.kind;
    assert!(h.body_schema.as_ref().unwrap()["oneOf"].is_array());
    let d = h.derive_input();
    assert_eq!(d.bindings[0].target, ArgTarget::WholeBody);

    // Unsupported: required multipart body, required cookie.
    let upload = op(&imported, "POST /upload");
    assert!(!upload.supported);
    assert!(
        upload.warnings[0].contains("multipart/form-data"),
        "{:?}",
        upload.warnings
    );
    let session = op(&imported, "GET /session");
    assert!(!session.supported);
    assert!(session.warnings[0].contains("cookie"));

    // Undeclared placeholder becomes a required param.
    let loose = op(&imported, "GET /loose/{a}/{b}");
    assert!(loose.supported, "{:?}", loose.warnings);
    assert!(loose.warnings.iter().any(|w| w.contains("{b}")));
    assert_eq!(args(loose), ["a", "b"]);
}

#[test]
fn swagger2_and_garbage_are_rejected_clearly() {
    let e = import(&json!({ "swagger": "2.0" }), &Options::default()).unwrap_err();
    assert!(e.to_string().contains("Swagger 2.0"));
    let e = import(&json!({ "hello": 1 }), &Options::default()).unwrap_err();
    assert!(e.to_string().contains("OpenAPI 3.x"));
}

#[test]
fn yaml_spec_with_numeric_keys() {
    let doc = parse_document(
        "openapi: 3.0.0\ninfo: {title: Y, version: '1'}\nservers: [{url: 'https://y.example.com'}]\npaths:\n  /ping:\n    get:\n      operationId: ping\n      responses:\n        200: {description: ok}\n",
    )
    .unwrap();
    let imported = import(&doc, &Options::default()).unwrap();
    assert_eq!(imported.operations[0].tool.name, "ping");
    assert!(imported.operations[0].supported);
}

/// Stress test on a large real spec. Run with
/// `MCP_FORGE_LARGE_SPEC=path/to/api.github.com.json cargo test -p forge-importers -- --ignored`.
#[test]
#[ignore = "needs MCP_FORGE_LARGE_SPEC"]
fn large_real_spec() {
    let path = std::env::var("MCP_FORGE_LARGE_SPEC").expect("set MCP_FORGE_LARGE_SPEC");
    let text = std::fs::read_to_string(&path).unwrap();
    let started = std::time::Instant::now();
    let doc = parse_document(&text).unwrap();
    let imported = import(&doc, &Options::default()).unwrap();
    let elapsed = started.elapsed();

    let total = imported.operations.len();
    let supported = imported.operations.iter().filter(|o| o.supported).count();
    let names: std::collections::HashSet<_> =
        imported.operations.iter().map(|o| &o.tool.name).collect();
    let biggest = imported
        .operations
        .iter()
        .map(|o| serde_json::to_string(&o.tool).unwrap().len())
        .max()
        .unwrap_or(0);
    println!(
        "{path}: {} {} — {total} ops, {supported} supported, largest tool {biggest} bytes, {elapsed:?}",
        imported.info.title, imported.info.openapi
    );
    for o in imported.operations.iter().filter(|o| !o.supported).take(10) {
        println!("  unsupported {}: {:?}", o.key, o.warnings);
    }
    assert!(total > 0);
    assert_eq!(names.len(), total, "tool names must be unique");
    for o in imported.operations.iter().filter(|o| o.supported) {
        o.tool
            .validate()
            .unwrap_or_else(|e| panic!("{}: {e}", o.key));
    }
    assert!(
        supported * 100 / total >= 90,
        "only {supported}/{total} supported"
    );
}
