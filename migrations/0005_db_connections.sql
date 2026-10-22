-- Stage 8: database connections and SQL tools.

CREATE TABLE db_connections (
    id          TEXT PRIMARY KEY NOT NULL,
    project_id  TEXT NOT NULL REFERENCES projects (id) ON DELETE CASCADE,
    name        TEXT NOT NULL,
    driver      TEXT NOT NULL,              -- sqlite | postgres | mysql
    dsn_ref     TEXT NOT NULL,              -- env:/keyring: reference, never a raw DSN
    settings    TEXT NOT NULL,              -- JSON: access mode, limits, allowlist, masks
    created_at  TEXT NOT NULL,
    updated_at  TEXT NOT NULL,
    UNIQUE (project_id, name)
);

CREATE INDEX db_connections_project_id ON db_connections (project_id);

-- Copied from kind.connection_id (like auth_ref) so the database enforces it.
ALTER TABLE tools ADD COLUMN connection_ref TEXT REFERENCES db_connections (id) ON DELETE NO ACTION;
CREATE INDEX tools_connection_ref ON tools (connection_ref);
