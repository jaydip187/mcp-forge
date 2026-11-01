//! Stage 23: Postman collections, HAR captures and curl commands become
//! tools through the OpenAPI importer, without carrying secrets along.

use forge_core::{ParamLocation, ToolKind};
use forge_importers::openapi3::{BuiltOperation, Imported, Options};
use forge_importers::{import_text, parse_document};
use serde_json::json;

fn import(text: &str) -> (Imported, String) {
    let (imported, converted) = import_text(text, &Options::default()).unwrap();
    (imported, converted.expect("converted"))
}

fn op<'a>(i: &'a Imported, key: &str) -> &'a BuiltOperation {
    i.operations
        .iter()
        .find(|o| o.key == key)
        .unwrap_or_else(|| {
            panic!(
                "no {key} in {:?}",
                i.operations.iter().map(|o| &o.key).collect::<Vec<_>>()
            )
        })
}

fn http(o: &BuiltOperation) -> &forge_core::HttpSpec {
    match &o.tool.kind {
        ToolKind::Http(h) => h,
        _ => panic!("http"),
    }
}

/// Names in the importer's order (body properties come out alphabetically).
fn param_names(o: &BuiltOperation, loc: ParamLocation) -> Vec<String> {
    http(o)
        .params
        .iter()
        .filter(|p| p.location == loc)
        .map(|p| p.name.clone())
        .collect()
}

/// Body fields come from JSON objects, which keep their written order when
/// serde_json's `preserve_order` is on (bson enables it in workspace builds).
fn body_names(o: &BuiltOperation) -> Vec<String> {
    let mut names = param_names(o, ParamLocation::Body);
    names.sort();
    names
}

const POSTMAN: &str = r#"{
  "info": { "_postman_id": "x", "name": "Shop API", "schema": "https://schema.getpostman.com/json/collection/v2.1.0/collection.json" },
  "auth": { "type": "bearer", "bearer": [{ "key": "token", "value": "SECRET-BEARER" }] },
  "variable": [
    { "key": "baseUrl", "value": "https://api.shop.example" },
    { "key": "apiToken", "value": "SECRET-VAR" }
  ],
  "item": [
    { "name": "Orders", "item": [
      { "name": "List orders",
        "request": { "method": "GET",
          "header": [{ "key": "X-Api-Version", "value": "2024-01" }, { "key": "X-Api-Key", "value": "SECRET-HEADER" }],
          "url": { "raw": "{{baseUrl}}/orders?status=open&limit=20", "host": ["{{baseUrl}}"], "path": ["orders"],
                   "query": [{ "key": "status", "value": "open" }, { "key": "limit", "value": "20" }, { "key": "debug", "value": "1", "disabled": true }] } } },
      { "name": "Get order",
        "request": { "method": "GET", "description": "One order by id.",
          "url": { "raw": "{{baseUrl}}/orders/:orderId", "path": ["orders", ":orderId"],
                   "variable": [{ "key": "orderId", "value": "42", "description": "The order" }] } } },
      { "name": "Create order",
        "request": { "method": "POST",
          "header": [{ "key": "Content-Type", "value": "application/json" }],
          "body": { "mode": "raw", "raw": "{\"item\": \"SECRET-BODY\", \"qty\": 2}", "options": { "raw": { "language": "json" } } },
          "url": "{{baseUrl}}/orders" } }
    ] },
    { "name": "Upload", "request": { "method": "POST", "body": { "mode": "formdata", "formdata": [] }, "url": "{{baseUrl}}/files" } },
    { "name": "Public ping", "request": { "method": "GET", "auth": { "type": "noauth" }, "url": "https://status.shop.example/ping" } }
  ]
}"#;

