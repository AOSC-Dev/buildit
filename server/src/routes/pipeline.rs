use crate::routes::{AnyhowError, AppState};
use crate::{
    api::{self, JobSource, PipelineStatus},
    models::{Job, Pipeline},
};

use anyhow::Context;
use axum::extract::{Json, Query, State};

use diesel::{Connection, ExpressionMethods, QueryDsl, RunQueryDsl};

use serde::{Deserialize, Serialize};

use teloxide::prelude::*;

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
    State(AppState { pool, .. }): State<AppState>,
    Json(payload): Json<PipelineNewRequest>,
) -> Result<Json<PipelineNewResponse>, AnyhowError> {
    let pipeline = api::pipeline_new(
        pool,
        &payload.git_branch,
        None,
        None,
        &payload.packages,
        &payload.archs,
        &JobSource::Manual,
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
    State(AppState { pool, .. }): State<AppState>,
    Json(payload): Json<PipelineNewPRRequest>,
) -> Result<Json<PipelineNewResponse>, AnyhowError> {
    let pipeline = api::pipeline_new_pr(
        pool,
        payload.pr,
        payload.archs.as_deref(),
        &JobSource::Manual,
    )
    .await?;
    Ok(Json(PipelineNewResponse { id: pipeline.id }))
}

#[derive(Deserialize)]
pub struct PipelineInfoRequest {
    pipeline_id: i32,
}

#[derive(Serialize)]
pub struct PipelineInfoResponseJob {
    job_id: i32,
    arch: String,
}

#[derive(Serialize)]
pub struct PipelineInfoResponse {
    // from pipeline
    pipeline_id: i32,
    packages: String,
    archs: String,
    git_branch: String,
    git_sha: String,
    creation_time: chrono::DateTime<chrono::Utc>,
    github_pr: Option<i64>,

    // related jobs
    jobs: Vec<PipelineInfoResponseJob>,
}

pub async fn pipeline_info(
    Query(query): Query<PipelineInfoRequest>,
    State(AppState { pool, .. }): State<AppState>,
) -> Result<Json<PipelineInfoResponse>, AnyhowError> {
    let mut conn = pool
        .get()
        .context("Failed to get db connection from pool")?;

    Ok(Json(
        conn.transaction::<PipelineInfoResponse, diesel::result::Error, _>(|conn| {
            let pipeline = crate::schema::pipelines::dsl::pipelines
                .find(query.pipeline_id)
                .get_result::<Pipeline>(conn)?;

            let jobs: Vec<PipelineInfoResponseJob> = crate::schema::jobs::dsl::jobs
                .filter(crate::schema::jobs::dsl::pipeline_id.eq(pipeline.id))
                .load::<Job>(conn)?
                .into_iter()
                .map(|job| PipelineInfoResponseJob {
                    job_id: job.id,
                    arch: job.arch,
                })
                .collect();

            Ok(PipelineInfoResponse {
                pipeline_id: pipeline.id,
                packages: pipeline.packages,
                archs: pipeline.archs,
                git_branch: pipeline.git_branch,
                git_sha: pipeline.git_sha,
                creation_time: pipeline.creation_time,
                github_pr: pipeline.github_pr,
                jobs,
            })
        })?,
    ))
}

#[derive(Deserialize)]
pub struct PipelineListRequest {
    page: i64,
    items_per_page: i64,
}

#[derive(Serialize)]
pub struct PipelineListResponseItem {
    id: i32,
    git_branch: String,
    packages: String,
    archs: String,
}

#[derive(Serialize)]
pub struct PipelineListResponse {
    total_items: i64,
    items: Vec<PipelineListResponseItem>,
}

pub async fn pipeline_list(
    Query(query): Query<PipelineListRequest>,
    State(AppState { pool, .. }): State<AppState>,
) -> Result<Json<PipelineListResponse>, AnyhowError> {
    let mut conn = pool
        .get()
        .context("Failed to get db connection from pool")?;

    Ok(Json(
        conn.transaction::<PipelineListResponse, diesel::result::Error, _>(|conn| {
            let total_items = crate::schema::pipelines::dsl::pipelines
                .count()
                .get_result(conn)?;

            let pipelines = if query.items_per_page == -1 {
                crate::schema::pipelines::dsl::pipelines
                    .order_by(crate::schema::pipelines::dsl::id)
                    .load::<Pipeline>(conn)?
            } else {
                crate::schema::pipelines::dsl::pipelines
                    .order_by(crate::schema::pipelines::dsl::id)
                    .offset((query.page - 1) * query.items_per_page)
                    .limit(query.items_per_page)
                    .load::<Pipeline>(conn)?
            };

            let mut items = vec![];
            for pipeline in pipelines {
                items.push(PipelineListResponseItem {
                    id: pipeline.id,
                    git_branch: pipeline.git_branch,
                    packages: pipeline.packages,
                    archs: pipeline.archs,
                });
            }

            Ok(PipelineListResponse { total_items, items })
        })?,
    ))
}

pub async fn pipeline_status(
    State(AppState { pool, .. }): State<AppState>,
) -> Result<Json<Vec<PipelineStatus>>, AnyhowError> {
    Ok(Json(api::pipeline_status(pool).await?))
}
