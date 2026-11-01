-- Stage 27: calls made by a chained tool's steps point at the chain's call.
ALTER TABLE call_log ADD COLUMN parent_id TEXT;
