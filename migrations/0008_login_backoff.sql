-- Stage 15: sign-in backoff survives restarts.

-- One row per backed-off key: `user:<name>` or `addr:<ip>`.
CREATE TABLE login_backoff (
    key              TEXT PRIMARY KEY NOT NULL,
    failures         INTEGER NOT NULL,
    last_failure_at  TEXT NOT NULL,
    locked_until     TEXT
);

CREATE INDEX login_backoff_last_failure ON login_backoff (last_failure_at);
