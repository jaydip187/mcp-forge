//! curl commands (as copied from docs, terminals or "Copy as cURL") →
//! OpenAPI 3.0 (see [`crate::convert`]). Several commands can be pasted at
//! once; each becomes one tool. Line continuations for bash (`\\`), cmd
//! (`^`) and PowerShell (`` ` ``) are understood.

use forge_core::is_secret_name;
use serde_json::Value;

use crate::convert::{body_from, header_dropped, sample_param, split_url, Body, Collection, Op};
use crate::ImportError;

/// Whether `text` is curl commands (not a JSON/YAML document).
pub fn is_curl(text: &str) -> bool {
    let t = text.trim_start();
    t.starts_with("curl ") || t.starts_with("curl.exe ") || t.starts_with("curl\t")
}

/// Splits shell words: single quotes literal, double quotes with `\`
/// escapes, `$'…'` with C escapes, `\` escapes outside quotes. Newlines
/// end a word like spaces do.
pub fn words(text: &str) -> Result<Vec<String>, ImportError> {
    let mut out = vec![];
    let mut cur = String::new();
    let mut in_word = false;
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            c if c.is_whitespace() => {
                if in_word {
                    out.push(std::mem::take(&mut cur));
                    in_word = false;
                }
            }
            '\'' => {
                in_word = true;
                loop {
                    match chars.next() {
                        Some('\'') => break,
                        Some(x) => cur.push(x),
                        None => return Err(unclosed("'")),
                    }
                }
            }
            '$' if chars.peek() == Some(&'\'') => {
                chars.next();
                in_word = true;
                loop {
                    match chars.next() {
                        Some('\'') => break,
                        Some('\\') => match chars.next() {
                            Some('n') => cur.push('\n'),
                            Some('t') => cur.push('\t'),
                            Some('r') => cur.push('\r'),
                            Some(x) => cur.push(x),
                            None => return Err(unclosed("$'")),
                        },
                        Some(x) => cur.push(x),
                        None => return Err(unclosed("$'")),
                    }
                }
            }
            '"' => {
                in_word = true;
                loop {
                    match chars.next() {
                        Some('"') => break,
                        Some('\\') => match chars.next() {
                            Some(x @ ('"' | '\\' | '$' | '`')) => cur.push(x),
                            Some('\n') => {}
                            Some(x) => {
                                cur.push('\\');
                                cur.push(x);
                            }
                            None => return Err(unclosed("\"")),
                        },
                        Some(x) => cur.push(x),
                        None => return Err(unclosed("\"")),
                    }
                }
            }
            '\\' => match chars.next() {
                Some('\n') => {}
                Some('\r') if chars.peek() == Some(&'\n') => {
                    chars.next();
                }
                Some(x) => {
                    in_word = true;
                    cur.push(x);
                }
                None => {}
            },
            // cmd.exe and PowerShell line continuations.
            '^' | '`' if matches!(chars.peek(), Some('\n' | '\r')) => {
                while matches!(chars.peek(), Some('\n' | '\r')) {
                    chars.next();
                }
            }
            x => {
                in_word = true;
                cur.push(x);
            }
        }
    }
    if in_word {
        out.push(cur);
    }
    Ok(out)
}

fn unclosed(q: &str) -> ImportError {
    ImportError::Parse(format!("a {q} quote in the curl command isn't closed"))
}

/// Options that take a value (the next word, or attached: `-XPOST`, `--header=…`).
const WITH_VALUE: &[&str] = &[
    "-X",
    "--request",
    "-H",
    "--header",
    "-d",
    "--data",
    "--data-raw",
    "--data-binary",
    "--data-ascii",
    "--data-urlencode",
    "--json",
    "-u",
    "--user",
    "--url",
    "-A",
    "--user-agent",
    "-e",
    "--referer",
    "-b",
    "--cookie",
    "-c",
    "--cookie-jar",
    "-o",
    "--output",
    "-m",
    "--max-time",
    "--connect-timeout",
    "-x",
    "--proxy",
    "-F",
    "--form",
    "-T",
    "--upload-file",
    "--cacert",
    "--cert",
    "-E",
    "--key",
    "-w",
    "--write-out",
    "--retry",
    "--resolve",
    "--oauth2-bearer",
    "-K",
    "--config",
];

#[derive(Default)]
struct Parsed {
    method: Option<String>,
    url: Option<String>,
    headers: Vec<(String, String)>,
    data: Vec<String>,
    json: bool,
    form: bool,
    get: bool,
    basic: bool,
    bearer: bool,
}

