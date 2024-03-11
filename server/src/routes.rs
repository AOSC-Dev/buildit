use crate::{
    api::{self, PipelineStatus},
    models::{Job, NewWorker, Pipeline, Worker},
    DbPool,
};
use anyhow::Context;
use axum::{
    extract::{Json, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use diesel::{Connection, ExpressionMethods, OptionalExtension, QueryDsl, RunQueryDsl};
use serde::{Deserialize, Serialize};

pub async fn ping() -> &'static str {
    "PONG"
}

// learned from https://github.com/tokio-rs/axum/blob/main/examples/anyhow-error-response/src/main.rs
pub struct AnyhowError(anyhow::Error);

impl IntoResponse for AnyhowError {
    fn into_response(self) -> Response {
        (StatusCode::INTERNAL_SERVER_ERROR, format!("{}", self.0)).into_response()
    }
}

impl<E> From<E> for AnyhowError
where
    E: Into<anyhow::Error>,
{
    fn from(err: E) -> Self {
        Self(err.into())
    }
}

#[derive(Deserialize)]
pub struct PipelineNewRequest {
    git_branch: String,
    packages: String,
    archs: String,
}

#[derive(Serialize)]
pub struct PipelineNewResponse {
    id: i32,
}

pub async fn pipeline_new(
    State(pool): State<DbPool>,
    Json(payload): Json<PipelineNewRequest>,
) -> Result<Json<PipelineNewResponse>, AnyhowError> {
    let pipeline = api::pipeline_new(
        pool,
        &payload.git_branch,
        None,
        None,
        &payload.packages,
        &payload.archs,
        &common::JobSource::Manual,
    )
    .await?;
    Ok(Json(PipelineNewResponse { id: pipeline.id }))
}

#[derive(Deserialize)]
pub struct PipelineNewPRRequest {
    pr: u64,
    archs: Option<String>,
}

pub async fn pipeline_new_pr(
    State(pool): State<DbPool>,
    Json(payload): Json<PipelineNewPRRequest>,
) -> Result<Json<PipelineNewResponse>, AnyhowError> {
    let pipeline =
        api::pipeline_new_pr(pool, payload.pr, payload.archs.as_ref().map(|s| s.as_str())).await?;
    Ok(Json(PipelineNewResponse { id: pipeline.id }))
}

#[derive(Deserialize)]
pub struct WorkerHeartbeatRequest {
    hostname: String,
    arch: String,
    git_commit: String,
    memory_bytes: i64,
    logical_cores: i32,
}

pub async fn worker_heartbeat(
    State(pool): State<DbPool>,
    Json(payload): Json<WorkerHeartbeatRequest>,
) -> Result<(), AnyhowError> {
    // insert or update worker
    let new_worker = NewWorker {
        hostname: payload.hostname.clone(),
        arch: payload.arch.clone(),
        git_commit: payload.git_commit.clone(),
        memory_bytes: payload.memory_bytes,
        logical_cores: payload.logical_cores,
        last_heartbeat_time: chrono::Utc::now(),
    };

    let mut conn = pool
        .get()
        .context("Failed to get db connection from pool")?;
    diesel::insert_into(crate::schema::workers::table)
        .values(&new_worker)
        .on_conflict((
            crate::schema::workers::hostname,
            crate::schema::workers::arch,
        ))
        .do_update()
        .set(&new_worker)
        .execute(&mut conn)?;
    Ok(())
}

#[derive(Deserialize)]
pub struct WorkerPollRequest {
    hostname: String,
    arch: String,
}

#[derive(Serialize)]
pub struct WorkerPollResponse {
    pipeline_id: i32,
    job_id: i32,
    git_branch: String,
    git_sha: String,
    packages: String,
}

pub async fn worker_poll(
    State(pool): State<DbPool>,
    Json(payload): Json<WorkerPollRequest>,
) -> Result<Json<Option<WorkerPollResponse>>, AnyhowError> {
    // find a job that can be assigned to the worker
    let mut conn = pool
        .get()
        .context("Failed to get db connection from pool")?;

    match conn.transaction::<Option<(Pipeline, Job)>, diesel::result::Error, _>(|conn| {
        use crate::schema::jobs::dsl::*;
        match jobs
            .filter(status.eq("created"))
            .filter(arch.eq(&payload.arch))
            .first::<Job>(conn)
            .optional()?
        {
            Some(job) => {
                // find worker id
                let worker = crate::schema::workers::dsl::workers
                    .filter(crate::schema::workers::dsl::hostname.eq(&payload.hostname))
                    .filter(crate::schema::workers::dsl::arch.eq(&payload.arch))
                    .first::<Worker>(conn)?;

                // remove if already allocated to the worker
                diesel::update(jobs.filter(assigned_worker_id.eq(worker.id)))
                    .set((status.eq("created"), assigned_worker_id.eq(None::<i32>)))
                    .execute(conn)?;

                // allocate to the worker
                diesel::update(&job)
                    .set((status.eq("assigned"), assigned_worker_id.eq(worker.id)))
                    .execute(conn)?;

                // get pipeline the job belongs to
                let pipeline = crate::schema::pipelines::dsl::pipelines
                    .find(job.pipeline_id)
                    .get_result::<Pipeline>(conn)?;

                Ok(Some((pipeline, job)))
            }
            None => Ok(None),
        }
    })? {
        Some((pipeline, job)) => {
            // job allocated
            Ok(Json(Some(WorkerPollResponse {
                pipeline_id: job.pipeline_id,
                job_id: job.id,
                git_branch: pipeline.git_branch,
                git_sha: pipeline.git_sha,
                packages: job.packages,
            })))
        }
        None => Ok(Json(None)),
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum JobResult {
    Ok(JobOk),
    Error(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobOk {
    /// Is the build successful?
    pub build_success: bool,
    /// List of packages successfully built
    pub successful_packages: Vec<String>,
    /// List of packages failed to build
    pub failed_package: Option<String>,
    /// List of packages skipped
    pub skipped_packages: Vec<String>,
    /// URL to build log
    pub log_url: Option<String>,
    /// Elapsed time of the job
    pub elapsed_secs: i64,
    /// If pushpkg succeeded
    pub pushpkg_success: bool,
}

#[derive(Deserialize)]
pub struct WorkerJobUpdateRequest {
    hostname: String,
    arch: String,
    job_id: i32,
    result: JobResult,
}

pub async fn worker_job_update(
    State(pool): State<DbPool>,
    Json(payload): Json<WorkerJobUpdateRequest>,
) -> Result<(), AnyhowError> {
    let mut conn = pool
        .get()
        .context("Failed to get db connection from pool")?;

    use crate::schema::jobs::dsl::*;
    match payload.result {
        JobResult::Ok(res) => {
            diesel::update(jobs.filter(id.eq(payload.job_id)))
                .set((
                    status.eq("finished"),
                    build_success.eq(res.build_success),
                    pushpkg_success.eq(res.pushpkg_success),
                    successful_packages.eq(res.successful_packages.join(",")),
                    failed_package.eq(res.failed_package),
                    skipped_packages.eq(res.skipped_packages.join(",")),
                    log_url.eq(res.log_url),
                    finish_time.eq(chrono::Utc::now()),
                    elapsed_secs.eq(res.elapsed_secs),
                    assigned_worker_id.eq(None::<i32>),
                ))
                .execute(&mut conn)?;
        }
        JobResult::Error(err) => {
            diesel::update(jobs.filter(id.eq(payload.job_id)))
                .set((status.eq("error"), error_message.eq(err)))
                .execute(&mut conn)?;
        }
    }
    Ok(())
}

pub async fn pipeline_status(
    State(pool): State<DbPool>,
) -> Result<Json<Vec<PipelineStatus>>, AnyhowError> {
    Ok(Json(api::pipeline_status(pool).await?))
}

pub async fn worker_status(State(pool): State<DbPool>) -> Result<Json<Vec<Worker>>, AnyhowError> {
    Ok(Json(api::worker_status(pool).await?))
}
