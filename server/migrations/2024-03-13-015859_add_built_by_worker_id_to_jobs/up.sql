-- Your SQL goes here
ALTER TABLE jobs ADD COLUMN built_by_worker_id INT;
ALTER TABLE jobs ADD CONSTRAINT built_by_worker FOREIGN KEY(built_by_worker_id) REFERENCES workers(id);
