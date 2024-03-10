-- Your SQL goes here
ALTER TABLE jobs ADD build_success BOOLEAN;
ALTER TABLE jobs ADD pushpkg_success BOOLEAN;
ALTER TABLE jobs ADD successful_packages TEXT;
ALTER TABLE jobs ADD failed_package TEXT;
ALTER TABLE jobs ADD skipped_packages TEXT;
ALTER TABLE jobs ADD log_url TEXT;
ALTER TABLE jobs ADD finish_time TIMESTAMP WITH TIME ZONE;
