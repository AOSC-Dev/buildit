-- This file should undo anything in `up.sql`
UPDATE jobs SET status = 'assigned' WHERE status = 'running';
