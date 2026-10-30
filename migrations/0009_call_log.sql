-- Stage 16: every tool call, from agents and the test console.

CREATE TABLE call_log (
    id               TEXT PRIMARY KEY NOT NULL,
    project_id       TEXT NOT NULL REFERENCES projects (id) ON DELETE CASCADE,
    ts               TEXT NOT NULL,
    tool_id          TEXT,            -- not a foreign key: entries outlive tools
    tool_name        TEXT NOT NULL,
    origin           TEXT NOT NULL,   -- mcp | console
    client           TEXT,
    arguments        TEXT NOT NULL,   -- JSON, secret-looking values redacted
    status           TEXT NOT NULL,   -- ok | error | unknown_tool
    duration_ms      INTEGER NOT NULL,
    response_chars   INTEGER NOT NULL,
    upstream_status  INTEGER,
    error            TEXT
);

CREATE INDEX call_log_project_ts ON call_log (project_id, ts);
CREATE INDEX call_log_project_tool_ts ON call_log (project_id, tool_name, ts);
