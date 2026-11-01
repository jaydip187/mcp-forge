use serde_json::json;
use wiremock::matchers::{header, method, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

use super::*;

#[test]
fn placeholders_get_their_value_types() {
    let (sql, params) = typed(
        "SELECT * FROM t WHERE a = {p1:String} AND b = {p2:String} OR a = {p1:String} AND c = {p3:String}",
        &[Bind::Int(7), Bind::Text("x\ty".into()), Bind::Null("boolean")],
    );
    assert_eq!(
        sql,
        "SELECT * FROM t WHERE a = {p1:Int64} AND b = {p2:String} OR a = {p1:Int64} AND c = {p3:Nullable(Bool)}"
    );
    assert_eq!(
        params,
        [
            ("param_p1".to_owned(), "7".to_owned()),
            ("param_p2".to_owned(), "x\\ty".to_owned()),
            ("param_p3".to_owned(), "\\N".to_owned()),
        ]
    );
}

#[test]
fn connection_strings() {
    let p = ClickhousePool::new("clickhouse://ro:s%40cret@db.example.com:8123/shop", 1000).unwrap();
    assert_eq!(p.base.as_str(), "http://db.example.com:8123/");
    assert_eq!(p.user.as_deref(), Some("ro"));
    assert_eq!(p.password.as_deref(), Some("s@cret"));
    assert_eq!(p.database.as_deref(), Some("shop"));
    assert!(ClickhousePool::new("tcp://db:9000", 1000).is_err());
    assert!(
        !format!("{p:?}").contains("cret"),
        "Debug hides the password"
    );
}

#[tokio::test]
async fn sends_read_only_queries_with_limits() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(query_param("readonly", "2"))
        .and(query_param("max_result_rows", "3"))
        .and(query_param("max_execution_time", "1.5"))
        .and(query_param("param_p1", "ada"))
        .and(header("X-ClickHouse-User", "ro"))
        .and(header("X-ClickHouse-Database", "shop"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "meta": [{ "name": "id", "type": "UInt64" }, { "name": "name", "type": "String" }],
            "data": [[1, "ada"], [2, "ada"], [3, "ada"]],
            "rows": 3
        })))
        .expect(1)
        .mount(&server)
        .await;
    let uri = server.uri().replace("http://", "http://ro:pw@");
    let pool = ClickhousePool::new(&format!("{uri}/shop"), 1500).unwrap();
    let limits = Limits {
        row_limit: 2,
        timeout_ms: 1500,
    };
    let r = run_read_only(
        &pool,
        "SELECT id, name FROM users WHERE name = {p1:String}",
        &[Bind::Text("ada".into())],
        limits,
    )
    .await
    .unwrap();
    assert_eq!(r.columns, ["id", "name"]);
    assert_eq!(r.rows.len(), 2);
    assert!(r.truncated);
}

#[tokio::test]
async fn errors_and_timeouts() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(query_param("param_p1", "slow"))
        .respond_with(ResponseTemplate::new(500).set_body_string(
            "Code: 159. DB::Exception: Timeout exceeded: elapsed 1.2 seconds. (TIMEOUT_EXCEEDED)",
        ))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(500).set_body_string(
            "Code: 164. DB::Exception: Cannot execute query in readonly mode. (READONLY) (version 24.8)\n",
        ))
        .mount(&server)
        .await;
    let pool = ClickhousePool::new(&server.uri(), 1000).unwrap();
    let limits = Limits {
        row_limit: 10,
        timeout_ms: 1000,
    };
    let e = run_read_only(
        &pool,
        "SELECT {p1:String}",
        &[Bind::Text("slow".into())],
        limits,
    )
    .await
    .unwrap_err();
    assert_eq!(e, SqlError::Timeout(0));
    let e = run_read_only(&pool, "INSERT INTO t VALUES (1)", &[], limits)
        .await
        .unwrap_err();
    assert_eq!(
        e,
        SqlError::Database(
            "Cannot execute query in readonly mode. (READONLY) (version 24.8)".into()
        )
    );
}
