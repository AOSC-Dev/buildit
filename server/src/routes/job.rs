use crate::models::{Job, Pipeline};
use crate::routes::{AnyhowError, AppState};
use crate::schema::jobs::BoxedQuery;
use anyhow::{bail, Context};
use axum::extract::{Json, Query, State};
use diesel::pg::Pg;
use diesel::query_builder::QueryFragment;
use diesel::{AppearsOnTable, Connection, ExpressionMethods, QueryDsl, RunQueryDsl};
use serde::{Deserialize, Serialize};

#[derive(Deserialize)]
pub struct JobListRequest {
    page: i64,
    items_per_page: i64,
    sort_key: Option<String>,
    sort_order: Option<String>,
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

// https://stackoverflow.com/questions/59291037/how-do-i-conditionally-order-by-a-column-based-on-a-dynamic-parameter-with-diese
fn sort_by_column<U: 'static>(
    query: BoxedQuery<'static, Pg>,
    column: U,
    sort_order: &str,
) -> anyhow::Result<BoxedQuery<'static, Pg>>
where
    U: ExpressionMethods + QueryFragment<Pg> + AppearsOnTable<crate::schema::jobs::table> + Send,
{
    Ok(match sort_order {
        "asc" => query.order_by(column.asc()),
        "desc" => query.order_by(column.desc()),
        _ => bail!("Invalid sort_order"),
    })
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

            let sql = crate::schema::jobs::dsl::jobs.into_boxed();

            let sort_key = query.sort_key.as_ref().map(String::as_str).unwrap_or("id");
            let sort_order = query
                .sort_order
                .as_ref()
                .map(String::as_str)
                .unwrap_or("desc");
            let sql = match sort_key {
                "id" => sort_by_column(sql, crate::schema::jobs::dsl::id, sort_order)?,
                "pipeline_id" => {
                    sort_by_column(sql, crate::schema::jobs::dsl::pipeline_id, sort_order)?
                }
                "packages" => sort_by_column(sql, crate::schema::jobs::dsl::packages, sort_order)?,
                "arch" => sort_by_column(sql, crate::schema::jobs::dsl::arch, sort_order)?,
                _ => {
                    bail!("Invalid sort_key");
                }
            };

            let jobs = if query.items_per_page == -1 {
                sql.load::<Job>(conn)?
            } else {
                sql.offset((query.page - 1) * query.items_per_page)
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
