-- Stage 22: admins and members; members get read or write per project.

-- Existing accounts become admins so nobody is locked out.
ALTER TABLE users ADD COLUMN role TEXT NOT NULL DEFAULT 'admin';   -- admin | member

CREATE TABLE project_access (
    user_id     TEXT NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    project_id  TEXT NOT NULL REFERENCES projects (id) ON DELETE CASCADE,
    level       TEXT NOT NULL,                                     -- read | write
    PRIMARY KEY (user_id, project_id)
);

CREATE INDEX project_access_project ON project_access (project_id);
