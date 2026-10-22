-- Remember the choices made at import (base URL, auth) so re-import can reuse them.
ALTER TABLE specs ADD COLUMN options TEXT NOT NULL DEFAULT '{}';
ALTER TABLE specs ADD COLUMN updated_at TEXT;
