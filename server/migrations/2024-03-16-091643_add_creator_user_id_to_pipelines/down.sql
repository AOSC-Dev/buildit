-- This file should undo anything in `up.sql`
ALTER TABLE pipelines DROP CONSTRAINT creator_user;
ALTER TABLE pipelines DROP COLUMN creator_user_id;
