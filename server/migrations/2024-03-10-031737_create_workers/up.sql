-- Your SQL goes here
CREATE TABLE workers (
  id SERIAL PRIMARY KEY,
  hostname TEXT NOT NULL,
  arch TEXT NOT NULL,
  git_commit TEXT NOT NULL,
  memory_bytes BIGINT NOT NULL,
  logical_cores INT NOT NULL
);