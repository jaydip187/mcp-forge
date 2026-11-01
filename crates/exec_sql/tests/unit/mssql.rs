use std::borrow::Cow;

use super::*;

#[test]
fn connection_strings() {
    let c =
        parse_dsn("mssql://reader:p%40ss@db.example.com:14330/shop?trust_server_certificate=true")
            .unwrap();
    assert_eq!(c.get_addr(), "db.example.com:14330");
    assert!(parse_dsn("sqlserver://reader:x@db/shop").is_ok());
    assert!(
        parse_dsn("Server=tcp:db.example.com,1433;Database=shop;User Id=reader;Password=x;")
            .is_ok()
    );
    assert!(parse_dsn(
        "jdbc:sqlserver://db.example.com:1433;databaseName=shop;user=reader;password=x"
    )
    .is_ok());
    let e = parse_dsn("postgres://x@y/z").unwrap_err();
    assert!(e.to_string().contains("mssql://"), "{e}");
}

#[test]
fn cells_to_json() {
    assert_eq!(cell(&ColumnData::I32(Some(7))), json!(7));
    assert_eq!(cell(&ColumnData::I64(None)), Value::Null);
    assert_eq!(cell(&ColumnData::Bit(Some(true))), json!(true));
    assert_eq!(
        cell(&ColumnData::String(Some(Cow::Borrowed("Ada")))),
        json!("Ada")
    );
    assert_eq!(
        cell(&ColumnData::Binary(Some(Cow::Borrowed(&[1, 2, 3])))),
        json!("<binary, 3 bytes>")
    );
    let long = "x".repeat(2_000);
    let capped = cell(&ColumnData::String(Some(Cow::Owned(long))));
    assert!(capped.as_str().unwrap().contains("more chars"));
}
