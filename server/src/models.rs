use diesel::prelude::*;
use serde::Serialize;

#[derive(Queryable, Selectable, Identifiable, Debug)]
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
    pub creator_user_id: Option<i32>,
    pub options: Option<String>,
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
    pub creator_user_id: Option<i32>,
    pub options: Option<String>,
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
    pub built_by_worker_id: Option<i32>,
    pub require_min_core: Option<i32>,
    pub require_min_total_mem: Option<i64>,
    pub require_min_total_mem_per_core: Option<f32>,
    pub require_min_disk: Option<i64>,
    pub assign_time: Option<chrono::DateTime<chrono::Utc>>,
    pub options: Option<String>,
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
    pub require_min_core: Option<i32>,
    pub require_min_total_mem: Option<i64>,
    pub require_min_total_mem_per_core: Option<f32>,
    pub require_min_disk: Option<i64>,
    pub options: Option<String>,
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
    pub disk_free_space_bytes: i64,
    pub performance: Option<i64>,
    pub visible: bool,
    pub internet_connectivity: bool,
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
    pub disk_free_space_bytes: i64,
    pub performance: Option<i64>,
    pub internet_connectivity: bool,
}

#[derive(Queryable, Selectable)]
#[diesel(table_name = crate::schema::users)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct User {
    pub id: i32,
    pub github_login: Option<String>,
    pub github_id: Option<i64>,
    pub github_name: Option<String>,
    pub github_avatar_url: Option<String>,
    pub github_email: Option<String>,
    pub telegram_chat_id: Option<i64>,
}

#[derive(Insertable, AsChangeset)]
#[diesel(table_name = crate::schema::users)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct NewUser {
    pub github_login: Option<String>,
    pub github_id: Option<i64>,
    pub github_name: Option<String>,
    pub github_avatar_url: Option<String>,
    pub github_email: Option<String>,
    pub telegram_chat_id: Option<i64>,
}