#[test]
fn postman_collection() {
    let (i, converted) = import(POSTMAN);
    assert_eq!(i.info.kind, "postman");
    assert_eq!(i.info.title, "Shop API");
    for secret in [
        "SECRET-BEARER",
        "SECRET-VAR",
        "SECRET-HEADER",
        "SECRET-BODY",
    ] {
        assert!(!converted.contains(secret), "{secret} leaked");
    }

    let list = op(&i, "GET /orders");
    assert_eq!(list.tool.name, "List_orders");
    assert_eq!(list.tool.tags, ["Orders"]);
    let h = http(list);
    assert_eq!(h.base_url, "https://api.shop.example");
    assert_eq!(h.path_template, "/orders");
    assert_eq!(
        param_names(list, ParamLocation::Query),
        ["status", "limit"],
        "disabled ones skipped"
    );
    assert_eq!(
        param_names(list, ParamLocation::Header),
        ["X-Api-Version"],
        "secret header became a scheme"
    );
    let status = h.params.iter().find(|p| p.name == "status").unwrap();
    assert_eq!(status.schema["default"], "open");
    assert!(list.needs_auth);

    let get = op(&i, "GET /orders/{orderId}");
    assert_eq!(http(get).path_template, "/orders/{orderId}");
    assert_eq!(get.tool.description, "Get order\n\nOne order by id.");

    let create = op(&i, "POST /orders");
    assert!(create.supported);
    let body = body_names(create);
    assert_eq!(body, ["item", "qty"]);
    assert_eq!(
        get.op_id(),
        "Get order",
        "re-import matches on the request name"
    );

    assert!(
        !op(&i, "POST /files").supported,
        "multipart bodies aren't supported"
    );
    let ping = op(&i, "GET /ping");
    assert!(!ping.needs_auth, "noauth");
    assert_eq!(http(ping).base_url, "https://status.shop.example");

    let schemes: Vec<&str> = i
        .info
        .security_schemes
        .iter()
        .map(|s| s.name.as_str())
        .collect();
    assert!(
        schemes.contains(&"bearerAuth") && schemes.contains(&"header_X_Api_Key"),
        "{schemes:?}"
    );

    // The converted text imports to the same operations (commit and re-import use it).
    let again = import_text(&converted, &Options::default()).unwrap();
    assert!(again.1.is_none(), "already OpenAPI");
    assert_eq!(again.0.info.kind, "postman");
    assert_eq!(again.0.operations.len(), i.operations.len());
}

#[test]
fn postman_host_variable_without_value() {
    let doc = json!({
        "info": { "name": "Vars", "schema": "https://schema.getpostman.com/json/collection/v2.1.0/collection.json" },
        "item": [{ "name": "Me", "request": { "method": "GET", "url": "{{host}}/v1/me/{{userId}}" } }]
    });
    let (i, _) = import(&doc.to_string());
    let me = op(&i, "GET /v1/me/{userId}");
    assert_eq!(http(me).path_template, "/v1/me/{userId}");
    assert!(!me.supported, "no base URL until the user gives one");
    let with_base = import_text(
        &doc.to_string(),
        &Options {
            base_url: Some("https://api.example.com".into()),
            source_url: None,
        },
    )
    .unwrap()
    .0;
    assert!(op(&with_base, "GET /v1/me/{userId}").supported);
}

const HAR: &str = r#"{ "log": { "version": "1.2", "entries": [
  { "request": { "method": "GET", "url": "https://app.example.com/index.html", "headers": [] },
    "response": { "status": 200, "content": { "mimeType": "text/html" } } },
  { "request": { "method": "GET", "url": "https://app.example.com/static/app.js", "headers": [] },
    "response": { "status": 200, "content": { "mimeType": "application/javascript" } } },
  { "request": { "method": "GET", "url": "https://api.example.com/v1/projects/42/tasks?page=2&access_token=SECRET-QUERY",
                 "headers": [{ "name": "Cookie", "value": "session=SECRET-COOKIE" }, { "name": "Authorization", "value": "Bearer SECRET-AUTH" }] },
    "response": { "status": 200, "content": { "mimeType": "application/json" } } },
  { "request": { "method": "GET", "url": "https://api.example.com/v1/projects/43/tasks?page=3", "headers": [] },
    "response": { "status": 200, "content": { "mimeType": "application/json; charset=utf-8" } } },
  { "request": { "method": "POST", "url": "https://api.example.com/v1/login",
                 "headers": [{ "name": "Content-Type", "value": "application/json" }],
                 "postData": { "mimeType": "application/json", "text": "{\"user\":\"ada\",\"password\":\"SECRET-PASSWORD\"}" } },
    "response": { "status": 200, "content": { "mimeType": "application/json" } } },
  { "request": { "method": "DELETE", "url": "https://api.example.com/v1/tasks/3f2504e0-4f89-11d3-9a0c-0305e82c3301", "headers": [] },
    "response": { "status": 204, "content": { "mimeType": "" } } }
] } }"#;

