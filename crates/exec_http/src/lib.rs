//! HTTP executor: turns a tool's IR plus agent arguments into an upstream
//! request, applies auth from secret references, and returns the response.
//!
//! Secret values are never logged or returned: they are redacted from the
//! reported URL, error messages and response bodies.

mod args;
mod secrets;
pub mod shape;

use std::sync::Arc;
use std::time::Duration;

use forge_core::{
    ApiKeyLocation, ArgTarget, AuthConfig, AuthKind, HttpMethod, HttpSpec, ParamLocation,
    SecretResolver, ValidationErrors,
};
use reqwest::header::{HeaderMap, HeaderName, HeaderValue, AUTHORIZATION};
use reqwest::Url;
use serde_json::{Map, Value};

pub use args::encode_path_segment;
pub use forge_core::check_args;
pub use secrets::{DefaultSecretResolver, MapSecretResolver, KEYRING_SERVICE};

/// Upstream bodies larger than this are cut off and flagged `truncated`.
pub const MAX_RESPONSE_BYTES: usize = 5 * 1024 * 1024;
const MAX_REDIRECTS: usize = 5;
const REDACTED: &str = "[REDACTED]";

#[derive(Debug, thiserror::Error)]
pub enum ExecError {
    /// The agent's arguments are wrong; each error names the argument.
    #[error("invalid arguments: {0}")]
    InvalidArgs(ValidationErrors),
    /// The tool or its auth is misconfigured (e.g. secret not set).
    #[error("configuration error: {0}")]
    Config(String),
    /// The request could not be completed (connect, timeout, TLS…).
    #[error("request failed: {0}")]
    Request(String),
}

/// What was sent and what came back.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpOutcome {
    pub method: String,
    /// Final request URL with secrets redacted.
    pub url: String,
    pub status: u16,
    pub content_type: Option<String>,
    pub body: String,
    /// True if the body exceeded [`MAX_RESPONSE_BYTES`] and was cut.
    pub truncated: bool,
}

impl HttpOutcome {
    pub fn is_success(&self) -> bool {
        (200..400).contains(&self.status)
    }

    pub fn status_line(&self) -> String {
        let reason = reqwest::StatusCode::from_u16(self.status)
            .ok()
            .and_then(|s| s.canonical_reason())
            .unwrap_or("");
        format!("{} {reason}", self.status).trim_end().to_owned()
    }
}

#[derive(Clone)]
pub struct HttpExecutor {
    client: reqwest::Client,
    secrets: Arc<dyn SecretResolver>,
}

impl HttpExecutor {
    pub fn new(secrets: Arc<dyn SecretResolver>) -> Self {
        // Follow redirects only within the same host so credentials in custom
        // headers never travel to a different origin.
        let policy = reqwest::redirect::Policy::custom(|attempt| {
            let same_host =
                attempt.previous().first().map(|u| u.host_str()) == Some(attempt.url().host_str());
            if attempt.previous().len() > MAX_REDIRECTS {
                attempt.error("too many redirects")
            } else if same_host {
                attempt.follow()
            } else {
                attempt.stop()
            }
        });
        let client = reqwest::Client::builder()
            .redirect(policy)
            .user_agent(concat!("mcp-forge/", env!("CARGO_PKG_VERSION")))
            .build()
            .expect("reqwest client config is static");
        Self { client, secrets }
    }

