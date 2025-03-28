use crate::models::{Job, Pipeline, User, Worker};
use crate::routes::{AnyhowError, AppState};
use anyhow::Context;
use axum::extract::{Json, Query, State};
use diesel::{
    Connection, ExpressionMethods, JoinOnDsl, NullableExpressionMethods, QueryDsl, RunQueryDsl,
};
use serde::{Deserialize, Serialize};

#[derive(Deserialize)]
pub struct JobListRequest {
    page: i64,
    items_per_page: i64,
}

#[derive(Serialize)]
pub struct JobListResponseItem {
    id: i32,
    pipeline_id: i32,
    packages: String,
    arch: String,
    status: String,
    elapsed_secs: Option<i64>,
    creation_time: chrono::DateTime<chrono::Utc>,
    log_url: Option<String>,
    build_success: Option<bool>,

    // from pipeline
    git_branch: String,
    git_sha: String,
    github_pr: Option<i64>,

    // from pipeline creator
    creator_github_login: Option<String>,
    creator_github_avatar_url: Option<String>,
}

#[derive(Serialize)]
pub struct JobListResponse {
    total_items: i64,
    items: Vec<JobListResponseItem>,
}

pub async fn job_list(
    Query(query): Query<JobListRequest>,
    State(AppState { pool, .. }): State<AppState>,
) -> Result<Json<JobListResponse>, AnyhowError> {
    let mut conn = pool
        .get()
        .context("Failed to get db connection from pool")?;

    Ok(Json(
        conn.transaction::<JobListResponse, anyhow::Error, _>(|conn| {
            let total_items = crate::schema::jobs::dsl::jobs.count().get_result(conn)?;

            let sql = crate::schema::jobs::dsl::jobs
                .inner_join(crate::schema::pipelines::dsl::pipelines)
                .left_join(
                    crate::schema::users::dsl::users
                        .on(crate::schema::pipelines::dsl::creator_user_id
                            .eq(crate::schema::users::dsl::id.nullable())),
                )
                .order(crate::schema::jobs::dsl::id.desc());

            // all
            let res = if query.items_per_page == -1 {
                sql.load::<(Job, Pipeline, Option<User>)>(conn)?
            } else {
                sql.offset((query.page - 1) * query.items_per_page)
                    .limit(query.items_per_page)
                    .load::<(Job, Pipeline, Option<User>)>(conn)?
            };

            let mut items = vec![];
            for (job, pipeline, creator) in res {
                items.push(JobListResponseItem {
                    id: job.id,
                    pipeline_id: job.pipeline_id,
                    packages: job.packages,
                    arch: job.arch,
                    status: job.status,
                    elapsed_secs: job.elapsed_secs,
                    creation_time: job.creation_time,
                    log_url: job.log_url,
                    build_success: job.build_success,

                    git_branch: pipeline.git_branch,
                    git_sha: pipeline.git_sha,
                    github_pr: pipeline.github_pr,

                    creator_github_login: creator
                        .as_ref()
                        .and_then(|user| user.github_login.as_ref())
                        .cloned(),
                    creator_github_avatar_url: creator
                        .as_ref()
                        .and_then(|user| user.github_avatar_url.as_ref())
                        .cloned(),
                });
            }

            Ok(JobListResponse { total_items, items })
        })?,
    ))
}

#[derive(Deserialize)]
pub struct JobInfoRequest {
    job_id: i32,
}

#[derive(Serialize)]
pub struct JobInfoResponse {
    // from job
    job_id: i32,
    pipeline_id: i32,
    packages: String,
    arch: String,
    creation_time: chrono::DateTime<chrono::Utc>,
    status: String,
    build_success: Option<bool>,
    pushpkg_success: Option<bool>,
    successful_packages: Option<String>,
    failed_package: Option<String>,
    skipped_packages: Option<String>,
    log_url: Option<String>,
    finish_time: Option<chrono::DateTime<chrono::Utc>>,
    error_message: Option<String>,
    elapsed_secs: Option<i64>,
    assigned_worker_id: Option<i32>,
    built_by_worker_id: Option<i32>,
    require_min_core: Option<i32>,
    require_min_total_mem: Option<i64>,
    require_min_total_mem_per_core: Option<f32>,
    require_min_disk: Option<i64>,
    assign_time: Option<chrono::DateTime<chrono::Utc>>,

