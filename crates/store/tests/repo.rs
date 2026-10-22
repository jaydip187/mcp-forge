use forge_core::{AuthConfigInput, ProjectInput, Source, ToolInput, ToolKind};
use forge_store::{Store, StoreError};
use serde_json::{json, Value};
use uuid::Uuid;

async fn store() -> Store {
    Store::open_in_memory().await.unwrap()
}

fn project(name: &str) -> ProjectInput {
    ProjectInput {
        name: name.into(),
        slug: None,
    }
}

fn tool(name: &str, auth_ref: Option<Uuid>) -> ToolInput {
    serde_json::from_value(json!({
        "name": name,
        "description": "Fetch a user",
        "kind": {
            "type": "http",
            "method": "GET",
            "base_url": "https://api.example.com",
            "path_template": "/users/{id}",
            "params": [{ "name": "id", "location": "path", "required": true }],
            "auth_ref": auth_ref,
        }
    }))
    .unwrap()
}

fn auth(name: &str) -> AuthConfigInput {
    serde_json::from_value(json!({
        "name": name, "kind": { "type": "bearer" }, "secret_ref": "env:TOKEN"
    }))
    .unwrap()
}

fn assert_conflict<T: std::fmt::Debug>(r: Result<T, StoreError>) {
    assert!(matches!(r, Err(StoreError::Conflict(_))), "{r:?}");
}

#[tokio::test]
async fn project_crud() {
    let s = store().await;
    let p = s.create_project(&project("Demo API")).await.unwrap();
    assert_eq!(p.slug, "demo-api");
    assert_eq!(s.get_project(p.id).await.unwrap(), p);
    assert_eq!(s.get_project_by_slug("demo-api").await.unwrap(), p);
    assert_eq!(s.list_projects().await.unwrap(), vec![p.clone()]);

    // Rename without slug keeps the slug.
    let renamed = s.update_project(p.id, &project("Renamed")).await.unwrap();
    assert_eq!(
        (renamed.name.as_str(), renamed.slug.as_str()),
        ("Renamed", "demo-api")
    );
    assert_eq!(renamed.created_at, p.created_at);

    s.delete_project(p.id).await.unwrap();
    assert!(matches!(
        s.get_project(p.id).await,
        Err(StoreError::NotFound("project"))
    ));
    assert!(matches!(
        s.delete_project(p.id).await,
        Err(StoreError::NotFound(_))
    ));
}

#[tokio::test]
async fn project_slug_unique() {
    let s = store().await;
    s.create_project(&project("Demo")).await.unwrap();
    assert_conflict(s.create_project(&project("demo")).await);
    let other = s.create_project(&project("Other")).await.unwrap();
    let clash = ProjectInput {
        name: "Other".into(),
        slug: Some("demo".into()),
    };
    assert_conflict(s.update_project(other.id, &clash).await);
}

#[tokio::test]
async fn project_validation_error() {
    let s = store().await;
    let r = s.create_project(&project("")).await;
    assert!(matches!(r, Err(StoreError::Validation(e)) if e.has("name")));
}

#[tokio::test]
async fn tool_crud_round_trip() {
    let s = store().await;
    let p = s.create_project(&project("Demo")).await.unwrap();
    let t = s.create_tool(p.id, &tool("get_user", None)).await.unwrap();
    assert_eq!(t.source, Source::Manual);
    assert!(t.enabled);
    assert_eq!(s.get_tool(p.id, t.id).await.unwrap(), t);
    assert_eq!(s.list_tools(p.id).await.unwrap(), vec![t.clone()]);

    let mut edit = tool("get_user_v2", None);
    edit.enabled = false;
    let u = s.update_tool(p.id, t.id, &edit).await.unwrap();
    assert_eq!(u.name, "get_user_v2");
    assert!(!u.enabled);
    assert_eq!(u.created_at, t.created_at);
    assert_eq!(s.get_tool(p.id, t.id).await.unwrap(), u);

    s.delete_tool(p.id, t.id).await.unwrap();
    assert!(matches!(
        s.get_tool(p.id, t.id).await,
        Err(StoreError::NotFound("tool"))
    ));
}

