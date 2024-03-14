use crate::github::get_crab_github_installation;
use crate::models::{Job, NewJob, Pipeline};
use crate::routes::{AnyhowError, AppState};
use anyhow::{bail, Context};
use axum::extract::{Json, Query, State};
use diesel::{Connection, ExpressionMethods, QueryDsl, RunQueryDsl};
use octocrab::models::checks::CheckRunOutput;
use octocrab::models::CheckRunId;
use serde::{Deserialize, Serialize};
use tokio::runtime::Handle;
use tracing::{error, warn};

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
    git_branch: String,
    git_sha: String,
    github_pr: Option<i64>,
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
                .order(crate::schema::jobs::dsl::id.desc());

            // all
            let res = if query.items_per_page == -1 {
                sql.load::<(Job, Pipeline)>(conn)?
            } else {
                sql.offset((query.page - 1) * query.items_per_page)
                    .limit(query.items_per_page)
                    .load::<(Job, Pipeline)>(conn)?
            };

            let mut items = vec![];
            for (job, pipeline) in res {
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
    let mut conn = pool
        .get()
        .context("Failed to get db connection from pool")?;

    let new_job = conn.transaction::<Job, anyhow::Error, _>(|conn| {
        let job = crate::schema::jobs::dsl::jobs
            .find(payload.job_id)
            .get_result::<Job>(conn)?;
        let pipeline = crate::schema::pipelines::dsl::pipelines
            .find(job.pipeline_id)
            .get_result::<Pipeline>(conn)?;

        // job must be finished
        if job.status != "finished" {
            bail!("Cannot restart the job unless it is finished");
        }

        // create a new job
        use crate::schema::jobs;
        let mut new_job = NewJob {
            pipeline_id: job.pipeline_id,
            packages: job.packages,
            arch: job.arch.clone(),
            creation_time: chrono::Utc::now(),
            status: "created".to_string(),
            github_check_run_id: None,
        };

        // create new github check run if the restarted job has one
        if job.github_check_run_id.is_some() {
            new_job.github_check_run_id = Handle::current().block_on(async {
                // authenticate with github app
                match get_crab_github_installation().await {
                    Ok(Some(crab)) => {
                        match crab
                            .checks("AOSC-Dev", "aosc-os-abbs")
                            .create_check_run(format!("buildit {}", job.arch), &pipeline.git_sha)
                            .status(octocrab::params::checks::CheckRunStatus::Queued)
                            .send()
                            .await
                        {
                            Ok(check_run) => {
                                return Some(check_run.id.0 as i64);
                            }
                            Err(err) => {
                                warn!("Failed to create check run: {}", err);
                            }
                        }
                    }
                    Ok(None) => {
                        // github app unavailable
                    }
                    Err(err) => {
                        warn!("Failed to get installation token: {}", err);
                    }
                }
                return None;
            });
        }

        let new_job: Job = diesel::insert_into(jobs::table)
            .values(&new_job)
            .get_result(conn)
            .context("Failed to create job")?;

        Ok(new_job)
    })?;

    Ok(Json(JobRestartResponse { job_id: new_job.id }))
}
