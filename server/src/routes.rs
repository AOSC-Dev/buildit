use crate::{
    api::{self, JobSource, PipelineStatus},
    formatter::{to_html_build_result, to_markdown_build_result, FAILED, SUCCESS},
    github::get_crab_github_installation,
    models::{Job, NewWorker, Pipeline, Worker},
    DbPool, ARGS,
};
use anyhow::anyhow;
use anyhow::Context;
use axum::{
    extract::{Json, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use buildit_utils::{AMD64, ARM64, LOONGSON3, MIPS64R6EL, PPC64EL, RISCV64};
use buildit_utils::{LOONGARCH64, NOARCH};
use chrono::Utc;
use common::{
    JobOk, JobResult, WorkerHeartbeatRequest, WorkerJobUpdateRequest, WorkerPollRequest,
    WorkerPollResponse,
};
use diesel::dsl::{count, sum};
use diesel::BoolExpressionMethods;
use diesel::{Connection, ExpressionMethods, OptionalExtension, QueryDsl, RunQueryDsl};
use octocrab::models::CheckRunId;
use octocrab::params::checks::CheckRunConclusion;
use octocrab::params::checks::CheckRunOutput;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use teloxide::types::ChatId;
use teloxide::{prelude::*, types::ParseMode};
use tracing::{error, info, warn};

pub async fn ping() -> &'static str {
    "PONG"
}

#[derive(Clone)]
pub struct AppState {
    pub pool: DbPool,
    pub bot: Option<Bot>,
}

// learned from https://github.com/tokio-rs/axum/blob/main/examples/anyhow-error-response/src/main.rs
pub struct AnyhowError(anyhow::Error);

impl IntoResponse for AnyhowError {
    fn into_response(self) -> Response {
        info!("Returing internal server error for {}", self.0);
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
        payload.archs.as_ref().map(|s| s.as_str()),
        &JobSource::Manual,
    )
    .await?;
    Ok(Json(PipelineNewResponse { id: pipeline.id }))
}

pub async fn worker_heartbeat(
    State(AppState { pool, .. }): State<AppState>,
    Json(payload): Json<WorkerHeartbeatRequest>,
) -> Result<(), AnyhowError> {
    if payload.worker_secret != ARGS.worker_secret {
        return Err(anyhow!("Invalid worker secret").into());
    }

    // insert or update worker
    let mut conn = pool
        .get()
        .context("Failed to get db connection from pool")?;

    conn.transaction::<(), diesel::result::Error, _>(|conn| {
        use crate::schema::workers::dsl::*;
        match workers
            .filter(hostname.eq(&payload.hostname))
            .filter(arch.eq(&payload.arch))
            .first::<Worker>(conn)
            .optional()?
        {
            Some(worker) => {
                // existing worker, update it
                diesel::update(workers.find(worker.id))
                    .set((
                        git_commit.eq(payload.git_commit),
                        memory_bytes.eq(payload.memory_bytes),
                        logical_cores.eq(payload.logical_cores),
                        last_heartbeat_time.eq(chrono::Utc::now()),
                    ))
                    .execute(conn)?;
            }
            None => {
                let new_worker = NewWorker {
                    hostname: payload.hostname.clone(),
                    arch: payload.arch.clone(),
                    git_commit: payload.git_commit.clone(),
                    memory_bytes: payload.memory_bytes,
                    logical_cores: payload.logical_cores,
                    last_heartbeat_time: chrono::Utc::now(),
                };
                diesel::insert_into(crate::schema::workers::table)
                    .values(&new_worker)
                    .execute(conn)?;
            }
        }
        Ok(())
    })?;
    Ok(())
}

pub async fn worker_poll(
    State(AppState { pool, .. }): State<AppState>,
    Json(payload): Json<WorkerPollRequest>,
) -> Result<Json<Option<WorkerPollResponse>>, AnyhowError> {
    if payload.worker_secret != ARGS.worker_secret {
        return Err(anyhow!("Invalid worker secret").into());
    }

    // find a job that can be assigned to the worker
    let mut conn = pool
        .get()
        .context("Failed to get db connection from pool")?;

    match conn.transaction::<Option<(Pipeline, Job)>, diesel::result::Error, _>(|conn| {
        use crate::schema::jobs::dsl::*;
        let res = if payload.arch == "amd64" {
            // route noarch to amd64
            jobs.filter(status.eq("created"))
                .filter(arch.eq(&payload.arch).or(arch.eq("noarch")))
                .first::<Job>(conn)
                .optional()?
        } else {
            jobs.filter(status.eq("created"))
                .filter(arch.eq(&payload.arch))
                .first::<Job>(conn)
                .optional()?
        };
        match res {
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
                job_id: job.id,
                git_branch: pipeline.git_branch,
                git_sha: pipeline.git_sha,
                packages: job.packages,
            })))
        }
        None => Ok(Json(None)),
    }
}

