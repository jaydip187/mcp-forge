//! Embedded web UI (`ui/dist`, built by `npm --prefix ui run build`).
//!
//! Release builds embed the files; debug builds read them from disk. If the UI
//! hasn't been built, `/` serves a short page explaining how to build it.

use axum::http::header::{CACHE_CONTROL, CONTENT_TYPE};
use axum::http::{HeaderValue, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use rust_embed::RustEmbed;

#[derive(RustEmbed)]
#[folder = "../../ui/dist"]
#[allow_missing = true]
struct Assets;

const CSP: &str = "default-src 'self'; img-src 'self' data:; style-src 'self' 'unsafe-inline'; \
                   connect-src 'self'; frame-ancestors 'none'; base-uri 'none'; form-action 'self'";

const NOT_BUILT: &str = "<!doctype html><meta charset=utf-8><title>mcp-forge</title>\
<body style=\"font-family:system-ui;max-width:40rem;margin:4rem auto;padding:0 1rem\">\
<h1>The web UI isn't built</h1><p>Build it, then rebuild the binary:</p>\
<pre>npm --prefix ui ci\nnpm --prefix ui run build\ncargo build</pre>\
<p>The REST API (<code>/api</code>) and MCP endpoints (<code>/mcp/&lt;slug&gt;</code>) work without it.</p>";

pub async fn serve(uri: Uri) -> Response {
    let path = uri.path().trim_start_matches('/');
    let path = if path.is_empty() { "index.html" } else { path };
    match Assets::get(path) {
        Some(file) => {
            let mime = file.metadata.mimetype().to_owned();
            // Vite fingerprints everything under assets/, so it can be cached forever.
            let cache = if path.starts_with("assets/") {
                "public, max-age=31536000, immutable"
            } else {
                "no-cache"
            };
            with_headers(file.data.into_owned(), &mime, cache, StatusCode::OK)
        }
        None if path == "index.html" => with_headers(
            NOT_BUILT.as_bytes().to_vec(),
            "text/html; charset=utf-8",
            "no-cache",
            StatusCode::OK,
        ),
        None => with_headers(
            b"not found".to_vec(),
            "text/plain; charset=utf-8",
            "no-cache",
            StatusCode::NOT_FOUND,
        ),
    }
}

fn with_headers(body: Vec<u8>, mime: &str, cache: &'static str, status: StatusCode) -> Response {
    let mut resp = (status, body).into_response();
    let h = resp.headers_mut();
    if let Ok(v) = HeaderValue::from_str(mime) {
        h.insert(CONTENT_TYPE, v);
    }
    h.insert(CACHE_CONTROL, HeaderValue::from_static(cache));
    h.insert("content-security-policy", HeaderValue::from_static(CSP));
    h.insert(
        "x-content-type-options",
        HeaderValue::from_static("nosniff"),
    );
    h.insert("referrer-policy", HeaderValue::from_static("no-referrer"));
    resp
}
