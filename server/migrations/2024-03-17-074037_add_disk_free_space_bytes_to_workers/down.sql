-- This file should undo anything in `up.sql`
ALTER TABLE workers DROP COLUMN disk_free_space_bytes_to_workers;
