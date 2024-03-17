-- Your SQL goes here
ALTER TABLE workers ADD COLUMN disk_free_space_bytes BIGINT;
UPDATE workers SET disk_free_space_bytes = 0;
ALTER TABLE workers ALTER COLUMN disk_free_space_bytes SET NOT NULL;
