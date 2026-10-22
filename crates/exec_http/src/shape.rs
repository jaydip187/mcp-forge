//! Response shaping: what an agent sees of a successful upstream response.
//!
//! JSON bodies can have nulls stripped and be transformed by a jq expression
//! (via jaq). Any body is then capped at a size limit with a notice saying
//! how much was cut.

use jaq_core::load::{Arena, File, Loader};
use jaq_core::{data, unwrap_valr, Compiler, Ctx, Vars};
use jaq_json::Val;
use serde_json::Value;

/// More outputs than this from one expression is treated as a runaway filter.
const MAX_OUTPUTS: usize = 10_000;

/// Per-project shaping settings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShapeOptions {
    pub strip_nulls: bool,
    /// Maximum characters returned to the agent.
    pub max_chars: usize,
}

/// What shaping did to a body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Shaped {
    /// Text for the agent (with a truncation notice when cut).
    pub text: String,
    /// True if stripping or a jq expression changed the body.
    pub transformed: bool,
    pub truncated: bool,
    /// The expression couldn't run; the unshaped body is used instead.
    pub shape_error: Option<String>,
}

/// Checks that `expr` parses and compiles. Returns a readable error.
pub fn check(expr: &str) -> Result<(), String> {
    compile(expr).map(|_| ())
}

fn compile(expr: &str) -> Result<jaq_core::Filter<data::JustLut<Val>>, String> {
    let defs = jaq_core::defs()
        .chain(jaq_std::defs())
        .chain(jaq_json::defs());
    let funs = jaq_core::funs()
        .chain(jaq_std::funs())
        .chain(jaq_json::funs());
    let loader = Loader::new(defs);
    let arena = Arena::default();
    let modules = loader
        .load(
            &arena,
            File {
                code: expr,
                path: (),
            },
        )
        .map_err(|errs| format!("can't parse the expression ({})", describe(&errs)))?;
    Compiler::default()
        .with_funs(funs)
        .compile(modules)
        .map_err(|errs| format!("can't compile the expression ({})", describe(&errs)))
}

/// jaq's error types are rich but not `Display`; their Debug form is short
/// enough once whitespace is squeezed.
fn describe(errs: &impl std::fmt::Debug) -> String {
    let s = format!("{errs:?}");
    let s: String = s.split_whitespace().collect::<Vec<_>>().join(" ");
    s.chars().take(300).collect()
}

/// Runs `expr` on `input`: no output → `null`, one → that value, many → array.
pub fn apply(expr: &str, input: &Value) -> Result<Value, String> {
    let filter = compile(expr)?;
    let text = serde_json::to_string(input).expect("JSON serializes");
    let val =
        jaq_json::read::parse_single(text.as_bytes()).map_err(|e| format!("bad input: {e:?}"))?;
    let ctx = Ctx::<data::JustLut<Val>>::new(&filter.lut, Vars::new([]));
    let mut outputs = Vec::new();
    for out in filter.id.run((ctx, val)).map(unwrap_valr) {
        let v = out.map_err(|e| format!("{e}"))?;
        if outputs.len() >= MAX_OUTPUTS {
            return Err(format!(
                "the expression produced more than {MAX_OUTPUTS} results"
            ));
        }
        let rendered = v.to_string();
        outputs.push(serde_json::from_str(&rendered).unwrap_or(Value::String(rendered)));
    }
    Ok(match outputs.len() {
        0 => Value::Null,
        1 => outputs.pop().expect("one output"),
        _ => Value::Array(outputs),
    })
}

/// Removes `null` members from objects, recursively. Array items stay put so
/// positions keep their meaning.
pub fn strip_nulls(v: &mut Value) {
    match v {
        Value::Object(m) => {
            m.retain(|_, x| !x.is_null());
            m.values_mut().for_each(strip_nulls);
        }
        Value::Array(items) => items.iter_mut().for_each(strip_nulls),
        _ => {}
    }
}

/// Shapes a successful response body for the agent.
pub fn shape_body(
    body: &str,
    content_type: Option<&str>,
    expr: Option<&str>,
    opts: &ShapeOptions,
) -> Shaped {
    let wants_json = opts.strip_nulls || expr.is_some();
    let looks_json = content_type.is_some_and(|c| c.contains("json"))
        || body.trim_start().starts_with(['{', '[']);
    let parsed: Option<Value> = (wants_json && looks_json)
        .then(|| serde_json::from_str(body).ok())
        .flatten();

    let mut shape_error = None;
    let (text, transformed) = match parsed {
        Some(mut v) => {
            if opts.strip_nulls {
                strip_nulls(&mut v);
            }
            if let Some(e) = expr {
                match apply(e, &v) {
                    Ok(out) => v = out,
                    Err(err) => shape_error = Some(err),
                }
            }
            (serde_json::to_string(&v).expect("JSON serializes"), true)
        }
        None => {
            if expr.is_some() {
                shape_error = Some("the response isn't JSON".into());
            }
            (body.to_owned(), false)
        }
    };
    let (text, truncated) = truncate(&text, opts.max_chars);
    let mut text = text;
    if let Some(err) = &shape_error {
        text =
            format!("[response_shape not applied: {err}. Showing the unshaped response.]\n{text}");
    }
    Shaped {
        text,
        transformed: transformed && shape_error.is_none(),
        truncated,
        shape_error,
    }
}