#[tokio::test]
async fn tool_names_unique_per_project_only() {
    let s = store().await;
    let a = s.create_project(&project("A")).await.unwrap();
    let b = s.create_project(&project("B")).await.unwrap();
    s.create_tool(a.id, &tool("get_user", None)).await.unwrap();
    assert_conflict(s.create_tool(a.id, &tool("get_user", None)).await);
    s.create_tool(b.id, &tool("get_user", None)).await.unwrap();

    let other = s.create_tool(a.id, &tool("other", None)).await.unwrap();
    assert_conflict(s.update_tool(a.id, other.id, &tool("get_user", None)).await);
}

#[tokio::test]
async fn tool_scoped_to_project() {
    let s = store().await;
    let a = s.create_project(&project("A")).await.unwrap();
    let b = s.create_project(&project("B")).await.unwrap();
    let t = s.create_tool(a.id, &tool("get_user", None)).await.unwrap();
    assert!(matches!(
        s.get_tool(b.id, t.id).await,
        Err(StoreError::NotFound(_))
    ));
    assert!(matches!(
        s.delete_tool(b.id, t.id).await,
        Err(StoreError::NotFound(_))
    ));
    let missing = s.create_tool(Uuid::new_v4(), &tool("x", None)).await;
    assert!(matches!(missing, Err(StoreError::NotFound("project"))));
}

#[tokio::test]
async fn auth_crud_and_reference_rules() {
    let s = store().await;
    let a = s.create_project(&project("A")).await.unwrap();
    let b = s.create_project(&project("B")).await.unwrap();
    let cfg = s.create_auth_config(a.id, &auth("token")).await.unwrap();
    assert_eq!(s.get_auth_config(a.id, cfg.id).await.unwrap(), cfg);
    assert_eq!(s.list_auth_configs(a.id).await.unwrap(), vec![cfg.clone()]);
    assert_conflict(s.create_auth_config(a.id, &auth("token")).await);

    // Tools may only reference auth configs in their own project.
    let cross = s.create_tool(b.id, &tool("t", Some(cfg.id))).await;
    assert!(matches!(cross, Err(StoreError::Validation(e)) if e.has("kind.auth_ref")));

    let t = s.create_tool(a.id, &tool("t", Some(cfg.id))).await.unwrap();
    let ToolKind::Http(h) = &t.kind;
    assert_eq!(h.auth_ref, Some(cfg.id));

    // In use → cannot delete; after unlinking → can.
    let r = s.delete_auth_config(a.id, cfg.id).await;
    assert!(
        matches!(&r, Err(StoreError::Conflict(m)) if m.contains("t")),
        "{r:?}"
    );
    s.update_tool(a.id, t.id, &tool("t", None)).await.unwrap();
    s.delete_auth_config(a.id, cfg.id).await.unwrap();
    assert!(s.list_auth_configs(a.id).await.unwrap().is_empty());
}

#[tokio::test]
async fn auth_update() {
    let s = store().await;
    let p = s.create_project(&project("A")).await.unwrap();
    let cfg = s.create_auth_config(p.id, &auth("token")).await.unwrap();
    let mut input = auth("renamed");
    input.secret_ref = forge_core::SecretRef::parse("keyring:gh").unwrap();
    let u = s.update_auth_config(p.id, cfg.id, &input).await.unwrap();
    assert_eq!(u.name, "renamed");
    assert_eq!(u.secret_ref.as_str(), "keyring:gh");
    assert_eq!(s.get_auth_config(p.id, cfg.id).await.unwrap(), u);
}

#[tokio::test]
async fn project_delete_cascades() {
    let s = store().await;
    let p = s.create_project(&project("A")).await.unwrap();
    let cfg = s.create_auth_config(p.id, &auth("token")).await.unwrap();
    s.create_tool(p.id, &tool("t", Some(cfg.id))).await.unwrap();
    s.delete_project(p.id).await.unwrap();

    let counts: (i64, i64) =
        sqlx::query_as("SELECT (SELECT COUNT(*) FROM tools), (SELECT COUNT(*) FROM auth_configs)")
            .fetch_one(s.pool())
            .await
            .unwrap();
    assert_eq!(counts, (0, 0));
}

