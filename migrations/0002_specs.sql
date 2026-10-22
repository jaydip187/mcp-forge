-- Raw imported API descriptions, kept for re-import (Stage 6).

CREATE TABLE specs (
    id          TEXT PRIMARY KEY NOT NULL,
    project_id  TEXT NOT NULL REFERENCES projects (id) ON DELETE CASCADE,
    kind        TEXT NOT NULL,              -- 'openapi3' (Stage 5 adds 'swagger2')
    title       TEXT NOT NULL,
    version     TEXT NOT NULL,
    source_url  TEXT,                       -- set when fetched from a URL
    filename    TEXT,                       -- set when uploaded
    raw         TEXT NOT NULL,
    created_at  TEXT NOT NULL
);

CREATE INDEX specs_project_id ON specs (project_id);
