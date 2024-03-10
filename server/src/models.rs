use diesel::prelude::*;

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

#[derive(Queryable, Selectable, Associations)]
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

#[derive(Queryable, Selectable)]
#[diesel(table_name = crate::schema::workers)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct Worker {
    pub id: i32,
    pub hostname: String,
    pub arch: String,
    pub git_commit: String,
    pub memory_bytes: i64,
    pub logical_cores: i32,
}
