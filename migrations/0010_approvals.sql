-- Stage 17: tools can require a person's approval before agents run them.

ALTER TABLE tools ADD COLUMN require_approval INTEGER NOT NULL DEFAULT 0;

-- How the approval step of a call ended (NULL when none was needed).
ALTER TABLE call_log ADD COLUMN approval TEXT;          -- approved | denied | timed_out | cancelled
ALTER TABLE call_log ADD COLUMN approved_by TEXT;
ALTER TABLE call_log ADD COLUMN approval_wait_ms INTEGER;
