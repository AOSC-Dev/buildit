use crate::routes::{AnyhowError, AppState};
use crate::{
    api::{self},
    formatter::{to_html_build_result, to_markdown_build_result, FAILED, SUCCESS},
    github::get_crab_github_installation,
    models::{Job, NewWorker, Pipeline, Worker},
    ARGS,
};
use anyhow::anyhow;
use anyhow::Context;
use axum::extract::{Json, Query, State};
use buildit_utils::{AMD64, ARM64, LOONGSON3, MIPS64R6EL, PPC64EL, RISCV64};
use buildit_utils::{LOONGARCH64, NOARCH};

use chrono::{DateTime, Utc};
use common::{
    JobOk, JobResult, WorkerHeartbeatRequest, WorkerJobUpdateRequest, WorkerPollRequest,
    WorkerPollResponse,
};

use diesel::BoolExpressionMethods;
use diesel::{Connection, ExpressionMethods, OptionalExtension, QueryDsl, RunQueryDsl};
use octocrab::models::CheckRunId;
use octocrab::params::checks::CheckRunConclusion;
use octocrab::params::checks::CheckRunOutput;
use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};

use teloxide::types::ChatId;
use teloxide::{prelude::*, types::ParseMode};
use tracing::{error, info, warn};

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
    disk_free_space_bytes: i64,
    is_live: bool,
    last_heartbeat_time: DateTime<Utc>,
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
                    .order_by(crate::schema::workers::dsl::arch)
                    .load::<Worker>(conn)?
            } else {
                crate::schema::workers::dsl::workers
                    .order_by(crate::schema::workers::dsl::arch)
                    .offset((query.page - 1) * query.items_per_page)
                    .limit(query.items_per_page)
                    .load::<Worker>(conn)?
            };

            let mut items = vec![];
            let deadline = Utc::now() - chrono::Duration::try_seconds(300).unwrap();
            for worker in workers {
                items.push(WorkerListResponseItem {
                    id: worker.id,
                    hostname: worker.hostname,
                    arch: worker.arch,
                    logical_cores: worker.logical_cores,
                    memory_bytes: worker.memory_bytes,
                    disk_free_space_bytes: worker.disk_free_space_bytes,
                    is_live: worker.last_heartbeat_time > deadline,
                    last_heartbeat_time: worker.last_heartbeat_time,
                });
            }

            Ok(WorkerListResponse { total_items, items })
        })?,
    ))
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
                        disk_free_space_bytes.eq(payload.disk_free_space_bytes),
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
                    disk_free_space_bytes: payload.disk_free_space_bytes,
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

        let mut sql = jobs.filter(status.eq("created")).into_boxed();
        if payload.arch == "amd64" {
            // route noarch to amd64
            sql = sql.filter(arch.eq(&payload.arch).or(arch.eq("noarch")));
        } else {
            sql = sql.filter(arch.eq(&payload.arch));
        }

        // handle filters
        sql = sql
            .filter(
                require_min_core
                    .is_null()
                    .or(require_min_core.le(payload.logical_cores)),
            )
            .filter(
                require_min_total_mem
                    .is_null()
                    .or(require_min_total_mem.le(payload.memory_bytes)),
            )
            .filter(
                require_min_total_mem_per_core
                    .is_null()
                    .or(require_min_total_mem_per_core
                        .le((payload.memory_bytes as f32) / (payload.logical_cores as f32))),
            )
            .filter(
                require_min_disk
                    .is_null()
                    .or(require_min_disk.le(payload.disk_free_space_bytes)),
            );

        let res = sql.first::<Job>(conn).optional()?;
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
                    .set((status.eq("running"), assigned_worker_id.eq(worker.id)))
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
            // update github check run status to in-progress
            if let Some(github_check_run_id) = job.github_check_run_id {
                tokio::spawn(async move {
                    if let Ok(Some(crab)) = get_crab_github_installation().await {
                        let output = CheckRunOutput {
                            title: format!("Running on {}", payload.hostname),
                            summary: String::new(),
                            text: None,
                            annotations: vec![],
                            images: vec![],
                        };
                        if let Err(err) = crab
                            .checks("AOSC-Dev", "aosc-os-abbs")
                            .update_check_run(CheckRunId(github_check_run_id as u64))
                            .status(octocrab::params::checks::CheckRunStatus::InProgress)
                            .output(output)
                            .details_url(format!("https://buildit.aosc.io/jobs/{}", job.id))
                            .send()
                            .await
                        {
                            warn!("Failed to update check run: {}", err);
                        }
                    }
                });
            }

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

    if job.status != "running" || job.assigned_worker_id != Some(worker.id) {
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
                    status.eq(if res.build_success && res.pushpkg_success {
                        "success"
                    } else {
                        "failed"
                    }),
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

static GITHUB_PR_CHECKLIST_LOCK: Lazy<tokio::sync::Mutex<()>> =
    Lazy::new(|| tokio::sync::Mutex::new(()));

pub enum HandleSuccessResult {
    Ok,
    Retry(u8),
    DoNotRetry,
}

#[tracing::instrument(skip(bot))]
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
                        pipeline,
                        job,
                        job_ok,
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
                        error!("Failed to send build result to telegram: {}", e);
                        return update_retry(retry);
                    }
                } else {
                    error!("Telegram bot not configured");
                    return HandleSuccessResult::DoNotRetry;
                }
            }

            // if associated with github pr, update comments
            let new_content =
                to_markdown_build_result(pipeline, job, job_ok, &req.hostname, &req.arch, success);
            if let Some(pr_num) = pipeline.github_pr {
                let crab = match octocrab::Octocrab::builder()
                    .user_access_token(ARGS.github_access_token.clone())
                    .build()
                {
                    Ok(crab) => crab,
                    Err(e) => {
                        error!("Failed to build octocrab instance: {e}");
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
                        error!("Failed to list comments of pr: {e}");
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
                                    error!("Failed to delete comment from pr: {e}");
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
                // the operation is not atomic, so we use lock to avoid racing
                let _lock = GITHUB_PR_CHECKLIST_LOCK.lock().await;
                let pr = match crab
                    .pulls("AOSC-Dev", "aosc-os-abbs")
                    .get(pr_num as u64)
                    .await
                {
                    Ok(pr) => pr,
                    Err(e) => {
                        error!("Failed to get pr info: {e:?}");
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
                    error!("Failed to update pr body: {e}");
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
                        let builder = handler
                            .update_check_run(CheckRunId(github_check_run_id as u64))
                            .status(octocrab::params::checks::CheckRunStatus::Completed)
                            .output(output)
                            .conclusion(if success {
                                CheckRunConclusion::Success
                            } else {
                                CheckRunConclusion::Failure
                            })
                            .details_url(format!("https://buildit.aosc.io/jobs/{}", job.id));

                        if let Err(e) = builder.send().await {
                            error!("Failed to update github check run: {e}");
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
                        error!("Failed to send message to telegram: {e}");
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
                        error!("Failed to create octocrab instance: {e}");
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
                    error!("Failed to create comment on github: {e}");
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

pub async fn worker_status(
    State(AppState { pool, .. }): State<AppState>,
) -> Result<Json<Vec<Worker>>, AnyhowError> {
    Ok(Json(api::worker_status(pool).await?))
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
    disk_free_space_bytes: i64,

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
                disk_free_space_bytes: worker.disk_free_space_bytes,
                last_heartbeat_time: worker.last_heartbeat_time,

                running_job_id: running_job.map(|job| job.id),
                built_job_count,
            })
        })?,
    ))
}
