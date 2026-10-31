//! Stage 24: GraphQL schemas (SDL and introspection) become one tool per
//! query and mutation field.

use forge_core::{GraphqlOperation, ToolKind};
use forge_importers::import_text;
use forge_importers::openapi3::{BuiltOperation, Imported, Options};
use serde_json::json;

const SDL: &str = r#"
"""A person."""
type User {
  id: ID!
  login: String!
  name: String
  role: Role
  repos(first: Int = 10, after: String): RepoConnection!
  secret(token: String!): String
}
enum Role { ADMIN MEMBER }
type RepoConnection { edges: [RepoEdge!]! }
type RepoEdge { node: Repo! }
type Repo { name: String! stars: Int owner: User! }
union SearchResult = User | Repo
input NewRepo { name: String! private: Boolean = false topics: [String!] }
type Query {
  "Look up a user by login."
  user(login: String!): User
  search(q: String!, limit: Int): [SearchResult!]!
  viewerCount: Int!
  old: String @deprecated(reason: "gone")
}
type Mutation {
  createRepo(input: NewRepo!): Repo!
}
"#;

fn op<'a>(i: &'a Imported, key: &str) -> &'a BuiltOperation {
    i.operations
        .iter()
        .find(|o| o.key == key)
        .unwrap_or_else(|| panic!("no {key}"))
}

fn gql(o: &BuiltOperation) -> &forge_core::GraphqlSpec {
    match &o.tool.kind {
        ToolKind::Graphql(g) => g,
        _ => panic!("graphql"),
    }
}

#[test]
fn sdl_schema() {
    let opts = Options {
        base_url: Some("https://api.example.com/graphql".into()),
        source_url: None,
    };
    let (i, converted) = import_text(SDL, &opts).unwrap();
    assert!(converted.is_none(), "stored as written");
    assert_eq!(i.info.kind, "graphql");
    assert_eq!(i.info.title, "GraphQL API at api.example.com");
    let keys: Vec<&str> = i.operations.iter().map(|o| o.key.as_str()).collect();
    assert_eq!(
        keys,
        [
            "query user",
            "query search",
            "query viewerCount",
            "mutation createRepo"
        ],
        "deprecated skipped"
    );

    let user = op(&i, "query user");
    let g = gql(user);
    assert_eq!(g.endpoint, "https://api.example.com/graphql");
    assert_eq!(g.operation, GraphqlOperation::Query);
    assert_eq!(g.operation_name.as_deref(), Some("User"));
    assert!(
        g.document
            .starts_with("query User($login: String!) {\n  user(login: $login) {"),
        "{}",
        g.document
    );
    for field in ["id", "login", "name", "role", "repos {", "edges {"] {
        assert!(g.document.contains(field), "{field} in\n{}", g.document);
    }
    assert!(
        !g.document.contains("node {"),
        "nesting stops at three levels"
    );
    assert!(
        !g.document.contains("secret"),
        "fields with required args are skipped"
    );
    assert_eq!(g.variables[0].graphql_type, "String!");
    assert_eq!(
        user.tool.description,
        "Look up a user by login.\n\nReturns User."
    );
    assert_eq!(user.tool.tags, ["Query"]);
    assert!(user.supported);

    let search = op(&i, "query search");
    let doc = &gql(search).document;
    assert!(
        doc.contains("__typename")
            && doc.contains("... on User {")
            && doc.contains("... on Repo {"),
        "{doc}"
    );

    let count = op(&i, "query viewerCount");
    assert_eq!(gql(count).document, "query ViewerCount {\n  viewerCount\n}");

    let create = op(&i, "mutation createRepo");
    let g = gql(create);
    assert_eq!(g.operation, GraphqlOperation::Mutation);
    assert_eq!(create.tool.tags, ["Mutation"]);
    let input = g.variables[0].schema.clone().unwrap();
    assert_eq!(
        input["required"],
        json!(["name"]),
        "defaults aren't required"
    );
    assert_eq!(input["properties"]["topics"]["type"], "array");
    assert!(
        create.tool.validate().is_ok(),
        "{:?}",
        create.tool.validate()
    );
}

#[test]
fn no_endpoint_until_given() {
    let (i, _) = import_text(SDL, &Options::default()).unwrap();
    assert!(i.operations.iter().all(|o| !o.supported));
    assert!(i.operations[0].warnings[0].contains("endpoint"));
}

#[test]
fn introspection_result() {
    let t = |kind: &str, name: &str| json!({ "kind": kind, "name": name, "ofType": null });
    let nn =
        |inner: serde_json::Value| json!({ "kind": "NON_NULL", "name": null, "ofType": inner });
    let doc = json!({ "data": { "__schema": {
        "queryType": { "name": "Query" },
        "mutationType": null,
        "types": [
            { "kind": "OBJECT", "name": "Query", "fields": [
                { "name": "issue", "description": "One issue.", "args": [
                    { "name": "number", "description": "Issue number", "type": nn(t("SCALAR", "Int")), "defaultValue": null }
                ], "type": t("OBJECT", "Issue") }
            ] },
            { "kind": "OBJECT", "name": "Issue", "fields": [
                { "name": "title", "args": [], "type": nn(t("SCALAR", "String")) },
                { "name": "state", "args": [], "type": t("ENUM", "IssueState") }
            ] },
            { "kind": "ENUM", "name": "IssueState", "enumValues": [{ "name": "OPEN" }, { "name": "CLOSED" }] },
            { "kind": "SCALAR", "name": "Int" },
            { "kind": "SCALAR", "name": "String" }
        ]
    } } });
    let opts = Options {
        base_url: None,
        source_url: Some("https://gh.example.com/graphql".into()),
    };
    let (i, _) = import_text(&doc.to_string(), &opts).unwrap();
    let issue = op(&i, "query issue");
    let g = gql(issue);
    assert_eq!(
        g.endpoint, "https://gh.example.com/graphql",
        "endpoint from the introspected URL"
    );
    assert_eq!(
        g.document,
        "query Issue($number: Int!) {\n  issue(number: $number) {\n    title\n    state\n  }\n}"
    );
    assert_eq!(g.variables[0].description, "Issue number");
    assert_eq!(
        g.variables[0].schema.clone().unwrap(),
        json!({ "type": "integer" })
    );
}
