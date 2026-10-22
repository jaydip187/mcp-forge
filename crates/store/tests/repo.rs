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
    let ToolKind::Http(h) = &t.kind else {
        panic!("expected an HTTP tool")
    };
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
        options: json!({ "base_url": "https://x.io" }),
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

fn spec_for_tests() -> forge_store::NewSpec {
    forge_store::NewSpec {
        kind: "openapi3".into(),
        title: "Pets".into(),
        version: "1".into(),
        source_url: None,
        filename: None,
        raw: "v1".into(),
        options: json!({}),
    }
}

#[tokio::test]
async fn edits_to_imported_tools_become_overrides() {
    let s = store().await;
    let p = s.create_project(&project("A")).await.unwrap();
    let (_, tools) = s
        .import_tools(p.id, &spec_for_tests(), &[(tool("t", None), "op".into())])
        .await
        .unwrap();
    let t = &tools[0];
    assert!(t.overrides.fields.is_empty());

    let mut edit = t.input();
    edit.description = "mine".into();
    edit.enabled = false;
    let u = s.update_tool(p.id, t.id, &edit).await.unwrap();
    let fields: Vec<_> = u.overrides.fields.iter().map(String::as_str).collect();
    assert_eq!(fields, ["description", "enabled"]);
    assert_eq!(s.get_tool(p.id, t.id).await.unwrap().overrides, u.overrides);

    // Saving again without changes doesn't add anything; manual tools never track.
    let again = s.update_tool(p.id, t.id, &edit).await.unwrap();
    assert_eq!(again.overrides, u.overrides);
    let m = s.create_tool(p.id, &tool("manual", None)).await.unwrap();
    let mut medit = m.input();
    medit.description = "x".into();
    assert!(s
        .update_tool(p.id, m.id, &medit)
        .await
        .unwrap()
        .overrides
        .fields
        .is_empty());

    let reset = s
        .reset_overrides(p.id, t.id, Some(&["enabled".to_owned()]))
        .await
        .unwrap();
    assert_eq!(
        reset.overrides.fields.iter().collect::<Vec<_>>(),
        ["description"]
    );
    assert_eq!(reset.description, "mine", "reset keeps current values");
    let all = s.reset_overrides(p.id, t.id, None).await.unwrap();
    assert!(all.overrides.fields.is_empty());
}

#[tokio::test]
async fn apply_reimport_updates_inserts_deletes_atomically() {
    let s = store().await;
    let p = s.create_project(&project("A")).await.unwrap();
    let (spec, tools) = s
        .import_tools(
            p.id,
            &spec_for_tests(),
            &[(tool("a", None), "a".into()), (tool("b", None), "b".into())],
        )
        .await
        .unwrap();
    let (a, b) = (&tools[0], &tools[1]);
    let mut edit = a.input();
    edit.description = "mine".into();
    s.update_tool(p.id, a.id, &edit).await.unwrap();

    let mut v2 = spec_for_tests();
    v2.raw = "v2".into();
    v2.version = "2".into();
    let mut new_a = a.input();
    new_a.description = "mine".into();
    let ToolKind::Http(h) = &mut new_a.kind else {
        panic!("expected an HTTP tool")
    };
    h.timeout_ms = 1234;
    let (rec, res) = s
        .apply_reimport(
            p.id,
            spec.id,
            &v2,
            &[(a.id, new_a, "a".into())],
            &[(tool("c", None), "c".into())],
            &[b.id],
        )
        .await
        .unwrap();
    assert_eq!(rec.version, "2");
    assert!(rec.updated_at.is_some());
    assert_eq!(s.get_spec_raw(p.id, spec.id).await.unwrap(), "v2");
    assert_eq!(res.deleted, [b.id]);
    assert_eq!(res.created[0].source.spec_id(), Some(spec.id));
    let a2 = s.get_tool(p.id, a.id).await.unwrap();
    let ToolKind::Http(h) = &a2.kind else {
        panic!("expected an HTTP tool")
    };
    assert_eq!(h.timeout_ms, 1234);
    assert!(
        a2.overrides.fields.contains("description"),
        "system updates keep overrides"
    );
    assert!(
        !a2.overrides.fields.contains("kind.timeout_ms"),
        "and don't add new ones"
    );

    // A failing step (name clash) rolls everything back.
    let mut clash = tool("c", None);
    clash.description = "dup".into();
    let r = s
        .apply_reimport(
            p.id,
            spec.id,
            &spec_for_tests(),
            &[],
            &[(clash, "x".into())],
            &[a.id],
        )
        .await;
    assert!(matches!(r, Err(StoreError::Conflict(_))), "{r:?}");
    assert!(
        s.get_tool(p.id, a.id).await.is_ok(),
        "delete was rolled back"
    );
    assert_eq!(s.get_spec_raw(p.id, spec.id).await.unwrap(), "v2");
    assert_eq!(s.tools_for_spec(p.id, spec.id).await.unwrap().len(), 2);
}

