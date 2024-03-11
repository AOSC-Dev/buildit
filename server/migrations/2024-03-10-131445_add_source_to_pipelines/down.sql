-- This file should undo anything in `up.sql`
ALTER TABLE pipelines DROP COLUMN source;
ALTER TABLE pipelines DROP COLUMN github_pr;
ALTER TABLE pipelines DROP COLUMN telegram_user;
