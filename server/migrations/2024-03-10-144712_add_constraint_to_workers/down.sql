-- This file should undo anything in `up.sql`
ALTER TABLE workers DROP CONSTRAINT unique_hostname_arch;
