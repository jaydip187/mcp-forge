//! Runs GraphQL tools by POSTing `{query, operationName, variables}` through
//! the HTTP executor, so auth, redirects, redaction and size limits are the
//! same as for HTTP tools.

use forge_core::{check_args, AuthConfig, GraphqlSpec, HttpMethod, HttpSpec, ParamDef};
use reqwest::Url;
use serde_json::{json, Map, Value};

use crate::{ExecError, HttpExecutor, HttpOutcome};

/// A GraphQL response, split for the agent.
#[derive(Debug, Clone, PartialEq)]
pub enum GraphqlResult {
    /// `data` (no `errors`), as JSON text.
    Data(String),
    /// `errors` were returned; partial `data` kept when present.
    Errors {
        messages: Vec<String>,
        data: Option<String>,
    },
    /// Not a GraphQL JSON response (proxy error page, 5xx, …).
    NotGraphql,
}

/// Reads a GraphQL response body.
pub fn read_response(body: &str) -> GraphqlResult {
    let Ok(v) = serde_json::from_str::<Value>(body) else {
        return GraphqlResult::NotGraphql;
    };
    let Some(obj) = v.as_object() else {
        return GraphqlResult::NotGraphql;
    };
    let data = obj
        .get("data")
        .filter(|d| !d.is_null())
        .map(Value::to_string);
    match obj
        .get("errors")
        .and_then(Value::as_array)
        .filter(|e| !e.is_empty())
    {
        Some(errors) => {
            let messages = errors
                .iter()
                .map(|e| {
                    let msg = e.get("message").and_then(Value::as_str).unwrap_or("error");
                    match e.get("path").and_then(Value::as_array) {
                        Some(p) if !p.is_empty() => {
                            let path: Vec<String> = p
                                .iter()
                                .map(|s| s.as_str().map_or_else(|| s.to_string(), str::to_owned))
                                .collect();
                            format!("{msg} (at {})", path.join("."))
                        }
                        _ => msg.to_owned(),
                    }
                })
                .collect();
            GraphqlResult::Errors { messages, data }
        }
        None if obj.contains_key("data") => {
            GraphqlResult::Data(data.unwrap_or_else(|| "null".into()))
        }
        None => GraphqlResult::NotGraphql,
    }
}

/// The request as a plain HTTP spec: POST to the endpoint with a whole JSON body.
fn as_http(spec: &GraphqlSpec) -> Result<HttpSpec, ExecError> {
    let url = Url::parse(&spec.endpoint)
        .map_err(|e| ExecError::Config(format!("GraphQL endpoint: {e}")))?;
    let origin = url.origin().ascii_serialization();
    Ok(HttpSpec {
        method: HttpMethod::Post,
        base_url: origin,
        path_template: url.path().to_owned(),
        params: Vec::<ParamDef>::new(),
        body_schema: Some(json!({ "type": "object" })),
        auth_ref: spec.auth_ref,
        timeout_ms: spec.timeout_ms,
    })
}

/// `{query, operationName?, variables}` from the agent's arguments.
fn request_body(spec: &GraphqlSpec, args: &Map<String, Value>) -> Map<String, Value> {
    let variables: Map<String, Value> = spec
        .variables
        .iter()
        .filter_map(|v| {
            args.get(&v.name)
                .filter(|a| !a.is_null())
                .map(|a| (v.name.clone(), a.clone()))
        })
        .collect();
    let mut body = Map::new();
    body.insert("query".into(), json!(spec.document));
    if let Some(n) = spec.operation_name.as_deref().filter(|n| !n.is_empty()) {
        body.insert("operationName".into(), json!(n));
    }
    body.insert("variables".into(), Value::Object(variables));
    body
}

impl HttpExecutor {
    /// Checks the arguments, then sends the operation. `token` is the OAuth
    /// access token for OAuth auth configs.
    pub async fn execute_graphql(
        &self,
        spec: &GraphqlSpec,
        auth: Option<&AuthConfig>,
        token: Option<&str>,
        args: &Map<String, Value>,
    ) -> Result<HttpOutcome, ExecError> {
        let derived = spec.derive_input();
        check_args(&derived.bindings, &derived.schema, args).map_err(ExecError::InvalidArgs)?;
        let http = as_http(spec)?;
        let mut wrapped = Map::new();
        wrapped.insert(
            forge_core::BODY_ARG.into(),
            Value::Object(request_body(spec, args)),
        );
        match (auth, token) {
            (Some(a), Some(t)) => self.execute_with_token(&http, a, t, &wrapped).await,
            _ => self.execute(&http, auth, &wrapped).await,
        }
    }
}

/// The standard introspection query (types, fields, args, enums, inputs).
pub const INTROSPECTION_QUERY: &str = r#"query IntrospectionQuery {
  __schema {
    queryType { name }
    mutationType { name }
    types {
      kind name description
      fields(includeDeprecated: false) {
        name description
        args { name description type { ...TypeRef } defaultValue }
        type { ...TypeRef }
      }
      inputFields { name description type { ...TypeRef } defaultValue }
      enumValues(includeDeprecated: false) { name }
      possibleTypes { name }
    }
  }
}
fragment TypeRef on __Type {
  kind name
  ofType { kind name ofType { kind name ofType { kind name ofType { kind name ofType { kind name } } } } }
}"#;

#[cfg(test)]
#[path = "../tests/unit/graphql.rs"]
mod tests;
