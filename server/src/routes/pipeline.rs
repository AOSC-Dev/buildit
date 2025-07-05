use crate::models::User;
use crate::routes::{AnyhowError, AppState};
use crate::{
    api::{self, JobSource, PipelineStatus},
    models::{Job, Pipeline},
};
use anyhow::Context;
use axum::extract::{Json, Query, State};
use diesel::{
    BelongingToDsl, Connection, ExpressionMethods, GroupedBy, QueryDsl, RunQueryDsl,
    SelectableHelper,
};
use serde::{Deserialize, Serialize};
use tracing::error;

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
    let (pipeline, _) = api::pipeline_new(
        pool,
        Some(&payload.git_branch),
        None,
        None,
        &payload.packages,
        &payload.archs,
        JobSource::Manual,
        false,
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
    let (pipeline, _) = api::pipeline_new_pr(
        pool,
        payload.pr,
        payload.archs.as_deref(),
        JobSource::Manual,
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
                .order(crate::schema::jobs::dsl::id.asc())
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
    stable_only: bool,
    github_pr_only: bool,
}

#[derive(Serialize)]
pub struct PipelineListResponseJob {
    job_id: i32,
    arch: String,
    status: String,
}

#[derive(Serialize)]
pub struct PipelineListResponseItem {
    id: i32,
    git_branch: String,
    git_sha: String,
    creation_time: chrono::DateTime<chrono::Utc>,
    github_pr: Option<i64>,
    packages: String,
    archs: String,
    status: &'static str,

    // from pipeline creator
    creator_github_login: Option<String>,
    creator_github_avatar_url: Option<String>,

    jobs: Vec<PipelineListResponseJob>,
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
            // compute total items for pagination
            let mut total_items_query = crate::schema::pipelines::dsl::pipelines.into_boxed();

            if query.stable_only {
                total_items_query = total_items_query
                    .filter(crate::schema::pipelines::dsl::git_branch.eq("stable"));
            }
            if query.github_pr_only {
                total_items_query = total_items_query
                    .filter(crate::schema::pipelines::dsl::github_pr.is_not_null());
            }

            let total_items = total_items_query.count().get_result(conn)?;

            // collect pipelines
            let mut sql = crate::schema::pipelines::dsl::pipelines
                .left_join(crate::schema::users::dsl::users)
                .order_by(crate::schema::pipelines::dsl::id.desc())
                .into_boxed();

            if query.stable_only {
                sql = sql.filter(crate::schema::pipelines::dsl::git_branch.eq("stable"));
            }
            if query.github_pr_only {
                sql = sql.filter(crate::schema::pipelines::dsl::github_pr.is_not_null());
            }

            let res: Vec<(Pipeline, Option<User>)> = if query.items_per_page == -1 {
                sql.load::<(Pipeline, Option<User>)>(conn)?
            } else {
                sql.offset((query.page - 1) * query.items_per_page)
                    .limit(query.items_per_page)
                    .load::<(Pipeline, Option<User>)>(conn)?
            };
            let (pipelines, users): (Vec<Pipeline>, Vec<Option<User>>) = res.into_iter().unzip();

            // get all jobs of these pipelines
            // and group by pipeline later
            // see https://diesel.rs/guides/relations.html
            let jobs = Job::belonging_to(&pipelines)
                .select(Job::as_select())
                .order(crate::schema::jobs::dsl::id.desc())
                .load(conn)?;

            let mut items = vec![];
            for ((mut jobs, pipeline), creator) in jobs
                .grouped_by(&pipelines)
                .into_iter()
                .zip(pipelines)
                .zip(users)
            {
                // Mimic gitlab behavior: for each arch, only keep the latest
                // (with maximum id) job. The maximum id is listed first via
                // `.order(crate::schema::jobs::dsl::id.desc())`. Then
                // `dedup_by` removes all but the first of consecutive elements.
                jobs.sort_by(|a, b| a.arch.cmp(&b.arch));
                jobs.dedup_by(|a, b| a.arch.eq(&b.arch));

                let mut has_error = false;
                let mut has_failed = false;
                let mut has_unfinished = false;
                for job in &jobs {
                    match job.status.as_str() {
                        "error" => has_error = true,
                        "success" => {
                            // success
                        }
                        "failed" => {
                            // failed
                            has_failed = true;
                        }
                        "created" => {
                            has_unfinished = true;
                        }
                        "running" => {
                            has_unfinished = true;
                        }
                        _ => {
                            error!("Got job with unknown status: {:?}", job);
                        }
                    }
                }

                let status = if has_error {
                    "error"
                } else if has_failed {
                    "failed"
                } else if has_unfinished {
                    "running"
                } else {
                    "success"
                };

                // compute pipeline status based on job status
                items.push(PipelineListResponseItem {
                    id: pipeline.id,
                    git_branch: pipeline.git_branch,
                    git_sha: pipeline.git_sha,
                    packages: pipeline.packages,
                    archs: pipeline.archs,
                    creation_time: pipeline.creation_time,
                    github_pr: pipeline.github_pr,
                    status,

                    creator_github_login: creator
                        .as_ref()
                        .and_then(|user| user.github_login.as_ref())
                        .cloned(),
                    creator_github_avatar_url: creator
                        .as_ref()
                        .and_then(|user| user.github_avatar_url.as_ref())
                        .cloned(),
                    jobs: jobs
                        .into_iter()
                        .map(|job| PipelineListResponseJob {
                            job_id: job.id,
                            arch: job.arch,
                            status: job.status,
                        })
                        .collect(),
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
