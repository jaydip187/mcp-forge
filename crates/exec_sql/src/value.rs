//! Database row → JSON cell conversion, per driver.

use chrono::{DateTime, NaiveDate, NaiveDateTime, NaiveTime, Utc};
use serde_json::{json, Value};
use sqlx::mysql::MySqlRow;
use sqlx::postgres::{PgRow, PgTypeKind};
use sqlx::sqlite::SqliteRow;
use sqlx::types::Decimal;
use sqlx::{Row, TypeInfo, ValueRef};

/// Text cells longer than this are cut, with a note saying how much.
pub const MAX_CELL_CHARS: usize = 1_000;

/// Caps long text so one cell can't flood the agent's context.
pub fn cap_cell(v: Value) -> Value {
    match v {
        Value::String(s) => Value::String(cap_str(s)),
        Value::Array(_) | Value::Object(_) => {
            let text = v.to_string();
            if text.chars().count() > MAX_CELL_CHARS {
                Value::String(cap_str(text))
            } else {
                v
            }
        }
        other => other,
    }
}

fn cap_str(s: String) -> String {
    let n = s.chars().count();
    if n <= MAX_CELL_CHARS {
        return s;
    }
    let mut out: String = s.chars().take(MAX_CELL_CHARS).collect();
    out.push_str(&format!("… [{} more chars]", n - MAX_CELL_CHARS));
    out
}

fn binary(bytes: &[u8]) -> Value {
    Value::String(format!("<binary, {} bytes>", bytes.len()))
}

fn unsupported(type_name: &str, hint: &str) -> Value {
    Value::String(format!("<unsupported type {type_name}; {hint}>"))
}

fn float(f: f64) -> Value {
    serde_json::Number::from_f64(f).map_or_else(|| Value::String(f.to_string()), Value::Number)
}

pub fn sqlite_row(row: &SqliteRow) -> Vec<Value> {
    (0..row.len())
        .map(|i| {
            let Ok(raw) = row.try_get_raw(i) else {
                return Value::Null;
            };
            if raw.is_null() {
                return Value::Null;
            }
            let ty = raw.type_info().name().to_owned();
            let v = match ty.as_str() {
                "INTEGER" => row.try_get_unchecked::<i64, _>(i).map(Value::from),
                "REAL" => row.try_get_unchecked::<f64, _>(i).map(float),
                "BLOB" => row.try_get_unchecked::<Vec<u8>, _>(i).map(|b| binary(&b)),
                _ => row.try_get_unchecked::<String, _>(i).map(Value::String),
            };
            cap_cell(v.unwrap_or_else(|_| unsupported(&ty, "cast it in the SQL")))
        })
        .collect()
}

pub fn pg_row(row: &PgRow) -> Vec<Value> {
    (0..row.len())
        .map(|i| {
            let Ok(raw) = row.try_get_raw(i) else {
                return Value::Null;
            };
            if raw.is_null() {
                return Value::Null;
            }
            let info = raw.type_info().into_owned();
            let ty = info.name().to_owned();
            let get = |r: Result<Value, sqlx::Error>| {
                r.unwrap_or_else(|_| unsupported(&ty, "cast it in the SQL, e.g. `col::text`"))
            };
            let v = match ty.as_str() {
                "BOOL" => get(row.try_get::<bool, _>(i).map(Value::from)),
                "INT2" => get(row.try_get::<i16, _>(i).map(Value::from)),
                "INT4" => get(row.try_get::<i32, _>(i).map(Value::from)),
                "INT8" => get(row.try_get::<i64, _>(i).map(Value::from)),
                "OID" => get(row
                    .try_get::<sqlx::postgres::types::Oid, _>(i)
                    .map(|o| Value::from(o.0))),
                "FLOAT4" => get(row.try_get::<f32, _>(i).map(|f| float(f64::from(f)))),
                "FLOAT8" => get(row.try_get::<f64, _>(i).map(float)),
                // Kept as text: exact decimals don't survive a JSON float.
                "NUMERIC" => get(row
                    .try_get::<Decimal, _>(i)
                    .map(|d| Value::String(d.to_string()))),
                "TEXT" | "VARCHAR" | "BPCHAR" | "NAME" | "CHAR" | "CITEXT" | "UNKNOWN" => {
                    get(row.try_get_unchecked::<String, _>(i).map(Value::String))
                }
                "UUID" => get(row
                    .try_get::<uuid::Uuid, _>(i)
                    .map(|u| Value::String(u.to_string()))),
                "JSON" | "JSONB" => get(row.try_get::<Value, _>(i)),
                "DATE" => get(row
                    .try_get::<NaiveDate, _>(i)
                    .map(|d| Value::String(d.to_string()))),
                "TIME" => get(row
                    .try_get::<NaiveTime, _>(i)
                    .map(|t| Value::String(t.to_string()))),
                "TIMESTAMP" => get(row
                    .try_get::<NaiveDateTime, _>(i)
                    .map(|t| Value::String(t.format("%Y-%m-%dT%H:%M:%S%.f").to_string()))),
                "TIMESTAMPTZ" => get(row
                    .try_get::<DateTime<Utc>, _>(i)
                    .map(|t| Value::String(t.to_rfc3339()))),
                "BYTEA" => get(row.try_get::<Vec<u8>, _>(i).map(|b| binary(&b))),
                "TEXT[]" | "VARCHAR[]" | "NAME[]" => {
                    get(row.try_get::<Vec<String>, _>(i).map(|v| json!(v)))
                }
                "INT2[]" => get(row.try_get::<Vec<i16>, _>(i).map(|v| json!(v))),
                "INT4[]" => get(row.try_get::<Vec<i32>, _>(i).map(|v| json!(v))),
                "INT8[]" => get(row.try_get::<Vec<i64>, _>(i).map(|v| json!(v))),
                "BOOL[]" => get(row.try_get::<Vec<bool>, _>(i).map(|v| json!(v))),
                "FLOAT8[]" => get(row.try_get::<Vec<f64>, _>(i).map(|v| json!(v))),
                _ if matches!(info.kind(), PgTypeKind::Enum(_)) => {
                    get(row.try_get_unchecked::<String, _>(i).map(Value::String))
                }
                _ => unsupported(&ty, "cast it in the SQL, e.g. `col::text`"),
            };
            cap_cell(v)
        })
        .collect()
}

