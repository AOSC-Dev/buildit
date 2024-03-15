// @generated automatically by Diesel CLI.

diesel::table! {
    jobs (id) {
        id -> Int4,
        pipeline_id -> Int4,
        packages -> Text,
        arch -> Text,
        creation_time -> Timestamptz,
        status -> Text,
        github_check_run_id -> Nullable<Int8>,
        build_success -> Nullable<Bool>,
        pushpkg_success -> Nullable<Bool>,
        successful_packages -> Nullable<Text>,
        failed_package -> Nullable<Text>,
        skipped_packages -> Nullable<Text>,
        log_url -> Nullable<Text>,
        finish_time -> Nullable<Timestamptz>,
        error_message -> Nullable<Text>,
        elapsed_secs -> Nullable<Int8>,
        assigned_worker_id -> Nullable<Int4>,
        built_by_worker_id -> Nullable<Int4>,
    }
}

diesel::table! {
    pipelines (id) {
        id -> Int4,
        packages -> Text,
        archs -> Text,
        git_branch -> Text,
        git_sha -> Text,
        creation_time -> Timestamptz,
        source -> Text,
        github_pr -> Nullable<Int8>,
        telegram_user -> Nullable<Int8>,
    }
}

diesel::table! {
    users (id) {
        id -> Int4,
        github_login -> Nullable<Text>,
        github_id -> Nullable<Int8>,
        github_name -> Nullable<Text>,
        github_avatar_url -> Nullable<Text>,
        github_email -> Nullable<Text>,
        telegram_chat_id -> Nullable<Int8>,
    }
}

diesel::table! {
    workers (id) {
        id -> Int4,
        hostname -> Text,
        arch -> Text,
        git_commit -> Text,
        memory_bytes -> Int8,
        logical_cores -> Int4,
        last_heartbeat_time -> Timestamptz,
    }
}

diesel::joinable!(jobs -> pipelines (pipeline_id));

diesel::allow_tables_to_appear_in_same_query!(jobs, pipelines, users, workers,);
