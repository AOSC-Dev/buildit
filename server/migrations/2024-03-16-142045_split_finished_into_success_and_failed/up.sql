-- Your SQL goes here
UPDATE jobs SET status = 'success' WHERE status = 'finished' AND build_success IS TRUE AND pushpkg_success IS TRUE;
UPDATE jobs SET status = 'failed' WHERE status = 'finished' AND (build_success IS NOT TRUE OR pushpkg_success IS NOT TRUE);
