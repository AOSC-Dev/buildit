-- This file should undo anything in `up.sql`
ALTER TABLE workers DROP COLUMN last_heartbeat_time;
