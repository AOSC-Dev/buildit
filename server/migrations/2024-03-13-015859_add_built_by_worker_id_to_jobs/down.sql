-- This file should undo anything in `up.sql`
ALTER TABLE jobs DROP CONSTRAINT built_by_worker;
ALTER TABLE jobs DROP COLUMN built_by_worker_id;
