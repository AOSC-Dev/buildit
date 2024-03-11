use diesel::prelude::*;
use serde::Serialize;

#[derive(Queryable, Selectable)]
#[diesel(table_name = crate::schema::pipelines)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct Pipeline {
    pub id: i32,
    pub packages: String,
    pub archs: String,
    pub git_branch: String,
    pub git_sha: String,
    pub creation_time: chrono::DateTime<chrono::Utc>,
    pub source: String,
    pub github_pr: Option<i64>,
    pub telegram_user: Option<i64>,
}

#[derive(Insertable)]
#[diesel(table_name = crate::schema::pipelines)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct NewPipeline {
    pub packages: String,
    pub archs: String,
    pub git_branch: String,
    pub git_sha: String,
    pub creation_time: chrono::DateTime<chrono::Utc>,
    pub source: String,
    pub github_pr: Option<i64>,
    pub telegram_user: Option<i64>,
}

#[derive(Queryable, Selectable, Associations, Identifiable, Debug)]
#[diesel(belongs_to(Pipeline))]
#[diesel(table_name = crate::schema::jobs)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct Job {
    pub id: i32,
    pub pipeline_id: i32,
    pub packages: String,
    pub arch: String,
    pub creation_time: chrono::DateTime<chrono::Utc>,
    pub status: String,
    pub github_check_run_id: Option<i64>,
    pub build_success: Option<bool>,
    pub pushpkg_success: Option<bool>,
    pub successful_packages: Option<String>,
    pub failed_package: Option<String>,
    pub skipped_packages: Option<String>,
    pub log_url: Option<String>,
    pub finish_time: Option<chrono::DateTime<chrono::Utc>>,
    pub error_message: Option<String>,
    pub elapsed_secs: Option<i64>,
    pub assigned_worker_id: Option<i32>,
}

#[derive(Insertable)]
#[diesel(table_name = crate::schema::jobs)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct NewJob {
    pub pipeline_id: i32,
    pub packages: String,
    pub arch: String,
    pub creation_time: chrono::DateTime<chrono::Utc>,
    pub status: String,
    pub github_check_run_id: Option<i64>,
}

#[derive(Queryable, Selectable, Serialize, Debug)]
#[diesel(table_name = crate::schema::workers)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct Worker {
    pub id: i32,
    pub hostname: String,
    pub arch: String,
    pub git_commit: String,
    pub memory_bytes: i64,
    pub logical_cores: i32,
    pub last_heartbeat_time: chrono::DateTime<chrono::Utc>,
}

#[derive(Insertable, AsChangeset)]
#[diesel(table_name = crate::schema::workers)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct NewWorker {
    pub hostname: String,
    pub arch: String,
    pub git_commit: String,
    pub memory_bytes: i64,
    pub logical_cores: i32,
    pub last_heartbeat_time: chrono::DateTime<chrono::Utc>,
}
