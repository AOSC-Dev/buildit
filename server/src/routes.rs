use crate::{api, models::NewWorker, DbPool};
use anyhow::Context;
use axum::{
    extract::{Json, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use diesel::RunQueryDsl;
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
