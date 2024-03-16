-- This file should undo anything in `up.sql`
UPDATE jobs SET status = 'finished' WHERE status = 'success';
UPDATE jobs SET status = 'finished' WHERE status = 'failed';