pub async fn worker_job_update(
    State(AppState { pool, bot }): State<AppState>,
    Json(payload): Json<WorkerJobUpdateRequest>,
) -> Result<(), AnyhowError> {
    if payload.worker_secret != ARGS.worker_secret {
        return Err(anyhow!("Invalid worker secret").into());
    }

    let mut conn = pool
        .get()
        .context("Failed to get db connection from pool")?;

    let job = crate::schema::jobs::dsl::jobs
        .find(payload.job_id)
        .first::<Job>(&mut conn)?;

    let worker = crate::schema::workers::dsl::workers
        .filter(crate::schema::workers::dsl::hostname.eq(&payload.hostname))
        .filter(crate::schema::workers::dsl::arch.eq(&payload.arch))
        .first::<Worker>(&mut conn)?;

    if job.status != "assigned" || job.assigned_worker_id != Some(worker.id) {
        return Err(anyhow!("Worker not assigned to the job").into());
    }

    let pipeline = crate::schema::pipelines::dsl::pipelines
        .find(job.pipeline_id)
        .first::<Pipeline>(&mut conn)?;

    let mut retry = None;
    loop {
        if retry.map(|x| x < 5).unwrap_or(true) {
            match handle_success_message(&job, &pipeline, &payload, &bot, retry).await {
                HandleSuccessResult::Ok | HandleSuccessResult::DoNotRetry => {
                    break;
                }
                HandleSuccessResult::Retry(x) => {
                    retry = Some(x);
                    continue;
                }
            }
        } else {
            break;
        }
    }

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
                    built_by_worker_id.eq(Some(worker.id)),
                ))
                .execute(&mut conn)?;
        }
        JobResult::Error(err) => {
            diesel::update(jobs.filter(id.eq(payload.job_id)))
                .set((
                    status.eq("error"),
                    error_message.eq(err),
                    built_by_worker_id.eq(Some(worker.id)),
                ))
                .execute(&mut conn)?;
        }
    }
    Ok(())
}

pub enum HandleSuccessResult {
    Ok,
    Retry(u8),
    DoNotRetry,
}