/// Cuts `text` to `max` characters and appends a notice saying what was cut.
pub fn truncate(text: &str, max: usize) -> (String, bool) {
    let total = text.chars().count();
    if total <= max {
        return (text.to_owned(), false);
    }
    let kept: String = text.chars().take(max).collect();
    (
        format!(
            "{kept}\n\n[Response truncated: showing the first {max} of {total} characters. \
             Ask for less data (filters, paging) or set a response_shape to keep only what's needed.]"
        ),
        true,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const OPTS: ShapeOptions = ShapeOptions {
        strip_nulls: false,
        max_chars: 1_000,
    };

    #[test]
    fn jq_expressions() {
        let users = json!([
            { "id": 1, "name": "Ada", "email": "ada@x.io", "meta": { "plan": "pro" } },
            { "id": 2, "name": "Linus", "email": null, "meta": { "plan": "free" } }
        ]);
        assert_eq!(
            apply("map({id, name})", &users).unwrap(),
            json!([{ "id": 1, "name": "Ada" }, { "id": 2, "name": "Linus" }])
        );
        assert_eq!(apply(".[0].meta.plan", &users).unwrap(), json!("pro"));
        assert_eq!(
            apply(".[] | .name", &users).unwrap(),
            json!(["Ada", "Linus"]),
            "many outputs → array"
        );
        assert_eq!(
            apply("empty", &users).unwrap(),
            Value::Null,
            "no output → null"
        );
        assert_eq!(apply("length", &users).unwrap(), json!(2));
        assert_eq!(
            apply("[.[] | select(.meta.plan == \"pro\") | .email]", &users).unwrap(),
            json!(["ada@x.io"])
        );
        assert_eq!(apply("map(.id) | add", &users).unwrap(), json!(3));
    }

    #[test]
    fn bad_expressions_are_reported() {
        assert!(check("map({id, name})").is_ok());
        let e = check(".foo | ").unwrap_err();
        assert!(e.contains("can't parse"), "{e}");
        let e = check("no_such_function(1)").unwrap_err();
        assert!(e.contains("can't compile"), "{e}");
        let e = apply("error(\"boom\")", &json!({ "a": 1 })).unwrap_err();
        assert!(e.contains("boom"), "{e}");
        let e = apply("repeat(1)", &json!(null)).unwrap_err();
        assert!(e.contains("more than"), "{e}");
    }

    #[test]
    fn strip_nulls_recursively() {
        let mut v = json!({ "a": null, "b": { "c": null, "d": 1 }, "e": [null, { "f": null }] });
        strip_nulls(&mut v);
        assert_eq!(v, json!({ "b": { "d": 1 }, "e": [null, {}] }));
    }

    #[test]
    fn truncation_notice() {
        let (t, cut) = truncate("héllo wörld", 5);
        assert!(cut);
        assert!(
            t.starts_with("héllo\n\n[Response truncated: showing the first 5 of 11 characters."),
            "{t}"
        );
        let (t, cut) = truncate("short", 5);
        assert_eq!((t.as_str(), cut), ("short", false));
    }

    #[test]
    fn shape_body_pipeline() {
        let body = r#"{"items":[{"id":1,"note":null},{"id":2,"note":"x"}],"next":null}"#;
        let s = shape_body(
            body,
            Some("application/json"),
            Some(".items | map(.id)"),
            &OPTS,
        );
        assert_eq!(s.text, "[1,2]");
        assert!(s.transformed && !s.truncated && s.shape_error.is_none());

        let strip = ShapeOptions {
            strip_nulls: true,
            max_chars: 1_000,
        };
        let s = shape_body(body, Some("application/json"), None, &strip);
        assert_eq!(s.text, r#"{"items":[{"id":1},{"id":2,"note":"x"}]}"#);

        // No shaping configured: the body passes through untouched.
        let s = shape_body(body, Some("application/json"), None, &OPTS);
        assert_eq!((s.text.as_str(), s.transformed), (body, false));

        // A failing expression falls back to the raw body with a note.
        let s = shape_body(
            body,
            Some("application/json"),
            Some("error(\"no such field\")"),
            &OPTS,
        );
        assert!(s.shape_error.is_some());
        assert!(
            s.text.starts_with("[response_shape not applied:"),
            "{}",
            s.text
        );
        assert!(s.text.ends_with(body));

        // Non-JSON with an expression: note, then the text.
        let s = shape_body("plain text", Some("text/plain"), Some("."), &OPTS);
        assert_eq!(s.shape_error.as_deref(), Some("the response isn't JSON"));

        // Size cap applies after shaping.
        let tiny = ShapeOptions {
            strip_nulls: false,
            max_chars: 10,
        };
        let s = shape_body(body, Some("application/json"), None, &tiny);
        assert!(s.truncated && s.text.starts_with(r#"{"items":["#));
    }
}
