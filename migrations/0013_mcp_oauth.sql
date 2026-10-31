-- Stage 21: mcp-forge as the OAuth 2.1 authorization server for /mcp.

-- Clients that registered themselves (RFC 7591), e.g. claude.ai connectors.
CREATE TABLE oauth_clients (
    client_id      TEXT PRIMARY KEY NOT NULL,
    secret_hash    TEXT,                -- SHA-256; NULL for public clients
    name           TEXT NOT NULL,
    redirect_uris  TEXT NOT NULL,       -- JSON array, matched exactly
    auth_method    TEXT NOT NULL,       -- none | client_secret_post | client_secret_basic
    created_at     TEXT NOT NULL
);

-- One row per approved connection: a client, a project, the person who
-- approved it. Tokens are stored as SHA-256 only.
CREATE TABLE oauth_grants (
    id                  TEXT PRIMARY KEY NOT NULL,
    client_id           TEXT NOT NULL REFERENCES oauth_clients (client_id) ON DELETE CASCADE,
    project_id          TEXT NOT NULL REFERENCES projects (id) ON DELETE CASCADE,
    approved_by         TEXT,
    access_hash         TEXT NOT NULL UNIQUE,
    access_expires_at   TEXT NOT NULL,
    refresh_hash        TEXT NOT NULL UNIQUE,
    refresh_expires_at  TEXT NOT NULL,
    created_at          TEXT NOT NULL,
    last_used_at        TEXT
);

CREATE INDEX oauth_grants_project ON oauth_grants (project_id);
