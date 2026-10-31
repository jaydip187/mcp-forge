use super::*;

fn spec(doc: &str, vars: Vec<GraphqlVar>) -> GraphqlSpec {
    GraphqlSpec {
        endpoint: "https://api.example.com/graphql".into(),
        operation: GraphqlOperation::Query,
        document: doc.into(),
        operation_name: None,
        variables: vars,
        auth_ref: None,
        timeout_ms: 30_000,
    }
}

fn var(name: &str, t: &str) -> GraphqlVar {
    GraphqlVar {
        name: name.into(),
        graphql_type: t.into(),
        description: String::new(),
        schema: None,
    }
}

#[test]
fn schemas_from_types() {
    assert_eq!(schema_for_type("Int!"), json!({ "type": "integer" }));
    assert_eq!(
        schema_for_type("[ID!]"),
        json!({ "type": "array", "items": { "type": "string" } })
    );
    assert_eq!(schema_for_type("DateTime"), json!({}));
    assert!(var("id", "ID!").required());
    assert!(!var("first", "Int").required());
}

#[test]
fn input_from_variables() {
    let mut first = var("first", "Int");
    first.description = "How many".into();
    let s = spec(
        "query Q($id: ID!, $first: Int) { user(id: $id) { repos(first: $first) { name } } }",
        vec![var("id", "ID!"), first],
    );
    let d = s.derive_input();
    assert_eq!(d.schema["required"], json!(["id"]));
    assert_eq!(
        d.schema["properties"]["first"],
        json!({ "type": "integer", "description": "How many" })
    );
    assert_eq!(d.bindings.len(), 2);
    let mut e = ValidationErrors::default();
    s.validate_into(&mut e);
    assert!(e.is_empty(), "{e:?}");
}

#[test]
fn validation() {
    let mut s = spec(
        "mutation M { x }",
        vec![var("gone", "ID!"), var("bad name", "ID"), var("t", "[ID")],
    );
    s.endpoint = "https://u:p@api.example.com/graphql?x=1".into();
    s.timeout_ms = 0;
    let mut e = ValidationErrors::default();
    s.validate_into(&mut e);
    for f in [
        "kind.endpoint",
        "kind.document",
        "kind.timeout_ms",
        "kind.variables[0].name",
        "kind.variables[1].name",
        "kind.variables[2].graphql_type",
    ] {
        assert!(e.has(f), "{f}: {e:?}");
    }
    // Declared but never used in the body.
    let unused = spec(
        "query Q($id: ID!, $idx: Int) { user(id: $idx) { name } }",
        vec![var("id", "ID!"), var("idx", "Int")],
    );
    let mut e = ValidationErrors::default();
    unused.validate_into(&mut e);
    assert!(e.has("kind.variables[0].name"), "{e:?}");
    assert!(!e.has("kind.variables[1].name"), "{e:?}");

    let shorthand = spec("{ viewer { login } }", vec![]);
    let mut e = ValidationErrors::default();
    shorthand.validate_into(&mut e);
    assert!(e.is_empty(), "`{{ … }}` is a query");
}
