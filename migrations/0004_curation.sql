-- Stage 7: per-project curation settings and per-tool tags.
ALTER TABLE projects ADD COLUMN settings TEXT NOT NULL DEFAULT '{}';
ALTER TABLE tools ADD COLUMN tags TEXT NOT NULL DEFAULT '[]';
