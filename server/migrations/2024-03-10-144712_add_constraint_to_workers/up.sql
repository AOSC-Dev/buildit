-- Your SQL goes here
ALTER TABLE workers ADD CONSTRAINT unique_hostname_arch UNIQUE (hostname, arch);