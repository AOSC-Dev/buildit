-- This file should undo anything in `up.sql`
ALTER TABLE jobs DROP COLUMN require_min_core;
ALTER TABLE jobs DROP COLUMN require_min_total_mem;
ALTER TABLE jobs DROP COLUMN require_min_total_mem_per_core;
ALTER TABLE jobs DROP COLUMN require_min_disk;
