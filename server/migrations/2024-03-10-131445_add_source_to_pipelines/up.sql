-- Your SQL goes here
ALTER TABLE pipelines ADD source TEXT NOT NULL;
ALTER TABLE pipelines ADD github_pr BIGINT;
ALTER TABLE pipelines ADD telegram_user BIGINT;