use super::*;

fn tool(name: &str, desc: &str, path: &str, tags: &[&str]) -> ToolDef {
    serde_json::from_value(json!({
            "id": uuid::Uuid::new_v4(), "project_id": uuid::Uuid::new_v4(),
            "name": name, "description": desc, "enabled": true, "tags": tags,
            "kind": { "type": "http", "method": "GET", "base_url": "https://x.io", "path_template": path },
            "source": { "type": "manual" }, "overrides": {}, "response_shape": null,
            "created_at": "2026-01-01T00:00:00Z", "updated_at": "2026-01-01T00:00:00Z"
        }))
        .unwrap()
}

#[test]
fn search_ranks_name_over_tags_over_description() {
    let tools = [
        tool(
            "list_orders",
            "Lists orders for a user",
            "/orders",
            &["orders"],
        ),
        tool("get_user", "Fetch one user", "/users/{id}", &["users"]),
        tool("ping", "Health check", "/ping", &[]),
    ];
    let names = |q: &str, n| {
        search_tools(&tools, q, n)
            .iter()
            .map(|t| t.name.clone())
            .collect::<Vec<_>>()
    };
    assert_eq!(names("user", 10), ["get_user", "list_orders"]);
    assert_eq!(names("orders", 10), ["list_orders"]);
    assert_eq!(names("health", 10), ["ping"]);
    assert!(names("nothing-matches", 10).is_empty());
    assert_eq!(names("", 2).len(), 2, "empty query lists tools");
    assert_eq!(names("user", 1), ["get_user"]);
}