    // from pipeline
    git_branch: String,
    git_sha: String,
    github_pr: Option<i64>,

    // from worker
    assigned_worker_hostname: Option<String>,
    built_by_worker_hostname: Option<String>,
}

pub async fn job_info(
    Query(query): Query<JobInfoRequest>,
    State(AppState { pool, .. }): State<AppState>,
) -> Result<Json<JobInfoResponse>, AnyhowError> {
    let mut conn = pool
        .get()
        .context("Failed to get db connection from pool")?;

    Ok(Json(
        conn.transaction::<JobInfoResponse, diesel::result::Error, _>(|conn| {
            // use alias to allow joining workers table twice
            // https://github.com/diesel-rs/diesel/issues/2569
            // https://github.com/diesel-rs/diesel/pull/2254
            // https://docs.rs/diesel/latest/diesel/macro.alias.html
            let assigned_workers = diesel::alias!(crate::schema::workers as assigned_workers);
            let (job, pipeline, assigned_worker, built_by_worker) = crate::schema::jobs::dsl::jobs
                .find(query.job_id)
                .inner_join(crate::schema::pipelines::dsl::pipelines)
                .left_join(
                    assigned_workers.on(crate::schema::jobs::dsl::assigned_worker_id.eq(
                        assigned_workers
                            .field(crate::schema::workers::dsl::id)
                            .nullable(),
                    )),
                )
                .left_join(
                    crate::schema::workers::dsl::workers
                        .on(crate::schema::jobs::dsl::built_by_worker_id
                            .eq(crate::schema::workers::dsl::id.nullable())),
                )
                .get_result::<(Job, Pipeline, Option<Worker>, Option<Worker>)>(conn)?;

            Ok(JobInfoResponse {
                job_id: job.id,
                pipeline_id: job.pipeline_id,
                packages: job.packages,
                arch: job.arch,
                creation_time: job.creation_time,
                status: job.status,
                build_success: job.build_success,
                pushpkg_success: job.pushpkg_success,
                successful_packages: job.successful_packages,
                failed_package: job.failed_package,
                skipped_packages: job.skipped_packages,
                log_url: job.log_url,
                finish_time: job.finish_time,
                error_message: job.error_message,
                elapsed_secs: job.elapsed_secs,
                assigned_worker_id: job.assigned_worker_id,
                built_by_worker_id: job.built_by_worker_id,
                require_min_core: job.require_min_core,
                require_min_total_mem: job.require_min_total_mem,
                require_min_total_mem_per_core: job.require_min_total_mem_per_core,
                require_min_disk: job.require_min_disk,
                assign_time: job.assign_time,

                // from pipeline
                git_branch: pipeline.git_branch,
                git_sha: pipeline.git_sha,
                github_pr: pipeline.github_pr,

                // from worker
                assigned_worker_hostname: assigned_worker.map(|w| w.hostname),
                built_by_worker_hostname: built_by_worker.map(|w| w.hostname),
            })
        })?,
    ))
}

#[derive(Deserialize)]
pub struct JobRestartRequest {
    job_id: i32,
}

#[derive(Serialize)]
pub struct JobRestartResponse {
    job_id: i32,
}

pub async fn job_restart(
    State(AppState { pool, .. }): State<AppState>,
    Json(payload): Json<JobRestartRequest>,
) -> Result<Json<JobRestartResponse>, AnyhowError> {
    let new_job = crate::api::job_restart(pool, payload.job_id).await?;
    Ok(Json(JobRestartResponse { job_id: new_job.id }))
}
