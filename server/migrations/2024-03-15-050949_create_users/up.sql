-- Your SQL goes here
CREATE TABLE users (
  id SERIAL PRIMARY KEY,
  github_login TEXT,
  github_id BIGINT,
  github_name TEXT,
  github_avatar_url TEXT,
  github_email TEXT,
  telegram_chat_id BIGINT
);
