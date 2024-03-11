-- Your SQL goes here
ALTER TABLE jobs ADD COLUMN assigned_worker_id INT;
ALTER TABLE jobs ADD CONSTRAINT assigned_worker FOREIGN KEY(assigned_worker_id) REFERENCES workers(id);
