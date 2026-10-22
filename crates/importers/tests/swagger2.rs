use std::collections::BTreeMap;

use forge_core::{ParamLocation, ToolKind};
use forge_importers::openapi3::{BuiltOperation, Options};
use forge_importers::{import_any, parse_document};
use serde_json::{json, Value};

const BASE: &str = "https://petstore.swagger.io/v2";

/// What makes two imports "equivalent": the tool an agent would see and the
/// request it would send.
fn signature(o: &BuiltOperation) -> Value {
    let ToolKind::Http(h) = &o.tool.kind;
    let mut params: Vec<Value> = h
        .params
        .iter()
        .map(|p| {
            json!({
                "name": p.name, "wire": p.wire(), "in": p.location.as_str(),
                "required": p.required, "type": p.schema.get("type"),
            })
        })
        .collect();
    params.sort_by_key(|p| p.to_string());
    json!({
        "name": o.tool.name,
        "method": h.method,
        "base_url": h.base_url,
        "path": h.path_template,
        "params": params,
        "whole_body": h.body_schema.is_some(),
        "supported": o.supported,
        "needs_auth": o.needs_auth,
    })
}

fn sigs(doc: &Value, opts: &Options) -> BTreeMap<String, Value> {
    import_any(doc, opts)
        .unwrap()
        .operations
        .iter()
        .map(|o| (o.key.clone(), signature(o)))
        .collect()
}

#[test]
fn swagger2_petstore_matches_openapi3_petstore() {
    let v2 = parse_document(include_str!("fixtures/petstore2.json")).unwrap();
    let v3 = parse_document(include_str!("fixtures/petstore3.json")).unwrap();
    let a = sigs(&v2, &Options::default());
    let b = sigs(
        &v3,
        &Options {
            base_url: Some(BASE.into()),
            ..Default::default()
        },
    );

    // Operations the two published specs themselves define differently
    // (checked against the fixtures), not conversion differences:
    let differs_in_source = [
        // 2.0: form fields; 3.0: query params.
        "POST /pet/{petId}",
        // 2.0: multipart upload; 3.0: raw octet-stream body.
        "POST /pet/{petId}/uploadImage",
        // 2.0: `status` is an array (collectionFormat multi); 3.0: a single string.
        "GET /pet/findByStatus",
        // 2.0: username/password required; 3.0: optional.
        "GET /user/login",
    ];

    let common: Vec<&String> = a.keys().filter(|k| b.contains_key(*k)).collect();
    assert!(
        common.len() >= 15,
        "only {} common operations",
        common.len()
    );
    let mut mismatches = Vec::new();
    for k in common
        .iter()
        .filter(|k| !differs_in_source.contains(&k.as_str()))
    {
        if a[*k] != b[*k] {
            mismatches.push(format!("{k}\n  2.0: {}\n  3.0: {}", a[*k], b[*k]));
        }
    }
    assert!(mismatches.is_empty(), "{}", mismatches.join("\n"));
}

#[test]
fn swagger2_petstore_details() {
    let v2 = parse_document(include_str!("fixtures/petstore2.json")).unwrap();
    let imported = import_any(&v2, &Options::default()).unwrap();
    assert_eq!(imported.info.kind, "swagger2");
    assert_eq!(imported.info.openapi, "2.0");
    assert_eq!(imported.info.base_url.as_deref(), Some(BASE));
    assert_eq!(imported.operations.len(), 20);
    let schemes: Vec<_> = imported
        .info
        .security_schemes
        .iter()
        .map(|s| s.name.as_str())
        .collect();
    assert_eq!(schemes, ["api_key", "petstore_auth"]);

    let op = |k: &str| imported.operations.iter().find(|o| o.key == k).unwrap();

    // Body param with $ref'd definition is flattened like a 3.x JSON body.
    let add = op("POST /pet");
    assert!(add.supported, "{:?}", add.warnings);
    let ToolKind::Http(h) = &add.tool.kind;
    let body: Vec<_> = h
        .params
        .iter()
        .filter(|p| p.location == ParamLocation::Body)
        .map(|p| p.name.as_str())
        .collect();
    assert!(
        body.contains(&"photoUrls") && body.contains(&"category"),
        "{body:?}"
    );

    // Array body → one whole-body arg.
    let many = op("POST /user/createWithList");
    let ToolKind::Http(h) = &many.tool.kind;
    assert_eq!(h.body_schema.as_ref().unwrap()["type"], "array");

    // collectionFormat multi = repeated keys, which is what the executor sends.
    let by_tags = op("GET /pet/findByTags");
    assert!(by_tags.supported);
    assert!(
        !by_tags.warnings.iter().any(|w| w.contains("delimited")),
        "{:?}",
        by_tags.warnings
    );
    // Multipart upload is unsupported.
    // Its form fields are all optional in this spec, so the multipart body is
    // skipped with a warning and the operation still imports.
    let upload = op("POST /pet/{petId}/uploadImage");
    assert!(upload.supported, "{:?}", upload.warnings);
    assert!(
        upload
            .warnings
            .iter()
            .any(|w| w.contains("multipart/form-data")),
        "{:?}",
        upload.warnings
    );
    // Optional form body is skipped, the rest still imports.
    let form = op("POST /pet/{petId}");
    assert!(form.supported, "{:?}", form.warnings);
    assert!(form
        .warnings
        .iter()
        .any(|w| w.contains("x-www-form-urlencoded")));
}

#[test]
fn csv_arrays_warn_about_delimited_values() {
    let doc = json!({
        "swagger": "2.0", "info": { "title": "C", "version": "1" }, "host": "c.io",
        "paths": { "/s": { "get": { "operationId": "s", "parameters": [
            { "name": "ids", "in": "query", "type": "array", "items": { "type": "integer" } }
        ]}}}
    });
    let imported = import_any(&doc, &Options::default()).unwrap();
    let o = &imported.operations[0];
    assert!(o.supported);
    assert!(
        o.warnings.iter().any(|w| w.contains("delimited")),
        "{:?}",
        o.warnings
    );
}

#[test]
fn swagger2_yaml_without_host_uses_base_path() {
    let doc = parse_document(
        "swagger: '2.0'\ninfo: {title: Y, version: '1'}\nbasePath: /api\npaths:\n  /ping:\n    get:\n      operationId: ping\n      responses:\n        200: {description: ok}\n",
    )
    .unwrap();
    let imported = import_any(
        &doc,
        &Options {
            source_url: Some("http://localhost:9000/spec.yaml".into()),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(
        imported.info.base_url.as_deref(),
        Some("http://localhost:9000/api")
    );
    assert!(imported.operations[0].supported);
}
