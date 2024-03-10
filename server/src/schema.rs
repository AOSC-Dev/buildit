// @generated automatically by Diesel CLI.

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
