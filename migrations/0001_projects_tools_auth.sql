-- Projects, auth configs and tools. Nested IR structures are JSON text columns.

CREATE TABLE projects (
    id          TEXT PRIMARY KEY NOT NULL,
    name        TEXT NOT NULL,
    slug        TEXT NOT NULL UNIQUE,
    created_at  TEXT NOT NULL,
    updated_at  TEXT NOT NULL
);

CREATE TABLE auth_configs (
    id          TEXT PRIMARY KEY NOT NULL,
    project_id  TEXT NOT NULL REFERENCES projects (id) ON DELETE CASCADE,
    name        TEXT NOT NULL,
    kind        TEXT NOT NULL,              -- JSON AuthKind
    secret_ref  TEXT NOT NULL,              -- env:/keyring: reference, never a raw secret
    created_at  TEXT NOT NULL,
    updated_at  TEXT NOT NULL,
    UNIQUE (project_id, name)
);

CREATE TABLE tools (
    id              TEXT PRIMARY KEY NOT NULL,
    project_id      TEXT NOT NULL REFERENCES projects (id) ON DELETE CASCADE,
    name            TEXT NOT NULL,
    description     TEXT NOT NULL,
    kind            TEXT NOT NULL,          -- JSON ToolKind
    -- Copied from kind.auth_ref so the database enforces the reference.
    -- NO ACTION (checked at statement end) lets a project delete cascade cleanly.
    auth_ref        TEXT REFERENCES auth_configs (id) ON DELETE NO ACTION,
    enabled         INTEGER NOT NULL,
    source          TEXT NOT NULL,          -- JSON Source
    overrides       TEXT NOT NULL,          -- JSON Overrides
    response_shape  TEXT,
    created_at      TEXT NOT NULL,
    updated_at      TEXT NOT NULL,
    UNIQUE (project_id, name)
);

CREATE INDEX tools_project_id ON tools (project_id);
CREATE INDEX tools_auth_ref ON tools (auth_ref);
CREATE INDEX auth_configs_project_id ON auth_configs (project_id);