pub fn mysql_row(row: &MySqlRow) -> Vec<Value> {
    (0..row.len())
        .map(|i| {
            let Ok(raw) = row.try_get_raw(i) else {
                return Value::Null;
            };
            if raw.is_null() {
                return Value::Null;
            }
            let ty = raw.type_info().name().to_owned();
            let v = match ty.as_str() {
                // TINYINT(1), MySQL's BOOL.
                "BOOLEAN" => row
                    .try_get_unchecked::<i64, _>(i)
                    .map(|v| Value::Bool(v != 0)),
                "TINYINT" | "SMALLINT" | "MEDIUMINT" | "INT" | "BIGINT" | "YEAR" => {
                    row.try_get_unchecked::<i64, _>(i).map(Value::from)
                }
                t if t.ends_with(" UNSIGNED") => {
                    row.try_get_unchecked::<u64, _>(i).map(Value::from)
                }
                "FLOAT" => row
                    .try_get_unchecked::<f32, _>(i)
                    .map(|f| float(f64::from(f))),
                "DOUBLE" => row.try_get_unchecked::<f64, _>(i).map(float),
                "DECIMAL" => row
                    .try_get::<Decimal, _>(i)
                    .map(|d| Value::String(d.to_string())),
                "DATE" => row
                    .try_get::<NaiveDate, _>(i)
                    .map(|d| Value::String(d.to_string())),
                "TIME" => row
                    .try_get::<NaiveTime, _>(i)
                    .map(|t| Value::String(t.to_string())),
                "DATETIME" => row
                    .try_get::<NaiveDateTime, _>(i)
                    .map(|t| Value::String(t.format("%Y-%m-%dT%H:%M:%S%.f").to_string())),
                "TIMESTAMP" => row
                    .try_get::<DateTime<Utc>, _>(i)
                    .map(|t| Value::String(t.to_rfc3339())),
                "JSON" => row.try_get::<Value, _>(i),
                "BINARY" | "VARBINARY" | "TINYBLOB" | "BLOB" | "MEDIUMBLOB" | "LONGBLOB"
                | "GEOMETRY" | "BIT" => {
                    row.try_get_unchecked::<Vec<u8>, _>(i)
                        .map(|b| match String::from_utf8(b) {
                            Ok(s) if ty != "GEOMETRY" && ty != "BIT" => Value::String(s),
                            Ok(s) => binary(s.as_bytes()),
                            Err(e) => binary(e.as_bytes()),
                        })
                }
                _ => row.try_get_unchecked::<String, _>(i).map(Value::String),
            };
            cap_cell(
                v.unwrap_or_else(|_| {
                    unsupported(&ty, "cast it in the SQL, e.g. CAST(col AS CHAR)")
                }),
            )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn caps_long_cells() {
        let long = "x".repeat(MAX_CELL_CHARS + 5);
        let Value::String(s) = cap_cell(Value::String(long)) else {
            panic!()
        };
        assert!(s.ends_with("… [5 more chars]"), "{s}");
        assert_eq!(cap_cell(json!("short")), json!("short"));
        assert_eq!(cap_cell(json!(5)), json!(5));
        let big = json!({ "a": "y".repeat(MAX_CELL_CHARS) });
        assert!(cap_cell(big).is_string());
    }
}
