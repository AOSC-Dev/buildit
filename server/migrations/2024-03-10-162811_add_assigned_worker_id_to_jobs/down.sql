-- This file should undo anything in `up.sql`
ALTER TABLE jobs DROP CONSTRAINT assigned_worker;
ALTER TABLE jobs DROP COLUMN assigned_worker_id;
