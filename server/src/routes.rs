use crate::{
    api::{self, JobSource, PipelineStatus},
    formatter::{to_html_build_result, to_markdown_build_result, FAILED, SUCCESS},
    job::get_crab_github_installation,
    models::{Job, NewWorker, Pipeline, Worker},
    DbPool, ARGS,
};
use anyhow::Context;
use axum::{
    extract::{Json, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use buildit_utils::LOONGARCH64;
use buildit_utils::{AMD64, ARM64, LOONGSON3, MIPS64R6EL, PPC64EL, RISCV64};
use common::{
    JobOk, JobResult, WorkerHeartbeatRequest, WorkerJobUpdateRequest, WorkerPollRequest,
    WorkerPollResponse,
};
use diesel::{Connection, ExpressionMethods, OptionalExtension, QueryDsl, RunQueryDsl};
use octocrab::models::CheckRunId;
use octocrab::params::checks::CheckRunConclusion;
use octocrab::params::checks::CheckRunOutput;
use serde::{Deserialize, Serialize};
use teloxide::types::ChatId;
use teloxide::{prelude::*, types::ParseMode};
use tracing::{error, info, warn};

pub async fn ping() -> &'static str {
    "PONG"
}

#[derive(Clone)]
pub struct AppState {
    pub pool: DbPool,
    pub bot: Bot,
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
    let pipeline =
        api::pipeline_new_pr(pool, payload.pr, payload.archs.as_ref().map(|s| s.as_str())).await?;
    Ok(Json(PipelineNewResponse { id: pipeline.id }))
}

pub async fn worker_heartbeat(
    State(AppState { pool, .. }): State<AppState>,
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

pub async fn worker_poll(
    State(AppState { pool, .. }): State<AppState>,
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
    let mut conn = pool
        .get()
        .context("Failed to get db connection from pool")?;

    let job = crate::schema::jobs::dsl::jobs
        .find(payload.job_id)
        .first::<Job>(&mut conn)?;

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

pub enum HandleSuccessResult {
    Ok,
    Retry(u8),
    DoNotRetry,
}

pub async fn handle_success_message(
    job: &Job,
    pipeline: &Pipeline,
    req: &WorkerJobUpdateRequest,
    bot: &Bot,
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
                    // "amd64" if job_parent.noarch => NOARCH,
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