pub async fn handle_success_message(
    job: &Job,
    pipeline: &Pipeline,
    req: &WorkerJobUpdateRequest,
    bot: &Option<Bot>,
    retry: Option<u8>,
) -> HandleSuccessResult {
    match &req.result {
        JobResult::Ok(job_ok) => {
            info!("Processing job result {:?} ...", job_ok);

            let JobOk {
                build_success,
                pushpkg_success,
                ..
            } = &job_ok;

            let success = *build_success && *pushpkg_success;

            if pipeline.source == "telegram" {
                if let Some(bot) = bot {
                    let s = to_html_build_result(
                        &pipeline,
                        &job,
                        &job_ok,
                        &req.hostname,
                        &req.arch,
                        success,
                    );

                    if let Err(e) = bot
                        .send_message(ChatId(pipeline.telegram_user.unwrap()), &s)
                        .parse_mode(ParseMode::Html)
                        .disable_web_page_preview(true)
                        .await
                    {
                        error!("{}", e);
                        return update_retry(retry);
                    }
                } else {
                    error!("Telegram bot not configured");
                    return HandleSuccessResult::DoNotRetry;
                }
            }

            // if associated with github pr, update comments
            let new_content = to_markdown_build_result(
                &pipeline,
                &job,
                &job_ok,
                &req.hostname,
                &req.arch,
                success,
            );
            if let Some(pr_num) = pipeline.github_pr {
                let crab = match octocrab::Octocrab::builder()
                    .user_access_token(ARGS.github_access_token.clone())
                    .build()
                {
                    Ok(crab) => crab,
                    Err(e) => {
                        error!("{e}");
                        return HandleSuccessResult::DoNotRetry;
                    }
                };

                let comments = crab
                    .issues("AOSC-Dev", "aosc-os-abbs")
                    .list_comments(pr_num as u64)
                    .send()
                    .await;

                let comments = match comments {
                    Ok(c) => c,
                    Err(e) => {
                        error!("{e}");
                        return update_retry(retry);
                    }
                };

                for c in comments {
                    if c.user.login == "aosc-buildit-bot" {
                        let body = c.body.unwrap_or_else(String::new);
                        if !body
                            .split_ascii_whitespace()
                            .next()
                            .map(|x| x == SUCCESS || x == FAILED)
                            .unwrap_or(false)
                        {
                            continue;
                        }

                        for line in body.split('\n') {
                            let arch = line.strip_prefix("Architecture:").map(|x| x.trim());
                            if arch.map(|x| x == job.arch).unwrap_or(false) {
                                if let Err(e) = crab
                                    .issues("AOSC-Dev", "aosc-os-abbs")
                                    .delete_comment(c.id)
                                    .await
                                {
                                    error!("{e}");
                                    return update_retry(retry);
                                }
                            }
                        }
                    }
                }

                // Disable comment posting, since we have check run reporting
                /*
                if let Err(e) = crab
                    .issues("AOSC-Dev", "aosc-os-abbs")
                    .create_comment(pr_num, new_content.clone())
                    .await
                {
                    error!("{e}");
                    return update_retry(retry);
                }
                */

                // update checklist
                let pr = match crab
                    .pulls("AOSC-Dev", "aosc-os-abbs")
                    .get(pr_num as u64)
                    .await
                {
                    Ok(pr) => pr,
                    Err(e) => {
                        error!("{e}");
                        return update_retry(retry);
                    }
                };

                let body = if let Some(body) = pr.body {
                    body
                } else {
                    return HandleSuccessResult::DoNotRetry;
                };

                let pr_arch = match job.arch.as_str() {
                    "noarch" => NOARCH,
                    "amd64" => AMD64,
                    "arm64" => ARM64,
                    "loongson3" => LOONGSON3,
                    "mips64r6el" => MIPS64R6EL,
                    "ppc64el" => PPC64EL,
                    "riscv64" => RISCV64,
                    "loongarch64" => LOONGARCH64,
                    x => {
                        error!("Unknown architecture: {x}");
                        return HandleSuccessResult::DoNotRetry;
                    }
                };

                let body = if success {
                    body.replace(&format!("- [ ] {pr_arch}"), &format!("- [x] {pr_arch}"))
                } else {
                    body.replace(&format!("- [x] {pr_arch}"), &format!("- [ ] {pr_arch}"))
                };

                if let Err(e) = crab
                    .pulls("AOSC-Dev", "aosc-os-abbs")
                    .update(pr_num as u64)
                    .body(body)
                    .send()
                    .await
                {
                    error!("{e}");
                    return update_retry(retry);
                }
            }

            // if associated with github check run, update status
            if let Some(github_check_run_id) = job.github_check_run_id {
                // authenticate with github app
                match get_crab_github_installation().await {
                    Ok(Some(crab)) => {
                        let handler = crab.checks("AOSC-Dev", "aosc-os-abbs");
                        let output = CheckRunOutput {
                            title: format!(
                                "Built {} packages in {}s",
                                job_ok.successful_packages.len(),
                                job_ok.elapsed_secs,
                            ),
                            summary: new_content,
                            text: None,
                            annotations: vec![],
                            images: vec![],
                        };
                        let mut builder = handler
                            .update_check_run(CheckRunId(github_check_run_id as u64))
                            .status(octocrab::params::checks::CheckRunStatus::Completed)
                            .output(output)
                            .conclusion(if success {
                                CheckRunConclusion::Success
                            } else {
                                CheckRunConclusion::Failure
                            });

                        if let Some(log) = &job_ok.log_url {
                            builder = builder.details_url(log);
                        }

                        if let Err(e) = builder.send().await {
                            error!("{e}");
                            return update_retry(retry);
                        }
                    }
                    Ok(None) => {
                        // github app unavailable
                    }
                    Err(err) => {
                        warn!("Failed to get installation token: {}", err);
                        return update_retry(retry);
                    }
                }
            }
        }
        JobResult::Error(error) => {
            if pipeline.source == "telegram" {
                if let Some(bot) = bot {
                    if let Err(e) = bot
                        .send_message(
                            ChatId(pipeline.telegram_user.unwrap()),
                            format!(
                                "{}({}) build packages: {:?} Got Error: {}",
                                req.hostname, job.arch, pipeline.packages, error
                            ),
                        )
                        .await
                    {
                        error!("{e}");
                        return update_retry(retry);
                    }
                } else {
                    error!("Telegram bot not configured");
                    return HandleSuccessResult::DoNotRetry;
                }
            } else if pipeline.source == "github" {
                let crab = match octocrab::Octocrab::builder()
                    .user_access_token(ARGS.github_access_token.clone())
                    .build()
                {
                    Ok(crab) => crab,
                    Err(e) => {
                        error!("{e}");
                        return HandleSuccessResult::DoNotRetry;
                    }
                };

                if let Err(e) = crab
                    .issues("AOSC-Dev", "aosc-os-abbs")
                    .create_comment(
                        pipeline.github_pr.unwrap() as u64,
                        format!(
                            "{}({}) build packages: {:?} Got Error: {}",
                            req.hostname, job.arch, pipeline.packages, error
                        ),
                    )
                    .await
                {
                    error!("{e}");
                    return update_retry(retry);
                }
            }
        }
    }

    HandleSuccessResult::Ok
}

