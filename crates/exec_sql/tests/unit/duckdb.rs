use super::*;

const LIMITS: Limits = Limits {
    row_limit: 10,
    timeout_ms: 5_000,
};

/// A database file with one table, a CSV inside `allowed/` and one outside.
fn fixture() -> (tempfile::TempDir, String, String) {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("shop.duckdb");
    let c = Connection::open(&db).unwrap();
    c.execute_batch(
        "CREATE TABLE users (id INTEGER PRIMARY KEY, name VARCHAR, joined DATE, score DOUBLE, tags VARCHAR[]);
         INSERT INTO users VALUES (1, 'ada', DATE '2026-01-02', 1.5, ['a', 'b']), (2, 'bob', NULL, NULL, []);",
    )
    .unwrap();
    drop(c);
    let allowed = dir.path().join("allowed");
    std::fs::create_dir(&allowed).unwrap();
    std::fs::write(allowed.join("people.csv"), "id,name\n1,Ada\n2,Linus\n").unwrap();
    std::fs::write(dir.path().join("secret.csv"), "k,v\npw,hunter2\n").unwrap();
    let fwd = |p: &std::path::Path| p.to_string_lossy().replace('\\', "/");
    (dir, fwd(&db), fwd(&allowed))
}

#[tokio::test]
async fn reads_values_binds_and_caps_rows() {
    let (_d, db, _) = fixture();
    let pool = DuckdbPool::open(&db, false, &[]).unwrap();
    let r = run_read_only(
        &pool,
        "SELECT id, name, joined, score, tags FROM users WHERE id >= ? ORDER BY id",
        &[Bind::Int(1)],
        LIMITS,
    )
    .await
    .unwrap();
    assert_eq!(r.columns, ["id", "name", "joined", "score", "tags"]);
    assert_eq!(
        r.rows,
        [
            vec![
                json!(1),
                json!("ada"),
                json!("2026-01-02"),
                json!(1.5),
                json!(["a", "b"])
            ],
            vec![json!(2), json!("bob"), json!(null), json!(null), json!([])]
        ]
    );
    let r = run_read_only(&pool, "SELECT * FROM range(100)", &[], LIMITS)
        .await
        .unwrap();
    assert_eq!(r.rows.len(), 10);
    assert!(r.truncated);
}

#[tokio::test]
async fn read_only_file_refuses_writes() {
    let (_d, db, _) = fixture();
    let pool = DuckdbPool::open(&db, false, &[]).unwrap();
    // The session layer alone (no guard): the file is read-only.
    let e = run_read_only(&pool, "DELETE FROM users", &[], LIMITS)
        .await
        .unwrap_err();
    assert!(matches!(e, SqlError::Database(_)), "{e}");
    let e = run_write(
        &pool,
        "DELETE FROM users WHERE id = 1",
        &[],
        LIMITS,
        WriteOptions {
            max_affected: 10,
            dry_run: false,
            returning: false,
        },
    )
    .await
    .unwrap_err();
    assert!(matches!(e, SqlError::Rejected(_)), "{e}");
}

#[tokio::test]
async fn files_only_from_allowed_folders_and_settings_locked() {
    let (dir, _, allowed) = fixture();
    let pool = DuckdbPool::open(":memory:", false, std::slice::from_ref(&allowed)).unwrap();
    let ok = run_read_only(
        &pool,
        &format!("SELECT count(*) FROM read_csv('{allowed}/people.csv')"),
        &[],
        LIMITS,
    )
    .await
    .unwrap();
    assert_eq!(ok.rows, [[json!(2)]]);
    let outside = dir
        .path()
        .join("secret.csv")
        .to_string_lossy()
        .replace('\\', "/");
    for sql in [
        format!("SELECT * FROM read_csv('{outside}')"),
        format!("SELECT * FROM '{outside}'"),
        format!("SELECT * FROM read_csv('{allowed}/../secret.csv')"),
        "SET enable_external_access = true".to_owned(),
        "SET allowed_directories = ['/']".to_owned(),
        "INSTALL httpfs".to_owned(),
        format!("ATTACH '{}/other.duckdb'", allowed),
    ] {
        let e = run_read_only(&pool, &sql, &[], LIMITS).await;
        assert!(e.is_err(), "{sql} should fail: {e:?}");
    }
    // No folders: no files at all.
    let none = DuckdbPool::open(":memory:", false, &[]).unwrap();
    let e = run_read_only(
        &none,
        &format!("SELECT * FROM read_csv('{allowed}/people.csv')"),
        &[],
        LIMITS,
    )
    .await;
    assert!(e.is_err(), "{e:?}");
}

#[tokio::test]
async fn timeouts_interrupt_the_query() {
    let pool = DuckdbPool::open(":memory:", false, &[]).unwrap();
    let started = std::time::Instant::now();
    let e = run_read_only(
        &pool,
        "SELECT count(*) FROM range(1000000000) a, range(1000) b",
        &[],
        Limits {
            row_limit: 10,
            timeout_ms: 300,
        },
    )
    .await
    .unwrap_err();
    assert_eq!(e, SqlError::Timeout(0));
    assert!(started.elapsed() < std::time::Duration::from_secs(5));
    // The connection still works afterwards.
    let r = run_read_only(&pool, "SELECT 1", &[], LIMITS).await.unwrap();
    assert_eq!(r.rows, [[json!(1)]]);
}

#[tokio::test]
async fn writes_are_capped_and_dry_runnable() {
    let (_d, db, _) = fixture();
    let pool = DuckdbPool::open(&db, true, &[]).unwrap();
    let opts = |max, dry_run, returning| WriteOptions {
        max_affected: max,
        dry_run,
        returning,
    };
    let e = run_write(
        &pool,
        "UPDATE users SET name = 'x' WHERE id > 0",
        &[],
        LIMITS,
        opts(1, false, false),
    )
    .await
    .unwrap_err();
    assert!(
        matches!(e, SqlError::TooManyRows { affected: 2, .. }),
        "{e}"
    );
    let dry = run_write(
        &pool,
        "DELETE FROM users WHERE id = ?",
        &[Bind::Int(2)],
        LIMITS,
        opts(5, true, false),
    )
    .await
    .unwrap();
    assert_eq!((dry.affected, dry.committed), (1, false));
    let done = run_write(
        &pool,
        "UPDATE users SET name = ? WHERE id = 2 RETURNING id, name",
        &[Bind::Text("zed".into())],
        LIMITS,
        opts(5, false, true),
    )
    .await
    .unwrap();
    assert_eq!((done.affected, done.committed), (1, true));
    assert_eq!(done.rows, [[json!(2), json!("zed")]]);
    let r = run_read_only(&pool, "SELECT name FROM users ORDER BY id", &[], LIMITS)
        .await
        .unwrap();
    assert_eq!(r.rows, [[json!("ada")], [json!("zed")]]);
    let report = probe(&pool).await.unwrap();
    assert_eq!(report.can_write, Some(true));
    assert!(report.server_version.unwrap().starts_with("DuckDB v"));
}
