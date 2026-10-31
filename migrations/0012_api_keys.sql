-- Stage 20: per-project access keys for /mcp.

CREATE TABLE api_keys (
    id            TEXT PRIMARY KEY NOT NULL,
    project_id    TEXT NOT NULL REFERENCES projects (id) ON DELETE CASCADE,
    name          TEXT NOT NULL,
    prefix        TEXT NOT NULL,          -- first characters, to recognise a key
    key_hash      TEXT NOT NULL UNIQUE,   -- SHA-256 of the whole key; the key itself is never stored
    tools         TEXT,                   -- JSON array of allowed tool names; NULL = all tools
    expires_at    TEXT,
    created_at    TEXT NOT NULL,
    created_by    TEXT,
    last_used_at  TEXT,
    UNIQUE (project_id, name)
);

CREATE INDEX api_keys_project ON api_keys (project_id);

-- Which key a call came in with.
ALTER TABLE call_log ADD COLUMN api_key TEXT;