pub fn update_retry(retry: Option<u8>) -> HandleSuccessResult {
    match retry {
        Some(retry) => HandleSuccessResult::Retry(retry + 1),
        None => HandleSuccessResult::Retry(1),
    }
}

pub async fn pipeline_status(
    State(AppState { pool, .. }): State<AppState>,
) -> Result<Json<Vec<PipelineStatus>>, AnyhowError> {
    Ok(Json(api::pipeline_status(pool).await?))
}

pub async fn worker_status(
    State(AppState { pool, .. }): State<AppState>,
) -> Result<Json<Vec<Worker>>, AnyhowError> {
    Ok(Json(api::worker_status(pool).await?))
}

#[derive(Serialize, Default)]
pub struct DashboardStatusResponseByArch {
    total_worker_count: i64,
    live_worker_count: i64,
    total_logical_cores: i64,
    total_memory_bytes: bigdecimal::BigDecimal,

    total_job_count: i64,
    pending_job_count: i64,
    running_job_count: i64,
}

#[derive(Serialize)]
pub struct DashboardStatusResponse {
    total_pipeline_count: i64,

    total_job_count: i64,
    pending_job_count: i64,
    running_job_count: i64,
    finished_job_count: i64,

    total_worker_count: i64,
    live_worker_count: i64,
    total_logical_cores: i64,
    total_memory_bytes: bigdecimal::BigDecimal,

    by_arch: BTreeMap<String, DashboardStatusResponseByArch>,
}