fn bt(input: ToolInput, auth: Option<&str>) -> forge_store::BundleTool {
    forge_store::BundleTool {
        input,
        auth: auth.map(str::to_owned),
        connection: None,
    }
}

#[tokio::test]
async fn import_bundle_policies() {
    use forge_store::ConflictPolicy;
    let s = store().await;
    let p = s.create_project(&project("A")).await.unwrap();
    let auths = vec![auth("token")];
    let tools = vec![
        bt(tool("a", None), Some("token")),
        bt(tool("b", None), None),
    ];

    let r = s
        .import_bundle(p.id, &auths, &[], &tools, ConflictPolicy::Fail)
        .await
        .unwrap();
    assert_eq!(r.auth_created, ["token"]);
    assert_eq!(r.tools_created, ["a", "b"]);
    let a = s.get_tool_by_name(p.id, "a").await.unwrap();
    let ToolKind::Http(h) = &a.kind else {
        panic!("expected an HTTP tool")
    };
    assert_eq!(
        h.auth_ref,
        Some(s.list_auth_configs(p.id).await.unwrap()[0].id)
    );
    assert_eq!(a.source, Source::Manual);

    // Same bundle again: identical auth is kept, tools conflict.
    let r = s
        .import_bundle(p.id, &auths, &[], &tools, ConflictPolicy::Fail)
        .await;
    assert!(matches!(r, Err(StoreError::Conflict(_))), "{r:?}");
    let r = s
        .import_bundle(p.id, &auths, &[], &tools, ConflictPolicy::Skip)
        .await
        .unwrap();
    assert_eq!((r.auth_kept.len(), r.tools_skipped.len()), (1, 2));

    let mut changed = tool("a", None);
    changed.description = "replaced".into();
    let r = s
        .import_bundle(
            p.id,
            &[],
            &[],
            &[bt(changed, Some("token")), bt(tool("c", None), None)],
            ConflictPolicy::Replace,
        )
        .await
        .unwrap();
    assert_eq!(
        (r.tools_updated.clone(), r.tools_created.clone()),
        (vec!["a".to_owned()], vec!["c".to_owned()])
    );
    assert_eq!(
        s.get_tool_by_name(p.id, "a").await.unwrap().description,
        "replaced"
    );

    // Unknown auth name → validation error, nothing written.
    let r = s
        .import_bundle(
            p.id,
            &[],
            &[],
            &[bt(tool("d", None), Some("nope"))],
            ConflictPolicy::Fail,
        )
        .await;
    assert!(matches!(r, Err(StoreError::Validation(e)) if e.has("tools[0].auth")));
    assert!(s.get_tool_by_name(p.id, "d").await.is_err());

    // A conflict partway through rolls back earlier inserts.
    let mut other = auth("other");
    other.secret_ref = forge_core::SecretRef::parse("env:OTHER").unwrap();
    let r = s
        .import_bundle(
            p.id,
            &[other],
            &[],
            &[bt(tool("e", None), None), bt(tool("a", None), None)],
            ConflictPolicy::Fail,
        )
        .await;
    assert!(matches!(r, Err(StoreError::Conflict(_))));
    assert!(s.get_tool_by_name(p.id, "e").await.is_err());
    assert_eq!(s.list_auth_configs(p.id).await.unwrap().len(), 1);
}

fn conn(name: &str, driver: &str) -> forge_core::DbConnectionInput {
    serde_json::from_value(json!({ "name": name, "driver": driver, "dsn_ref": "env:DB_URL" }))
        .unwrap()
}

fn sql_tool(name: &str, connection_id: Uuid) -> ToolInput {
    serde_json::from_value(json!({
        "name": name,
        "kind": { "type": "sql", "connection_id": connection_id,
                  "mode": { "type": "query", "sql": "SELECT 1" } }
    }))
    .unwrap()
}

