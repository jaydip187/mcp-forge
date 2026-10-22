-- Stage 10: every SQL write attempt that reached a database.

CREATE TABLE audit_log (
    id               TEXT PRIMARY KEY NOT NULL,
    project_id       TEXT NOT NULL REFERENCES projects (id) ON DELETE CASCADE,
    ts               TEXT NOT NULL,
    tool_id          TEXT,            -- no FK: entries outlive deleted tools
    tool_name        TEXT NOT NULL,
    connection_id    TEXT,
    connection_name  TEXT NOT NULL,
    origin           TEXT NOT NULL,   -- mcp | console
    statement_class  TEXT NOT NULL,   -- write | ddl
    sql              TEXT NOT NULL,
    params           TEXT NOT NULL,   -- JSON array of bound values
    rows_affected    INTEGER,
    outcome          TEXT NOT NULL,   -- committed | rolled_back | dry_run | failed
    error            TEXT
);

CREATE INDEX audit_log_project_ts ON audit_log (project_id, ts DESC);
