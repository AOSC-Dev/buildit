-- Your SQL goes here
ALTER TABLE pipelines ADD COLUMN creator_user_id INT;
ALTER TABLE pipelines ADD CONSTRAINT creator_user FOREIGN KEY(creator_user_id) REFERENCES users(id);
