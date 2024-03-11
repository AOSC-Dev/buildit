-- Your SQL goes here
CREATE TABLE pipelines (
  id SERIAL PRIMARY KEY,
  packages TEXT NOT NULL,
  archs TEXT NOT NULL,
  git_branch TEXT NOT NULL,
  git_sha TEXT NOT NULL,
  creation_time TIMESTAMP WITH TIME ZONE NOT NULL
);