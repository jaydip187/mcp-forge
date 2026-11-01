//! Denied functions and object-name helpers.

use sqlparser::ast::{ObjectName, ObjectNamePart};

/// Functions that can write, reach outside the database, read server files,
/// take locks, or stall the server. Matched on the unqualified name.
const DENIED_FUNCTIONS: &[&str] = &[
    // Postgres
    "pg_terminate_backend",
    "pg_cancel_backend",
    "pg_reload_conf",
    "pg_rotate_logfile",
    "pg_read_file",
    "pg_read_binary_file",
    "pg_ls_dir",
    "pg_stat_file",
    "pg_sleep",
    "pg_sleep_for",
    "pg_sleep_until",
    "pg_switch_wal",
    "pg_create_restore_point",
    "pg_logical_emit_message",
    "pg_notify",
    "set_config",
    "nextval",
    "setval",
    "query_to_xml",
    "query_to_xmlschema",
    "query_to_xml_and_xmlschema",
    "cursor_to_xml",
    "cursor_to_xmlschema",
    // These read whole tables by name or run SQL from a string, which would
    // get around the table allowlist.
    "table_to_xml",
    "table_to_xmlschema",
    "table_to_xml_and_xmlschema",
    "schema_to_xml",
    "schema_to_xmlschema",
    "schema_to_xml_and_xmlschema",
    "database_to_xml",
    "database_to_xmlschema",
    "database_to_xml_and_xmlschema",
    "ts_stat",
    // MySQL
    "sleep",
    "benchmark",
    "load_file",
    "get_lock",
    "release_lock",
    "release_all_locks",
    "master_pos_wait",
    "source_pos_wait",
    // SQLite
    "load_extension",
    "readfile",
    "writefile",
    "edit",
    // DuckDB: run SQL from a string, read other databases, env and secrets.
    // Files are checked against the allowed folders instead (`files`).
    "query",
    "query_table",
    "getenv",
    "sqlite_scan",
    "sqlite_attach",
    "postgres_scan",
    "postgres_scan_pushdown",
    "postgres_attach",
    "postgres_query",
    "mysql_scan",
    "mysql_query",
    "duckdb_secrets",
    "which_secret",
    // SQL Server: other servers and files.
    "openrowset",
    "openquery",
    "opendatasource",
    "fn_get_audit_file",
    "fn_xe_file_target_read_file",
    "fn_trace_gettable",
    // ClickHouse: files, URLs, other servers and programs.
    "file",
    "url",
    "urlcluster",
    "input",
    "remote",
    "remotesecure",
    "cluster",
    "clusterallreplicas",
    "s3",
    "s3cluster",
    "gcs",
    "hdfs",
    "hdfscluster",
    "azureblobstorage",
    "azureblobstoragecluster",
    "jdbc",
    "odbc",
    "mysql",
    "postgresql",
    "sqlite",
    "mongodb",
    "redis",
    "executable",
    "executablepool",
    "iceberg",
    "deltalake",
    "hudi",
    "dictionary",
    "sleepeachrow",
];

/// Name prefixes denied as families (`dblink_exec`, `lo_import`, …).
const DENIED_PREFIXES: &[&str] = &[
    "dblink",
    "lo_",
    "pg_advisory",
    "pg_try_advisory",
    // ClickHouse dictionaries and Join tables read data by name.
    "dictget",
    "joinget",
    "xp_",
];

pub(super) fn denied(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    DENIED_FUNCTIONS.contains(&n.as_str()) || DENIED_PREFIXES.iter().any(|p| n.starts_with(p))
}

pub(super) fn last_part(name: &ObjectName) -> String {
    match name.0.last() {
        Some(ObjectNamePart::Identifier(i)) => i.value.clone(),
        Some(ObjectNamePart::Function(f)) => f.name.value.clone(),
        None => String::new(),
    }
}

pub(super) fn full_name(name: &ObjectName) -> String {
    name.0
        .iter()
        .map(|p| match p {
            ObjectNamePart::Identifier(i) => i.value.clone(),
            ObjectNamePart::Function(f) => f.name.value.clone(),
        })
        .collect::<Vec<_>>()
        .join(".")
}