    /// Executes `spec` with `args`. Upstream 4xx/5xx are returned as `Ok`
    /// outcomes; only argument, configuration and transport problems are `Err`.
    pub async fn execute(
        &self,
        spec: &HttpSpec,
        auth: Option<&AuthConfig>,
        args: &Map<String, Value>,
    ) -> Result<HttpOutcome, ExecError> {
        let derived = spec.derive_input();
        check_args(&derived.bindings, &derived.schema, args).map_err(ExecError::InvalidArgs)?;

        // Collect (location, IR name, wire name, value): fixed values first,
        // then agent args. Path placeholders use the IR name; everything else
        // goes out under the wire name.
        let mut values: Vec<(ParamLocation, &str, &str, &Value)> = spec
            .params
            .iter()
            .filter_map(|p| {
                p.fixed_value
                    .as_ref()
                    .map(|v| (p.location, p.name.as_str(), p.wire(), v))
            })
            .collect();
        let mut whole_body: Option<&Value> = None;
        for b in &derived.bindings {
            let Some(v) = args.get(&b.arg).filter(|v| !v.is_null()) else {
                continue;
            };
            match &b.target {
                ArgTarget::Param {
                    location,
                    name,
                    wire,
                } => values.push((*location, name, wire, v)),
                ArgTarget::WholeBody => whole_body = Some(v),
                // SQL bindings never come from an HTTP spec.
                ArgTarget::SqlParam { .. } => {}
            }
        }

        // Path.
        let mut path = spec.path_template.clone();
        for (loc, name, _, v) in &values {
            if *loc == ParamLocation::Path {
                let seg = encode_path_segment(&args::joined_value(v));
                path = path.replace(&format!("{{{name}}}"), &seg);
            }
        }
        let raw_url = format!("{}{}", spec.base_url.trim_end_matches('/'), path);
        let mut url = Url::parse(&raw_url)
            .map_err(|e| ExecError::Config(format!("cannot build request URL: {e}")))?;

        // Query.
        {
            let mut q = url.query_pairs_mut();
            for (loc, _, wire, v) in &values {
                if *loc == ParamLocation::Query {
                    for item in args::query_values(v) {
                        q.append_pair(wire, &item);
                    }
                }
            }
        }
        if url.query() == Some("") {
            url.set_query(None);
        }

        // Headers.
        let mut headers = HeaderMap::new();
        let mut bad = ValidationErrors::default();
        for (loc, name, wire, v) in &values {
            if *loc != ParamLocation::Header {
                continue;
            }
            match (
                HeaderName::from_bytes(wire.as_bytes()),
                HeaderValue::from_str(&args::joined_value(v)),
            ) {
                (Ok(n), Ok(val)) => {
                    headers.insert(n, val);
                }
                _ => bad.push(*name, "not a valid HTTP header value"),
            }
        }
        if !bad.is_empty() {
            return Err(ExecError::InvalidArgs(bad));
        }

        // Body.
        let body_fields: Map<String, Value> = values
            .iter()
            .filter(|(loc, ..)| *loc == ParamLocation::Body)
            .map(|(_, _, wire, v)| ((*wire).to_owned(), (*v).clone()))
            .collect();
        let body = match whole_body {
            Some(v) => Some(v.clone()),
            None if !body_fields.is_empty() => Some(Value::Object(body_fields)),
            None => None,
        };

        // Auth.
        let mut secrets_used = Vec::new();
        if let Some(cfg) = auth {
            let secret = self.resolve_secret(cfg).await?;
            match &cfg.kind {
                AuthKind::Bearer => {
                    headers.insert(AUTHORIZATION, sensitive(&format!("Bearer {secret}"))?);
                }
                AuthKind::Basic { username } => {
                    let token = base64_encode(format!("{username}:{secret}").as_bytes());
                    headers.insert(AUTHORIZATION, sensitive(&format!("Basic {token}"))?);
                    secrets_used.push(token);
                }
                AuthKind::ApiKey { location, name } => match location {
                    ApiKeyLocation::Header => {
                        let n = HeaderName::from_bytes(name.as_bytes()).map_err(|_| {
                            ExecError::Config(format!("auth header name `{name}` is invalid"))
                        })?;
                        headers.insert(n, sensitive(&secret)?);
                    }
                    ApiKeyLocation::Query => {
                        url.query_pairs_mut().append_pair(name, &secret);
                    }
                },
            }
            secrets_used.push(secret);
        }
        let redact = |s: &str| redact_all(s, &secrets_used);

        let method = to_reqwest_method(spec.method);
        let mut req = self
            .client
            .request(method.clone(), url.clone())
            .headers(headers)
            .timeout(Duration::from_millis(u64::from(spec.timeout_ms)));
        if let Some(b) = &body {
            req = req.json(b);
        }

        tracing::debug!(method = %method, url = %redact(url.as_str()), "upstream request");
        let mut resp = req.send().await.map_err(|e| {
            let msg = if e.is_timeout() {
                format!("timed out after {} ms", spec.timeout_ms)
            } else if e.is_connect() {
                format!(
                    "could not connect to {}: {}",
                    redact(url.as_str()),
                    e.without_url()
                )
            } else {
                e.without_url().to_string()
            };
            ExecError::Request(redact(&msg))
        })?;

        let status = resp.status().as_u16();
        let content_type = resp
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned);
        let mut bytes = Vec::new();
        let mut truncated = false;
        loop {
            match resp.chunk().await {
                Ok(Some(chunk)) => {
                    let room = MAX_RESPONSE_BYTES - bytes.len();
                    if chunk.len() > room {
                        bytes.extend_from_slice(&chunk[..room]);
                        truncated = true;
                        break;
                    }
                    bytes.extend_from_slice(&chunk);
                }
                Ok(None) => break,
                Err(e) => {
                    let msg = if e.is_timeout() {
                        format!(
                            "timed out after {} ms while reading the response",
                            spec.timeout_ms
                        )
                    } else {
                        format!("error reading response: {}", e.without_url())
                    };
                    return Err(ExecError::Request(redact(&msg)));
                }
            }
        }