#[tokio::test]
async fn connection_crud_and_reference_rules() {
    let s = store().await;
    let p = s.create_project(&project("A")).await.unwrap();
    let other = s.create_project(&project("B")).await.unwrap();
    let c = s
        .create_connection(p.id, &conn("shop", "postgres"))
        .await
        .unwrap();
    assert_eq!(
        s.list_connections(p.id).await.unwrap(),
        std::slice::from_ref(&c)
    );
    assert_eq!(s.get_connection(p.id, c.id).await.unwrap(), c);
    assert_conflict(s.create_connection(p.id, &conn("shop", "mysql")).await);
    assert!(matches!(
        s.get_connection(other.id, c.id).await,
        Err(StoreError::NotFound(_))
    ));

    // Masks and limits round-trip through the settings column.
    let mut input = conn("shop", "postgres");
    input.row_limit = 5;
    input.column_masks = vec![forge_core::ColumnMask::parse("users.email").unwrap()];
    let c2 = s.update_connection(p.id, c.id, &input).await.unwrap();
    assert_eq!(s.get_connection(p.id, c.id).await.unwrap(), c2);
    assert_eq!(c2.row_limit, 5);

    // Tools must use a connection from their own project.
    let foreign = s
        .create_connection(other.id, &conn("x", "sqlite"))
        .await
        .unwrap();
    let r = s.create_tool(p.id, &sql_tool("q", foreign.id)).await;
    assert!(matches!(r, Err(StoreError::Validation(e)) if e.has("kind.connection_id")));
    let t = s.create_tool(p.id, &sql_tool("q", c.id)).await.unwrap();
    assert_eq!(t.kind.connection_ref(), Some(c.id));

    // In use: no delete, no driver change; other edits are fine.
    assert_conflict(s.delete_connection(p.id, c.id).await);
    assert_conflict(
        s.update_connection(p.id, c.id, &conn("shop", "mysql"))
            .await,
    );
    s.delete_tool(p.id, t.id).await.unwrap();
    s.update_connection(p.id, c.id, &conn("shop", "mysql"))
        .await
        .unwrap();
    s.delete_connection(p.id, c.id).await.unwrap();
    assert!(s.list_connections(p.id).await.unwrap().is_empty());

    // Project delete cascades through SQL tools and connections.
    let c = s
        .create_connection(p.id, &conn("shop", "sqlite"))
        .await
        .unwrap();
    s.create_tool(p.id, &sql_tool("q", c.id)).await.unwrap();
    s.delete_project(p.id).await.unwrap();
}

#[tokio::test]
async fn bundle_imports_connections_and_sql_tools() {
    use forge_store::{BundleTool, ConflictPolicy};
    let s = store().await;
    let p = s.create_project(&project("A")).await.unwrap();
    let tools = vec![BundleTool {
        input: sql_tool("q", Uuid::nil()),
        auth: None,
        connection: Some("shop".into()),
    }];
    let r = s
        .import_bundle(
            p.id,
            &[],
            &[conn("shop", "sqlite")],
            &tools,
            ConflictPolicy::Fail,
        )
        .await
        .unwrap();
    assert_eq!(r.connections_created, ["shop"]);
    let c = &s.list_connections(p.id).await.unwrap()[0];
    let t = s.get_tool_by_name(p.id, "q").await.unwrap();
    assert_eq!(t.kind.connection_ref(), Some(c.id));

    // Identical connection is kept; a SQL tool without a connection name fails.
    let r = s
        .import_bundle(
            p.id,
            &[],
            &[conn("shop", "sqlite")],
            &[],
            ConflictPolicy::Fail,
        )
        .await
        .unwrap();
    assert_eq!(r.connections_kept, ["shop"]);
    let r = s
        .import_bundle(
            p.id,
            &[],
            &[],
            &[BundleTool {
                input: sql_tool("q2", Uuid::nil()),
                auth: None,
                connection: None,
            }],
            ConflictPolicy::Fail,
        )
        .await;
    assert!(matches!(r, Err(StoreError::Validation(e)) if e.has("tools[0].connection")));
}

#[tokio::test]
async fn audit_log_append_and_page() {
    use forge_core::{AuditEntry, AuditOutcome};
    let s = store().await;
    let p = s.create_project(&project("A")).await.unwrap();
    let base = chrono::Utc::now();
    for i in 0..3 {
        s.append_audit(&AuditEntry {
            id: Uuid::new_v4(),
            project_id: p.id,
            ts: base + chrono::Duration::seconds(i),
            tool_id: None,
            tool_name: format!("t{i}"),
            connection_id: None,
            connection_name: "shop".into(),
            origin: "mcp".into(),
            statement_class: "write".into(),
            sql: "UPDATE t SET a = ? WHERE id = ?".into(),
            params: vec![json!(i), json!("x")],
            rows_affected: Some(i as u64),
            outcome: if i == 2 {
                AuditOutcome::RolledBack
            } else {
                AuditOutcome::Committed
            },
            error: None,
        })
        .await
        .unwrap();
    }
    let all = s.list_audit(p.id, 100, None).await.unwrap();
    let names: Vec<_> = all.iter().map(|e| e.tool_name.as_str()).collect();
    assert_eq!(names, ["t2", "t1", "t0"], "newest first");
    assert_eq!(all[0].outcome, AuditOutcome::RolledBack);
    assert_eq!(all[0].params, [json!(2), json!("x")]);
    let page = s.list_audit(p.id, 1, Some(all[0].ts)).await.unwrap();
    assert_eq!(page[0].tool_name, "t1");
    s.delete_project(p.id).await.unwrap();
}
