use crate::models::{Job, Pipeline, Worker};
use crate::routes::{AnyhowError, AppState};

use anyhow::Context;
use axum::extract::{Json, Query, State};

use diesel::{Connection, ExpressionMethods, OptionalExtension, QueryDsl, RunQueryDsl};

use serde::{Deserialize, Serialize};

use teloxide::prelude::*;

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
        conn.transaction::<JobListResponse, diesel::result::Error, _>(|conn| {
            let total_items = crate::schema::jobs::dsl::jobs.count().get_result(conn)?;

            let jobs = if query.items_per_page == -1 {
                crate::schema::jobs::dsl::jobs
                    .order_by(crate::schema::jobs::dsl::id)
                    .load::<Job>(conn)?
            } else {
                crate::schema::jobs::dsl::jobs
                    .order_by(crate::schema::jobs::dsl::id)
                    .offset((query.page - 1) * query.items_per_page)
                    .limit(query.items_per_page)
                    .load::<Job>(conn)?
            };

            let mut items = vec![];
            for job in jobs {
                items.push(JobListResponseItem {
                    id: job.id,
                    pipeline_id: job.pipeline_id,
                    packages: job.packages,
                    arch: job.arch,
                    status: job.status,
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

    // from pipeline
    git_branch: String,
    git_sha: String,
    github_pr: Option<i64>,
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
            let job = crate::schema::jobs::dsl::jobs
                .find(query.job_id)
                .get_result::<Job>(conn)?;

            let pipeline = crate::schema::pipelines::dsl::pipelines
                .find(job.pipeline_id)
                .get_result::<Pipeline>(conn)?;

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
                // from pipeline
                git_branch: pipeline.git_branch,
                git_sha: pipeline.git_sha,
                github_pr: pipeline.github_pr,
            })
        })?,
    ))
}

#[derive(Deserialize)]
pub struct WorkerInfoRequest {
    worker_id: i32,
}

#[derive(Serialize)]
pub struct WorkerInfoResponse {
    // from worker
    worker_id: i32,
    hostname: String,
    arch: String,
    git_commit: String,
    memory_bytes: i64,
    logical_cores: i32,
    last_heartbeat_time: chrono::DateTime<chrono::Utc>,

    // status
    running_job_id: Option<i32>,

    // statistics
    built_job_count: i64,
}

pub async fn worker_info(
    Query(query): Query<WorkerInfoRequest>,
    State(AppState { pool, .. }): State<AppState>,
) -> Result<Json<WorkerInfoResponse>, AnyhowError> {
    let mut conn = pool
        .get()
        .context("Failed to get db connection from pool")?;

    Ok(Json(
        conn.transaction::<WorkerInfoResponse, diesel::result::Error, _>(|conn| {
            let worker = crate::schema::workers::dsl::workers
                .find(query.worker_id)
                .get_result::<Worker>(conn)?;

            let running_job = crate::schema::jobs::dsl::jobs
                .filter(crate::schema::jobs::dsl::assigned_worker_id.eq(worker.id))
                .first::<Job>(conn)
                .optional()?;

            let built_job_count = crate::schema::jobs::dsl::jobs
                .filter(crate::schema::jobs::dsl::built_by_worker_id.eq(worker.id))
                .count()
                .get_result::<i64>(conn)?;

            Ok(WorkerInfoResponse {
                worker_id: worker.id,
                hostname: worker.hostname,
                arch: worker.arch,
                git_commit: worker.git_commit,
                memory_bytes: worker.memory_bytes,
                logical_cores: worker.logical_cores,
                last_heartbeat_time: worker.last_heartbeat_time,

                running_job_id: running_job.map(|job| job.id),
                built_job_count,
            })
        })?,
    ))
}
