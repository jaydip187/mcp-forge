-- Stage 13: accounts for the web UI and REST API.

CREATE TABLE users (
    id                   TEXT PRIMARY KEY NOT NULL,
    username             TEXT NOT NULL UNIQUE COLLATE NOCASE,
    password_hash        TEXT NOT NULL,   -- argon2id PHC string
    created_at           TEXT NOT NULL,
    password_changed_at  TEXT NOT NULL
);

-- The cookie holds a random token; only its SHA-256 is stored, so a copy of
-- the database can't be used to sign in.
CREATE TABLE sessions (
    token_hash    TEXT PRIMARY KEY NOT NULL,
    user_id       TEXT NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    created_at    TEXT NOT NULL,
    last_seen_at  TEXT NOT NULL
);

CREATE INDEX sessions_user ON sessions (user_id);
