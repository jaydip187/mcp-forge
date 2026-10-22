use super::*;

#[test]
fn servers_from_host_and_base_path() {
    let d = |v: Value| server_url(&v);
    assert_eq!(
        d(json!({ "host": "api.x.io", "basePath": "/v2/", "schemes": ["http", "https"] }))
            .as_deref(),
        Some("https://api.x.io/v2")
    );
    assert_eq!(
        d(json!({ "host": "api.x.io", "schemes": ["http"] })).as_deref(),
        Some("http://api.x.io")
    );
    assert_eq!(d(json!({ "basePath": "/api" })).as_deref(), Some("/api"));
    assert_eq!(d(json!({})), None);
}

#[test]
fn converts_params_bodies_and_refs() {
    let doc = json!({
        "swagger": "2.0",
        "info": { "title": "T", "version": "1" },
        "host": "x.io",
        "consumes": ["application/json"],
        "definitions": { "A": { "type": "object", "properties": {
            "b": { "$ref": "#/definitions/B" },
            "n": { "type": "string", "x-nullable": true }
        }}, "B": { "type": "string" } },
        "parameters": { "Limit": { "name": "limit", "in": "query", "type": "integer", "maximum": 100 } },
        "securityDefinitions": { "basic": { "type": "basic" } },
        "paths": { "/a/{id}": {
            "parameters": [{ "name": "id", "in": "path", "type": "string" }],
            "post": {
                "operationId": "createA",
                "parameters": [
                    { "$ref": "#/parameters/Limit" },
                    { "name": "ids", "in": "query", "type": "array", "items": { "type": "integer" } },
                    { "name": "tags", "in": "query", "type": "array", "collectionFormat": "multi", "items": { "type": "string" } },
                    { "name": "body", "in": "body", "required": true, "schema": { "$ref": "#/definitions/A" } }
                ]
            },
            "put": {
                "consumes": ["multipart/form-data"],
                "parameters": [
                    { "name": "file", "in": "formData", "type": "file", "required": true },
                    { "name": "note", "in": "formData", "type": "string" }
                ]
            }
        }}
    });
    let v3 = to_openapi3(&doc).unwrap();
    assert_eq!(v3["servers"][0]["url"], "https://x.io");
    assert_eq!(
        v3["components"]["schemas"]["A"]["properties"]["b"]["$ref"],
        "#/components/schemas/B"
    );
    assert_eq!(
        v3["components"]["schemas"]["A"]["properties"]["n"]["nullable"],
        true
    );
    assert_eq!(
        v3["components"]["securitySchemes"]["basic"]["scheme"],
        "basic"
    );

    let post = &v3["paths"]["/a/{id}"]["post"];
    let params = post["parameters"].as_array().unwrap();
    assert_eq!(params[0]["name"], "id"); // path-level param inherited
    assert_eq!(params[0]["required"], true);
    assert_eq!(
        params[1]["schema"],
        json!({ "type": "integer", "maximum": 100 })
    ); // $ref'd global param
    assert_eq!(
        (params[2]["style"].clone(), params[2]["explode"].clone()),
        (json!("form"), json!(false))
    );
    assert_eq!(params[3]["explode"], true);
    assert_eq!(post["requestBody"]["required"], true);
    assert_eq!(
        post["requestBody"]["content"]["application/json"]["schema"]["$ref"],
        "#/components/schemas/A"
    );

    let put = &v3["paths"]["/a/{id}"]["put"];
    let form = &put["requestBody"]["content"]["multipart/form-data"]["schema"];
    assert_eq!(form["properties"]["file"]["format"], "binary");
    assert_eq!(form["required"], json!(["file"]));
}

#[test]
fn rejects_non_swagger() {
    assert!(to_openapi3(&json!({ "openapi": "3.0.0" })).is_err());
}
