-- Stage 19: tokens from the OAuth2 authorization code flow, one row per
-- auth config. Token columns are encrypted (see crates/store/src/cipher.rs).

CREATE TABLE oauth_tokens (
    auth_config_id  TEXT PRIMARY KEY NOT NULL REFERENCES auth_configs (id) ON DELETE CASCADE,
    access_token    TEXT NOT NULL,
    refresh_token   TEXT,
    expires_at      TEXT,
    scope           TEXT,
    connected_at    TEXT NOT NULL,
    connected_by    TEXT,
    updated_at      TEXT NOT NULL
);
