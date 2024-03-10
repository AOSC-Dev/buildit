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

diesel::allow_tables_to_appear_in_same_query!(jobs, pipelines, workers,);
