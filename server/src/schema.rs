// @generated automatically by Diesel CLI.

diesel::table! {
    jobs (id) {
        id -> Int4,
        pipeline_id -> Int4,
        packages -> Text,
        arch -> Text,
        creation_time -> Timestamptz,
        status -> Text,
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
    }
}

diesel::joinable!(jobs -> pipelines (pipeline_id));

diesel::allow_tables_to_appear_in_same_query!(jobs, pipelines, workers,);
