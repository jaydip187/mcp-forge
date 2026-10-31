//! HAR (HTTP Archive, as saved from browser dev tools) → OpenAPI 3.0 (see
//! [`crate::convert`]).
//!
//! Keeps API calls (JSON responses, or anything but GET), skips pages,
//! scripts, styles, images and fonts. Path segments that look like ids
//! (numbers, UUIDs, long hex) become parameters, so `/users/42` and
//! `/users/43` are one tool. No header, cookie or query *values* are kept:
//! captures are full of session tokens.

use forge_core::is_secret_name;
use serde_json::Value;

use crate::convert::{body_from, sample_param, split_url, unique_param, Collection, Op, Param};
use crate::ImportError;

const STATIC_EXT: &[&str] = &[
    ".js", ".mjs", ".css", ".map", ".png", ".jpg", ".jpeg", ".gif", ".svg", ".ico", ".webp",
    ".avif", ".woff", ".woff2", ".ttf", ".otf", ".html", ".htm", ".mp4", ".webm", ".mp3", ".wasm",
];

pub fn is_har(doc: &Value) -> bool {
    doc.pointer("/log/entries").is_some_and(Value::is_array)
}

/// Whether a path segment is an id rather than a fixed word.
pub fn looks_like_id(seg: &str) -> bool {
    let hex = |s: &str| s.chars().all(|c| c.is_ascii_hexdigit());
    let uuid = seg.len() == 36
        && seg.split('-').map(str::len).eq([8, 4, 4, 4, 12])
        && hex(&seg.replace('-', ""));
    !seg.is_empty()
        && (seg.chars().all(|c| c.is_ascii_digit())
            || uuid
            || (seg.len() >= 16 && hex(seg))
            || (seg.len() >= 20
                && seg.chars().any(|c| c.is_ascii_digit())
                && seg.chars().any(|c| c.is_ascii_alphabetic())
                && seg
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')))
}

pub fn to_openapi3(doc: &Value) -> Result<Value, ImportError> {
    let entries = doc
        .pointer("/log/entries")
        .and_then(Value::as_array)
        .ok_or_else(|| ImportError::Unsupported("This HAR file has no entries.".into()))?;
    let mut c = Collection::default();
    for e in entries {
        if let Some(op) = entry(&mut c, e) {
            c.ops.push(op);
        }
    }
    if c.ops.is_empty() {
        return Err(ImportError::Unsupported(
            "No API calls found in this HAR file (only pages, scripts, styles or images). \
             Record while using the app, then save with \"Save all as HAR\"."
                .into(),
        ));
    }
    let host = c
        .ops
        .iter()
        .filter_map(|o| o.base.as_deref())
        .next()
        .and_then(|b| b.split("://").nth(1))
        .unwrap_or("captured traffic")
        .to_owned();
    c.title = format!("HAR capture of {host}");
    Ok(c.into_openapi("har"))
}

fn entry(c: &mut Collection, e: &Value) -> Option<Op> {
    let req = e.get("request")?;
    let method = req.get("method")?.as_str()?.to_ascii_uppercase();
    if !matches!(method.as_str(), "GET" | "POST" | "PUT" | "PATCH" | "DELETE") {
        return None;
    }
    let (base, url) = split_url(req.get("url")?.as_str()?)?;
    let path = url.path();
    let lower = path.to_ascii_lowercase();
    if STATIC_EXT.iter().any(|x| lower.ends_with(x)) {
        return None;
    }
    let response_mime = e
        .pointer("/response/content/mimeType")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_ascii_lowercase();
    if method == "GET" && !response_mime.contains("json") {
        return None;
    }

    let mut op = Op {
        method,
        base,
        ..Op::default()
    };
    let mut segs = vec![];
    let mut prev = "";
    for seg in path.split('/').filter(|s| !s.is_empty()) {
        if looks_like_id(seg) {
            let base_name = if prev.is_empty() {
                "id".to_owned()
            } else {
                format!("{prev}_id")
            };
            let name = unique_param(&base_name, &op.path_params);
            segs.push(format!("{{{name}}}"));
            op.path_params.push(Param {
                name,
                required: true,
                ..Param::default()
            });
        } else {
            segs.push(seg.to_owned());
        }
        prev = seg;
    }
    op.path = format!("/{}", segs.join("/"));

    for (k, _) in url.query_pairs() {
        if op.query.iter().any(|p| p.name == k) {
            continue;
        }
        if is_secret_name(&k) {
            let s = c.api_key_scheme(&k, "query");
            op.security.get_or_insert_with(Vec::new).push(s);
            continue;
        }
        op.query.push(sample_param(&k, None, false));
    }

    let mut content_type = None;
    for h in req
        .get("headers")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let (Some(k), Some(v)) = (
            h.get("name").and_then(Value::as_str),
            h.get("value").and_then(Value::as_str),
        ) else {
            continue;
        };
        if k.eq_ignore_ascii_case("content-type") {
            content_type = Some(v.to_owned());
        }
        if let Some(s) = c.scheme_for_header(k, v) {
            if !op.security.as_ref().is_some_and(|x| x.contains(&s)) {
                op.security.get_or_insert_with(Vec::new).push(s);
            }
        }
        // Other headers are browser noise in a capture: none become parameters.
    }

    if let Some(pd) = req.get("postData") {
        let mime = pd
            .get("mimeType")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .or(content_type);
        op.body = body_from(
            pd.get("text").and_then(Value::as_str).unwrap_or(""),
            mime.as_deref(),
        );
    }
    Some(op)
}
