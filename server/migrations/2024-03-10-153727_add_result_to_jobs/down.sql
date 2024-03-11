-- This file should undo anything in `up.sql`
ALTER TABLE jobs DROP COLUMN build_success;
ALTER TABLE jobs DROP COLUMN pushpkg_success;
ALTER TABLE jobs DROP COLUMN successful_packages;
ALTER TABLE jobs DROP COLUMN failed_package;
ALTER TABLE jobs DROP COLUMN skipped_packages;
ALTER TABLE jobs DROP COLUMN log_url;
ALTER TABLE jobs DROP COLUMN finish_time;