pub async fn dashboard_status(
    State(AppState { pool, .. }): State<AppState>,
) -> Result<Json<DashboardStatusResponse>, AnyhowError> {
    let mut conn = pool
        .get()
        .context("Failed to get db connection from pool")?;

    Ok(Json(
        conn.transaction::<DashboardStatusResponse, diesel::result::Error, _>(|conn| {
            let total_pipeline_count = crate::schema::pipelines::dsl::pipelines
                .count()
                .get_result(conn)?;
            let total_job_count = crate::schema::jobs::dsl::jobs.count().get_result(conn)?;
            let pending_job_count = crate::schema::jobs::dsl::jobs
                .filter(crate::schema::jobs::dsl::status.eq("created"))
                .count()
                .get_result(conn)?;
            let running_job_count = crate::schema::jobs::dsl::jobs
                .filter(crate::schema::jobs::dsl::status.eq("assigned"))
                .count()
                .get_result(conn)?;
            let finished_job_count = crate::schema::jobs::dsl::jobs
                .filter(crate::schema::jobs::dsl::status.eq("finished"))
                .count()
                .get_result(conn)?;
            let total_worker_count = crate::schema::workers::dsl::workers
                .count()
                .get_result(conn)?;
            let (total_logical_cores, total_memory_bytes) = crate::schema::workers::dsl::workers
                .select((
                    sum(crate::schema::workers::dsl::logical_cores),
                    sum(crate::schema::workers::dsl::memory_bytes),
                ))
                .get_result::<(Option<i64>, Option<bigdecimal::BigDecimal>)>(conn)?;

            let deadline = Utc::now() - chrono::Duration::try_seconds(300).unwrap();
            let live_worker_count = crate::schema::workers::dsl::workers
                .filter(crate::schema::workers::last_heartbeat_time.gt(deadline))
                .count()
                .get_result(conn)?;

            // collect information by arch
            let mut by_arch: BTreeMap<String, DashboardStatusResponseByArch> = BTreeMap::new();

            for (arch, count, cores, bytes) in crate::schema::workers::dsl::workers
                .group_by(crate::schema::workers::dsl::arch)
                .select((
                    crate::schema::workers::dsl::arch,
                    count(crate::schema::workers::dsl::id),
                    sum(crate::schema::workers::dsl::logical_cores),
                    sum(crate::schema::workers::dsl::memory_bytes),
                ))
                .load::<(String, i64, Option<i64>, Option<bigdecimal::BigDecimal>)>(conn)?
            {
                by_arch.entry(arch.clone()).or_default().total_worker_count = count;
                by_arch.entry(arch.clone()).or_default().total_logical_cores =
                    cores.unwrap_or_default();
                by_arch.entry(arch).or_default().total_memory_bytes = bytes.unwrap_or_default();
            }

            for (arch, count) in crate::schema::workers::dsl::workers
                .filter(crate::schema::workers::last_heartbeat_time.gt(deadline))
                .group_by(crate::schema::workers::dsl::arch)
                .select((
                    crate::schema::workers::dsl::arch,
                    count(crate::schema::workers::dsl::id),
                ))
                .load::<(String, i64)>(conn)?
            {
                by_arch.entry(arch).or_default().live_worker_count = count;
            }

            for (arch, count) in crate::schema::jobs::dsl::jobs
                .group_by(crate::schema::jobs::dsl::arch)
                .select((
                    crate::schema::jobs::dsl::arch,
                    count(crate::schema::jobs::dsl::id),
                ))
                .load::<(String, i64)>(conn)?
            {
                let arch = if arch == "noarch" {
                    "amd64".to_string()
                } else {
                    arch
                };
                by_arch.entry(arch).or_default().total_job_count = count;
            }

            for (arch, count) in crate::schema::jobs::dsl::jobs
                .filter(crate::schema::jobs::dsl::status.eq("created"))
                .group_by(crate::schema::jobs::dsl::arch)
                .select((
                    crate::schema::jobs::dsl::arch,
                    count(crate::schema::jobs::dsl::id),
                ))
                .load::<(String, i64)>(conn)?
            {
                let arch = if arch == "noarch" {
                    "amd64".to_string()
                } else {
                    arch
                };
                by_arch.entry(arch).or_default().pending_job_count = count;
            }

            for (arch, count) in crate::schema::jobs::dsl::jobs
                .filter(crate::schema::jobs::dsl::status.eq("assigned"))
                .group_by(crate::schema::jobs::dsl::arch)
                .select((
                    crate::schema::jobs::dsl::arch,
                    count(crate::schema::jobs::dsl::id),
                ))
                .load::<(String, i64)>(conn)?
            {
                let arch = if arch == "noarch" {
                    "amd64".to_string()
                } else {
                    arch
                };
                by_arch.entry(arch).or_default().running_job_count = count;
            }

            Ok(DashboardStatusResponse {
                total_pipeline_count,
                total_job_count,
                pending_job_count,
                running_job_count,
                finished_job_count,
                total_worker_count,
                live_worker_count,
                total_logical_cores: total_logical_cores.unwrap_or(0),
                total_memory_bytes: total_memory_bytes.unwrap_or_default(),
                by_arch,
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
pub struct WorkerListRequest {
    page: i64,
    items_per_page: i64,
}

#[derive(Serialize)]
pub struct WorkerListResponseItem {
    id: i32,
    hostname: String,
    arch: String,
    logical_cores: i32,
    memory_bytes: i64,
}

#[derive(Serialize)]
pub struct WorkerListResponse {
    total_items: i64,
    items: Vec<WorkerListResponseItem>,
}

pub async fn worker_list(
    Query(query): Query<WorkerListRequest>,
    State(AppState { pool, .. }): State<AppState>,
) -> Result<Json<WorkerListResponse>, AnyhowError> {
    let mut conn = pool
        .get()
        .context("Failed to get db connection from pool")?;

    Ok(Json(
        conn.transaction::<WorkerListResponse, diesel::result::Error, _>(|conn| {
            let total_items = crate::schema::workers::dsl::workers
                .count()
                .get_result(conn)?;

            let workers = if query.items_per_page == -1 {
                crate::schema::workers::dsl::workers
                    .order_by(crate::schema::workers::dsl::id)
                    .load::<Worker>(conn)?
            } else {
                crate::schema::workers::dsl::workers
                    .order_by(crate::schema::workers::dsl::id)
                    .offset((query.page - 1) * query.items_per_page)
                    .limit(query.items_per_page)
                    .load::<Worker>(conn)?
            };

            let mut items = vec![];
            for worker in workers {
                items.push(WorkerListResponseItem {
                    id: worker.id,
                    hostname: worker.hostname,
                    arch: worker.arch,
                    logical_cores: worker.logical_cores,
                    memory_bytes: worker.memory_bytes,
                });
            }

            Ok(WorkerListResponse { total_items, items })
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

#[derive(Deserialize)]
pub struct PipelineInfoRequest {
    pipeline_id: i32,
}

#[derive(Serialize)]
pub struct PipelineInfoResponseJob {
    job_id: i32,
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
                .map(|job| PipelineInfoResponseJob { job_id: job.id })
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
