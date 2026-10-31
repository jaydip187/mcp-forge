use std::sync::Arc;

use forge_core::{GraphqlOperation, GraphqlVar};
use serde_json::json;
use wiremock::matchers::{body_json, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use super::*;
use crate::MapSecretResolver;

fn spec(endpoint: &str) -> GraphqlSpec {
    GraphqlSpec {
        endpoint: endpoint.into(),
        operation: GraphqlOperation::Query,
        document: "query User($login: String!, $first: Int) { user(login: $login) { name } }"
            .into(),
        operation_name: Some("User".into()),
        variables: vec![
            GraphqlVar {
                name: "login".into(),
                graphql_type: "String!".into(),
                description: String::new(),
                schema: None,
            },
            GraphqlVar {
                name: "first".into(),
                graphql_type: "Int".into(),
                description: String::new(),
                schema: None,
            },
        ],
        auth_ref: None,
        timeout_ms: 5_000,
    }
}

#[test]
fn reads_responses() {
    assert_eq!(
        read_response(r#"{"data":{"a":1}}"#),
        GraphqlResult::Data(r#"{"a":1}"#.into())
    );
    assert_eq!(
        read_response(r#"{"data":null,"errors":[{"message":"Not found","path":["user",0]}]}"#),
        GraphqlResult::Errors {
            messages: vec!["Not found (at user.0)".into()],
            data: None
        }
    );
    match read_response(r#"{"data":{"a":1},"errors":[{"message":"partial"}]}"#) {
        GraphqlResult::Errors { data: Some(d), .. } => assert_eq!(d, r#"{"a":1}"#),
        other => panic!("{other:?}"),
    }
    assert_eq!(
        read_response("<html>bad gateway</html>"),
        GraphqlResult::NotGraphql
    );
    assert_eq!(
        read_response(r#"{"message":"nope"}"#),
        GraphqlResult::NotGraphql
    );
}

#[tokio::test]
async fn posts_query_and_declared_variables_only() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/graphql"))
        .and(body_json(json!({
            "query": "query User($login: String!, $first: Int) { user(login: $login) { name } }",
            "operationName": "User",
            "variables": { "login": "octocat" },
        })))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({ "data": { "user": { "name": "The Octocat" } } })),
        )
        .expect(1)
        .mount(&server)
        .await;
    let exec = HttpExecutor::new(Arc::new(MapSecretResolver::default()));
    let s = spec(&format!("{}/api/graphql", server.uri()));
    let args = json!({ "login": "octocat", "first": null })
        .as_object()
        .unwrap()
        .clone();
    let out = exec.execute_graphql(&s, None, None, &args).await.unwrap();
    assert_eq!(out.status, 200);
    assert!(out.body.contains("The Octocat"));

    // Arguments are checked like any tool's.
    let bad = json!({ "first": 3, "nope": 1 })
        .as_object()
        .unwrap()
        .clone();
    assert!(matches!(
        exec.execute_graphql(&s, None, None, &bad).await,
        Err(ExecError::InvalidArgs(_))
    ));
}
