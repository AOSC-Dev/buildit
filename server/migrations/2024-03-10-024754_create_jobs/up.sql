-- Your SQL goes here
CREATE TABLE jobs (
  id SERIAL PRIMARY KEY,
  pipeline_id INT NOT NULL,
  packages TEXT NOT NULL,
  arch TEXT NOT NULL,
  creation_time TIMESTAMP WITH TIME ZONE NOT NULL,
  status TEXT NOT NULL,
  CONSTRAINT pipeline FOREIGN KEY(pipeline_id) REFERENCES pipelines(id)
);