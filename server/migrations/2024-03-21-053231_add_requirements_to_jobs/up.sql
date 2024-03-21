-- Your SQL goes here
ALTER TABLE jobs ADD COLUMN require_min_core INT;
ALTER TABLE jobs ADD COLUMN require_min_total_mem BIGINT;
ALTER TABLE jobs ADD COLUMN require_min_total_mem_per_core REAL;
ALTER TABLE jobs ADD COLUMN require_min_disk BIGINT;