fn parse_command(args: &[String]) -> Parsed {
    let mut p = Parsed::default();
    let mut i = 0;
    while i < args.len() {
        let a = &args[i];
        i += 1;
        let (opt, attached): (&str, Option<String>) =
            if let Some((o, v)) = a.split_once('=').filter(|_| a.starts_with("--")) {
                (o, Some(v.to_owned()))
            } else if a.starts_with('-')
                && !a.starts_with("--")
                && a.len() > 2
                && WITH_VALUE.contains(&&a[..2])
            {
                (&a[..2], Some(a[2..].to_owned()))
            } else {
                (a.as_str(), None)
            };
        if !opt.starts_with('-') || opt == "-" {
            if p.url.is_none() {
                p.url = Some(a.clone());
            }
            continue;
        }
        let value = if WITH_VALUE.contains(&opt) {
            match attached {
                Some(v) => Some(v),
                None => {
                    i += 1;
                    args.get(i - 1).cloned()
                }
            }
        } else {
            None
        };
        let v = value.unwrap_or_default();
        match opt {
            "-X" | "--request" => p.method = Some(v.to_ascii_uppercase()),
            "-H" | "--header" => {
                if let Some((k, val)) = v.split_once(':') {
                    p.headers.push((k.trim().to_owned(), val.trim().to_owned()));
                }
            }
            "-d" | "--data" | "--data-raw" | "--data-binary" | "--data-ascii"
            | "--data-urlencode" => p.data.push(v),
            "--json" => {
                p.json = true;
                p.data.push(v);
            }
            "-F" | "--form" => p.form = true,
            "-u" | "--user" => p.basic = true,
            "--oauth2-bearer" => p.bearer = true,
            "--url" => p.url = Some(v),
            "-G" | "--get" => p.get = true,
            _ => {}
        }
    }
    p
}

pub fn to_openapi3(text: &str) -> Result<Value, ImportError> {
    let all = words(text)?;
    // Each `curl` word starts a command.
    let mut commands: Vec<&[String]> = vec![];
    let mut start = None;
    for (i, w) in all.iter().enumerate() {
        if w == "curl" || w == "curl.exe" {
            if let Some(s) = start {
                commands.push(&all[s..i]);
            }
            start = Some(i + 1);
        }
    }
    if let Some(s) = start {
        commands.push(&all[s..]);
    }
    let mut c = Collection {
        title: "curl commands".into(),
        ..Collection::default()
    };
    for (n, args) in commands.iter().enumerate() {
        let p = parse_command(args);
        let url = p.url.as_deref().ok_or_else(|| {
            ImportError::Unsupported(format!("curl command {} has no URL", n + 1))
        })?;
        let with_scheme = if url.contains("://") {
            url.to_owned()
        } else {
            format!("http://{url}")
        };
        let (base, u) = split_url(&with_scheme).ok_or_else(|| {
            ImportError::Unsupported(format!("curl command {} has an invalid URL: {url}", n + 1))
        })?;
        let method = p.method.clone().unwrap_or_else(|| {
            if !p.data.is_empty() && !p.get || p.form {
                "POST".into()
            } else {
                "GET".into()
            }
        });
        let mut op = Op {
            method,
            base,
            path: if u.path().is_empty() {
                "/".into()
            } else {
                u.path().to_owned()
            },
            ..Op::default()
        };
        let mut query: Vec<(String, String)> = u
            .query_pairs()
            .map(|(k, v)| (k.into_owned(), v.into_owned()))
            .collect();
        if p.get {
            for d in &p.data {
                query.extend(
                    url::form_urlencoded::parse(d.as_bytes())
                        .map(|(k, v)| (k.into_owned(), v.into_owned())),
                );
            }
        }
        for (k, v) in query {
            if op.query.iter().any(|q| q.name == k) {
                continue;
            }
            if is_secret_name(&k) {
                let s = c.api_key_scheme(&k, "query");
                op.security.get_or_insert_with(Vec::new).push(s);
                continue;
            }
            op.query.push(sample_param(&k, Some(&v), true));
        }
        let mut content_type = p.json.then(|| "application/json".to_owned());
        for (k, v) in &p.headers {
            if k.eq_ignore_ascii_case("content-type") {
                content_type = Some(v.clone());
            }
            if let Some(s) = c.scheme_for_header(k, v) {
                op.security.get_or_insert_with(Vec::new).push(s);
                continue;
            }
            if header_dropped(k) || op.headers.iter().any(|h| h.name.eq_ignore_ascii_case(k)) {
                continue;
            }
            op.headers.push(sample_param(k, Some(v), true));
        }
        if p.basic {
            let s = c.add_scheme(
                "basicAuth",
                serde_json::json!({ "type": "http", "scheme": "basic" }),
            );
            op.security.get_or_insert_with(Vec::new).push(s);
        }
        if p.bearer {
            let s = c.add_scheme(
                "bearerAuth",
                serde_json::json!({ "type": "http", "scheme": "bearer" }),
            );
            op.security.get_or_insert_with(Vec::new).push(s);
        }
        op.body = if p.form {
            Body::Other("multipart/form-data".into())
        } else if p.get || p.data.is_empty() {
            Body::None
        } else {
            body_from(&p.data.join("&"), content_type.as_deref())
        };
        c.ops.push(op);
    }
    if c.ops.is_empty() {
        return Err(ImportError::Unsupported("No curl command found.".into()));
    }
    Ok(c.into_openapi("curl"))
}