        Ok(HttpOutcome {
            method: method.to_string(),
            url: redact(resp.url().as_str()),
            status,
            content_type,
            body: redact(&String::from_utf8_lossy(&bytes)),
            truncated,
        })
    }

    async fn resolve_secret(&self, cfg: &AuthConfig) -> Result<String, ExecError> {
        let resolver = self.secrets.clone();
        let r = cfg.secret_ref.clone();
        let name = cfg.name.clone();
        let value = tokio::task::spawn_blocking(move || resolver.resolve(&r))
            .await
            .map_err(|e| ExecError::Config(format!("secret lookup panicked: {e}")))?
            .map_err(|e| ExecError::Config(format!("auth config `{name}`: {e}")))?;
        if value.is_empty() {
            return Err(ExecError::Config(format!(
                "auth config `{name}`: secret `{}` is empty",
                cfg.secret_ref
            )));
        }
        Ok(value)
    }
}

fn sensitive(v: &str) -> Result<HeaderValue, ExecError> {
    let mut h = HeaderValue::from_str(v).map_err(|_| {
        ExecError::Config("secret contains characters not allowed in a header".into())
    })?;
    h.set_sensitive(true);
    Ok(h)
}

fn to_reqwest_method(m: HttpMethod) -> reqwest::Method {
    match m {
        HttpMethod::Get => reqwest::Method::GET,
        HttpMethod::Post => reqwest::Method::POST,
        HttpMethod::Put => reqwest::Method::PUT,
        HttpMethod::Patch => reqwest::Method::PATCH,
        HttpMethod::Delete => reqwest::Method::DELETE,
        HttpMethod::Head => reqwest::Method::HEAD,
        HttpMethod::Options => reqwest::Method::OPTIONS,
    }
}

/// Replaces every secret (raw and URL-encoded forms) with a marker.
fn redact_all(s: &str, secrets: &[String]) -> String {
    let mut out = s.to_owned();
    for secret in secrets.iter().filter(|s| !s.is_empty()) {
        out = out.replace(secret.as_str(), REDACTED);
        let encoded: String = url_encode(secret);
        if encoded != *secret {
            out = out.replace(&encoded, REDACTED);
        }
    }
    out
}

fn url_encode(s: &str) -> String {
    let mut u = Url::parse("http://x/").expect("static URL");
    u.query_pairs_mut().append_pair("k", s);
    u.query().unwrap_or("").trim_start_matches("k=").to_owned()
}

fn base64_encode(input: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(input.len().div_ceil(3) * 4);
    for chunk in input.chunks(3) {
        let b = [
            chunk[0],
            *chunk.get(1).unwrap_or(&0),
            *chunk.get(2).unwrap_or(&0),
        ];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        out.push(T[(n >> 18) as usize & 63] as char);
        out.push(T[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 {
            T[(n >> 6) as usize & 63] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            T[n as usize & 63] as char
        } else {
            '='
        });
    }
    out
}

#[cfg(test)]
#[path = "../tests/unit/lib.rs"]
mod tests;