#[test]
fn har_capture() {
    let (i, converted) = import(HAR);
    assert_eq!(i.info.kind, "har");
    assert_eq!(i.info.title, "HAR capture of api.example.com");
    for secret in [
        "SECRET-QUERY",
        "SECRET-COOKIE",
        "SECRET-AUTH",
        "SECRET-PASSWORD",
        "ada",
    ] {
        assert!(!converted.contains(secret), "{secret} leaked");
    }
    let keys: Vec<&str> = i.operations.iter().map(|o| o.key.as_str()).collect();
    assert_eq!(
        keys.len(),
        3,
        "pages and scripts skipped, repeats merged: {keys:?}"
    );

    let schemes: Vec<&str> = i
        .info
        .security_schemes
        .iter()
        .map(|s| s.name.as_str())
        .collect();
    assert!(
        !schemes
            .iter()
            .any(|s| s.to_ascii_lowercase().contains("cookie")),
        "{schemes:?}"
    );
    let tasks = op(&i, "GET /v1/projects/{projects_id}/tasks");
    assert_eq!(http(tasks).base_url, "https://api.example.com");
    assert_eq!(param_names(tasks, ParamLocation::Query), ["page"]);
    assert!(
        http(tasks)
            .params
            .iter()
            .all(|p| p.schema.get("default").is_none()),
        "no captured values"
    );
    assert!(tasks.needs_auth);

    let login = op(&i, "POST /v1/login");
    assert_eq!(body_names(login), ["password", "user"]);
    assert!(op(&i, "DELETE /v1/tasks/{tasks_id}").supported);
}

#[test]
fn har_without_api_calls() {
    let doc = r#"{ "log": { "entries": [
      { "request": { "method": "GET", "url": "https://a.example.com/", "headers": [] },
        "response": { "status": 200, "content": { "mimeType": "text/html" } } } ] } }"#;
    let err = import_text(doc, &Options::default())
        .unwrap_err()
        .to_string();
    assert!(err.contains("No API calls"), "{err}");
}

const CURL: &str = r#"curl -X POST 'https://api.github.com/repos/rust-lang/rust/issues?per_page=5' \
  -H 'Authorization: Bearer ghp_SECRETTOKEN' \
  -H "X-GitHub-Api-Version: 2022-11-28" \
  -H 'Accept: application/vnd.github+json' \
  --data '{"title":"Found a bug","labels":["bug"],"draft":false}'

curl.exe "https://api.example.com/v2/search?q=rust&api_key=SECRETKEY" -u user:SECRETPASS

curl https://api.example.com/v2/upload -F file=@photo.png"#;

#[test]
fn curl_commands() {
    let (i, converted) = import(CURL);
    assert_eq!(i.info.kind, "curl");
    for secret in ["ghp_SECRETTOKEN", "SECRETKEY", "SECRETPASS", "Found a bug"] {
        assert!(!converted.contains(secret), "{secret} leaked");
    }
    assert_eq!(i.operations.len(), 3);

    let issue = op(&i, "POST /repos/rust-lang/rust/issues");
    let h = http(issue);
    assert_eq!(h.base_url, "https://api.github.com");
    assert_eq!(param_names(issue, ParamLocation::Query), ["per_page"]);
    assert_eq!(
        param_names(issue, ParamLocation::Header),
        ["X-GitHub-Api-Version"]
    );
    let version = h
        .params
        .iter()
        .find(|p| p.name == "X-GitHub-Api-Version")
        .unwrap();
    assert_eq!(version.schema["default"], "2022-11-28");
    assert_eq!(body_names(issue), ["draft", "labels", "title"]);
    assert!(issue.needs_auth);

    let search = op(&i, "GET /v2/search");
    assert_eq!(
        param_names(search, ParamLocation::Query),
        ["q"],
        "api_key became a scheme"
    );
    let schemes: Vec<&str> = i
        .info
        .security_schemes
        .iter()
        .map(|s| s.name.as_str())
        .collect();
    assert!(
        schemes.contains(&"query_api_key") && schemes.contains(&"basicAuth"),
        "{schemes:?}"
    );

    assert!(!op(&i, "POST /v2/upload").supported, "multipart");
}

#[test]
fn openapi_still_imports_as_before() {
    let doc = parse_document(include_str!("fixtures/petstore3.json")).unwrap();
    let (i, converted) = import_text(&doc.to_string(), &Options::default()).unwrap();
    assert!(converted.is_none());
    assert_eq!(i.info.kind, "openapi3");
    let err = import_text("just some words", &Options::default())
        .unwrap_err()
        .to_string();
    assert!(
        err.contains("Postman collection, a HAR file, or curl commands"),
        "{err}"
    );
}
