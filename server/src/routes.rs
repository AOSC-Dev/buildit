use axum::{
    extract::{Json, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use diesel::{
    r2d2::{ConnectionManager, Pool},
    PgConnection,
};
use serde::{Deserialize, Serialize};

use crate::api;

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
    State(pool): State<Pool<ConnectionManager<PgConnection>>>,
    Json(payload): Json<PipelineNewRequest>,
) -> Result<Json<PipelineNewResponse>, AnyhowError> {
    let pipeline_id = api::pipeline_new(
        pool,
        &payload.git_branch,
        None,
        &payload.packages,
        &payload.archs,
        &common::JobSource::Manual,
    )
    .await?;
    Ok(Json(PipelineNewResponse { id: pipeline_id }))
}

#[derive(Deserialize)]
pub struct PipelineNewPRRequest {
    pr: u64,
    archs: Option<String>,
}

pub async fn pipeline_new_pr(
    State(pool): State<Pool<ConnectionManager<PgConnection>>>,
    Json(payload): Json<PipelineNewPRRequest>,
) -> Result<Json<PipelineNewResponse>, AnyhowError> {
    let pipeline_id =
        api::pipeline_new_pr(pool, payload.pr, payload.archs.as_ref().map(|s| s.as_str())).await?;
    Ok(Json(PipelineNewResponse { id: pipeline_id }))
}