#[tokio::test]
async fn stored_json_is_ir_shaped() {
    let s = store().await;
    let p = s.create_project(&project("A")).await.unwrap();
    s.create_tool(p.id, &tool("t", None)).await.unwrap();
    let (kind, source): (String, String) = sqlx::query_as("SELECT kind, source FROM tools")
        .fetch_one(s.pool())
        .await
        .unwrap();
    let kind: Value = serde_json::from_str(&kind).unwrap();
    assert_eq!(kind["type"], "http");
    assert_eq!(kind["path_template"], "/users/{id}");
    assert_eq!(source, r#"{"type":"manual"}"#);
}

#[tokio::test]
async fn lookup_by_name_and_fingerprint() {
    let s = store().await;
    let p = s.create_project(&project("A")).await.unwrap();
    let empty = s.tools_fingerprint(p.id).await.unwrap();
    let t = s.create_tool(p.id, &tool("t", None)).await.unwrap();
    assert_eq!(s.get_tool_by_name(p.id, "t").await.unwrap(), t);
    assert!(matches!(
        s.get_tool_by_name(p.id, "nope").await,
        Err(StoreError::NotFound(_))
    ));

    let one = s.tools_fingerprint(p.id).await.unwrap();
    assert_ne!(one, empty);
    tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    s.update_tool(p.id, t.id, &tool("t", None)).await.unwrap();
    let edited = s.tools_fingerprint(p.id).await.unwrap();
    assert_ne!(edited, one);
    s.delete_tool(p.id, t.id).await.unwrap();
    assert_eq!(s.tools_fingerprint(p.id).await.unwrap(), empty);
}

#[tokio::test]
async fn import_tools_is_atomic_and_tags_source() {
    let s = store().await;
    let p = s.create_project(&project("A")).await.unwrap();
    let spec = forge_store::NewSpec {
        kind: "openapi3".into(),
        title: "Pets".into(),
        version: "1.0".into(),
        source_url: None,
        filename: Some("pets.json".into()),
        raw: "{\"openapi\":\"3.0.0\"}".into(),
    };
    let (rec, tools) = s
        .import_tools(
            p.id,
            &spec,
            &[
                (tool("a", None), "getA".into()),
                (tool("b", None), "GET /b".into()),
            ],
        )
        .await
        .unwrap();
    assert_eq!(tools.len(), 2);
    assert_eq!(
        tools[0].source,
        Source::OpenApi {
            spec_id: rec.id,
            op_id: "getA".into()
        }
    );
    assert_eq!(s.list_specs(p.id).await.unwrap(), vec![rec.clone()]);
    assert_eq!(s.get_spec_raw(p.id, rec.id).await.unwrap(), spec.raw);
    let mut names = s.tool_names(p.id).await.unwrap();
    names.sort();
    assert_eq!(names, ["a", "b"]);

    // A clash in the batch rolls back everything, including the spec row.
    let r = s
        .import_tools(
            p.id,
            &spec,
            &[
                (tool("c", None), "c".into()),
                (tool("a", None), "dup".into()),
            ],
        )
        .await;
    assert!(matches!(r, Err(StoreError::Conflict(_))), "{r:?}");
    assert_eq!(s.list_specs(p.id).await.unwrap().len(), 1);
    assert_eq!(s.tool_names(p.id).await.unwrap().len(), 2);

    // Invalid tool → validation error naming its index.
    let mut bad = tool("ok", None);
    bad.name = "bad name".into();
    let r = s
        .import_tools(
            p.id,
            &spec,
            &[(tool("ok2", None), "x".into()), (bad, "y".into())],
        )
        .await;
    assert!(matches!(r, Err(StoreError::Validation(e)) if e.has("tools[1].name")));
}
