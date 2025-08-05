use crate::{
    ARGS, DbPool,
    github::{get_crab_github_installation, get_packages_from_pr},
    models::{Job, NewJob, NewPipeline, Pipeline, User, Worker},
};
use anyhow::Context;
use anyhow::{anyhow, bail};
use buildit_utils::{
    ABBS_REPO_LOCK, ALL_ARCH,
    github::{get_archs, get_environment_requirement, resolve_packages, update_abbs},
};
use diesel::r2d2::PoolTransactionManager;
use diesel::{
    ExpressionMethods, OptionalExtension, PgConnection, QueryDsl, RunQueryDsl, dsl::count,
};
use diesel::{
    SelectableHelper,
    connection::{AnsiTransactionManager, TransactionManager},
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use tracing::warn;

#[derive(Debug, Copy, Clone, Serialize, Deserialize)]
pub enum JobSource {
    /// Telegram user/group
    Telegram(i64),
    /// GitHub PR comment
    GitHub { pr: u64, user: i64 },
    /// Manual
    Manual,
}

// create github check run for the specified git commit
#[tracing::instrument(skip(crab))]
async fn create_check_run(crab: octocrab::Octocrab, arch: String, git_sha: String) -> Option<u64> {
    match crab
        .checks("AOSC-Dev", "aosc-os-abbs")
        .create_check_run(format!("buildit {}", arch), git_sha)
        .status(octocrab::params::checks::CheckRunStatus::Queued)
        .send()
        .await
    {
        Ok(check_run) => {
            return Some(check_run.id.0);
        }
        Err(err) => {
            warn!("Failed to create check run: {}", err);
        }
    }
    return None;
}

#[tracing::instrument(skip(pool))]
#[allow(clippy::too_many_arguments)]
pub async fn pipeline_new(
    pool: DbPool,
    git_branch: &str,
    git_sha: Option<&str>,
    github_pr: Option<u64>,
    packages: &str,
    archs: &str,
    source: JobSource,
    skip_git_fetch: bool,
) -> anyhow::Result<(Pipeline, Vec<Job>)> {
    // sanitize archs arg
    let mut archs: Vec<&str> = archs.split(',').collect();
    archs.sort();
    archs.dedup();
    if archs.contains(&"noarch") && archs.len() > 1 {
        return Err(anyhow!("Architecture noarch must not be mixed with others"));
    }
    if archs.contains(&"mainline") {
        // archs
        archs.extend(ALL_ARCH.iter());
        archs.retain(|arch| *arch != "mainline");
    }
    for arch in &archs {
        if !ALL_ARCH.contains(arch) && arch != &"noarch" {
            return Err(anyhow!("Architecture {arch} is not supported"));
        }
    }
    archs.sort();
    archs.dedup();

    // sanitize packages arg
    if !packages.chars().all(|ch| {
        ch.is_ascii_alphanumeric()
            || ch == ','
            || ch == '-'
            || ch == '.'
            || ch == '+'
            || ch == ':'
            || ch == '/'
    }) {
        return Err(anyhow!("Invalid packages: {packages}"));
    }

    // sanitize git_branch arg
    if !git_branch
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || ch == '.' || ch == '-' || ch == '+' || ch == '_')
    {
        return Err(anyhow!("Invalid branch: {git_branch}"));
    }

    let lock = ABBS_REPO_LOCK.lock().await;
    update_abbs(git_branch, &ARGS.abbs_path, skip_git_fetch)
        .await
        .context("Failed to update ABBS tree")?;

    // resolve branch name to commit hash if not specified
    let git_sha = match git_sha {
        Some(git_sha) => {
            if !git_sha.chars().all(|ch| ch.is_ascii_alphanumeric()) {
                return Err(anyhow!("Invalid git sha: {git_sha}"));
            }
            git_sha.to_string()
        }
        None => {
            let output = tokio::process::Command::new("git")
                .arg("rev-parse")
                .arg("HEAD")
                .current_dir(&ARGS.abbs_path)
                .output()
                .await
                .context("Failed to resolve branch to git commit")?;
            String::from_utf8_lossy(&output.stdout).trim().to_string()
        }
    };

    // find environment requirements
    let resolved_pkgs = resolve_packages(
        &packages
            .split(",")
            .map(|s| s.to_string())
            .collect::<Vec<String>>(),
        &ARGS.abbs_path,
    )
    .context("Resolve packages")?;

    let env_req = get_environment_requirement(&ARGS.abbs_path, &resolved_pkgs);
    drop(lock);

    // create a new pipeline
    let mut conn = pool
        .get()
        .context("Failed to get db connection from pool")?;
    use crate::schema::pipelines;
    let (source, github_pr, telegram_user, creator_user_id) = match source {
        JobSource::Telegram(id) => {
            // lookup user id via telegram chat id
            let user = crate::schema::users::dsl::users
                .filter(crate::schema::users::dsl::telegram_chat_id.eq(id))
                .first::<User>(&mut conn)
                .optional()?;
            let creator_user_id = user.map(|user| user.id);
            ("telegram", github_pr, Some(id), creator_user_id)
        }
        JobSource::GitHub { pr, user } => {
            let user = crate::schema::users::dsl::users
                .filter(crate::schema::users::dsl::github_id.eq(user))
                .first::<User>(&mut conn)
                .optional()?;
            let telegram_user = user.as_ref().and_then(|user| user.telegram_chat_id);
            let creator_user_id = user.map(|user| user.id);
            ("github", Some(pr), telegram_user, creator_user_id)
        }
        JobSource::Manual => ("manual", github_pr, None, None),
    };
    let new_pipeline = NewPipeline {
        packages: packages.to_string(),
        archs: archs.join(","),
        git_branch: git_branch.to_string(),
        git_sha: git_sha.clone(),
        creation_time: chrono::Utc::now(),
        source: source.to_string(),
        github_pr: github_pr.map(|pr| pr as i64),
        telegram_user,
        creator_user_id,
    };
    let pipeline = diesel::insert_into(pipelines::table)
        .values(&new_pipeline)
        .returning(Pipeline::as_returning())
        .get_result(&mut conn)
        .context("Failed to create pipeline")?;

    // authenticate with github app
    let crab = match get_crab_github_installation().await {
        Ok(Some(crab)) => Some(crab),
        Ok(None) => {
            // github app unavailable
            None
        }
        Err(err) => {
            warn!("Failed to build octocrab: {}", err);
            None
        }
    };

    // for eatch arch, create github check run in parallel
    let github_check_run_ids: Vec<Option<u64>> = if let Some(crab) = &crab {
        let mut handles = vec![];
        for arch in &archs {
            handles.push(tokio::spawn(create_check_run(
                crab.clone(),
                arch.to_string(),
                git_sha.to_string(),
            )));
        }

        let mut res = vec![];
        for handle in handles {
            res.push(handle.await.unwrap());
        }
        res
    } else {
        vec![None; archs.len()]
    };

    // for each arch, create a new job
    let mut jobs = Vec::new();
    for (arch, check_run_id) in archs.iter().zip(github_check_run_ids.iter()) {
        // create a new job
        use crate::schema::jobs;
        let env_req_current = env_req.get(*arch).cloned().unwrap_or_default();
        let new_job = NewJob {
            pipeline_id: pipeline.id,
            packages: packages.to_string(),
            arch: arch.to_string(),
            creation_time: chrono::Utc::now(),
            status: "created".to_string(),
            github_check_run_id: check_run_id.map(|id| id as i64),
            require_min_core: env_req_current.min_core,
            require_min_total_mem: env_req_current.min_total_mem,
            require_min_total_mem_per_core: env_req_current.min_total_mem_per_core,
            require_min_disk: env_req_current.min_disk,
        };
        jobs.push(
            diesel::insert_into(jobs::table)
                .values(&new_job)
                .returning(Job::as_returning())
                .get_result(&mut conn)
                .context("Failed to create job")?,
        );
    }

    Ok((pipeline, jobs))
}

#[tracing::instrument(skip(pool))]
pub async fn pipeline_new_pr(
    pool: DbPool,
    pr: u64,
    archs: Option<&str>,
    source: JobSource,
) -> anyhow::Result<(Pipeline, Vec<Job>)> {
    match octocrab::instance()
        .pulls("AOSC-Dev", "aosc-os-abbs")
        .get(pr)
        .await
    {
        Ok(pr) => {
            // If the pull request has been merged,
            // build and push packages based on stable
            let (git_branch, git_sha) = if pr.merged_at.is_some() {
                (
                    "stable",
                    pr.merge_commit_sha
                        .as_ref()
                        .context("merge_commit_sha should not be None")?,
                )
            } else {
                (pr.head.ref_field.as_str(), &pr.head.sha)
            };

            if pr.head.repo.as_ref().and_then(|x| x.fork).unwrap_or(false) {
                return Err(anyhow!("Failed to create job: Pull request is a fork"));
            }

            // find lines starting with #buildit
            let packages = get_packages_from_pr(&pr);
            if !packages.is_empty() {
                let mut skip_git_fetch = false;
                let archs = if let Some(archs) = archs {
                    archs.to_string()
                } else {
                    let path = &ARGS.abbs_path;

                    let _lock = ABBS_REPO_LOCK.lock().await;
                    update_abbs(git_branch, &ARGS.abbs_path, false)
                        .await
                        .context("Failed to update ABBS tree")?;
                    // skip next git fetch in pipeline_new
                    skip_git_fetch = true;

                    let resolved_packages =
                        resolve_packages(&packages, path).context("Failed to resolve packages")?;

                    get_archs(path, &resolved_packages).join(",")
                };

                pipeline_new(
                    pool,
                    git_branch,
                    Some(git_sha),
                    Some(pr.number),
                    &packages.join(","),
                    &archs,
                    source,
                    skip_git_fetch,
                )
                .await
            } else {
                Err(anyhow!(
                    "Please list packages to build in pr info starting with '#buildit'"
                ))
            }
        }
        Err(err) => Err(anyhow!("Failed to get pr info: {err:?}")),
    }
}

#[derive(Serialize)]
pub struct PipelineStatus {
    pub arch: String,
    pub pending: u64,
    pub running: u64,
    pub available_servers: u64,
}

#[tracing::instrument(skip(pool))]
pub async fn pipeline_status(pool: DbPool) -> anyhow::Result<Vec<PipelineStatus>> {
    let mut conn = pool
        .get()
        .context("Failed to get db connection from pool")?;
    // find pending/running jobs
    let mut pending: BTreeMap<String, i64> = crate::schema::jobs::dsl::jobs
        .filter(crate::schema::jobs::dsl::status.eq("created"))
        .group_by(crate::schema::jobs::dsl::arch)
        .select((
            crate::schema::jobs::dsl::arch,
            count(crate::schema::jobs::dsl::id),
        ))
        .load::<(String, i64)>(&mut conn)?
        .into_iter()
        .collect();
    let mut running: BTreeMap<String, i64> = crate::schema::jobs::dsl::jobs
        .filter(crate::schema::jobs::dsl::status.eq("running"))
        .group_by(crate::schema::jobs::dsl::arch)
        .select((
            crate::schema::jobs::dsl::arch,
            count(crate::schema::jobs::dsl::id),
        ))
        .load::<(String, i64)>(&mut conn)?
        .into_iter()
        .collect();

    use crate::schema::workers::dsl::*;
    let available_servers: BTreeMap<String, i64> = workers
        .group_by(arch)
        .select((arch, count(id)))
        .load::<(String, i64)>(&mut conn)?
        .into_iter()
        .collect();

    // fold noarch into amd64
    let pending_noarch = *pending.get("noarch").unwrap_or(&0);
    *pending.entry("amd64".to_string()).or_default() += pending_noarch;
    let running_noarch = *running.get("noarch").unwrap_or(&0);
    *running.entry("amd64".to_string()).or_default() += running_noarch;
    let pending_noarch = *pending.get("optenv32").unwrap_or(&0);
    *pending.entry("amd64".to_string()).or_default() += pending_noarch;
    let running_noarch = *running.get("optenv32").unwrap_or(&0);
    *running.entry("amd64".to_string()).or_default() += running_noarch;

    let mut res = vec![];
    for a in ALL_ARCH {
        res.push(PipelineStatus {
            arch: a.to_string(),
            pending: *pending.get(*a).unwrap_or(&0) as u64,
            running: *running.get(*a).unwrap_or(&0) as u64,
            available_servers: *available_servers.get(*a).unwrap_or(&0) as u64,
        });
    }

    Ok(res)
}

#[tracing::instrument(skip(pool))]
pub async fn worker_status(pool: DbPool) -> anyhow::Result<Vec<Worker>> {
    let mut conn = pool
        .get()
        .context("Failed to get db connection from pool")?;

    let workers = crate::schema::workers::dsl::workers.load::<Worker>(&mut conn)?;
    Ok(workers)
}

async fn job_restart_in_transaction(job_id: i32, conn: &mut PgConnection) -> anyhow::Result<Job> {
    let job = crate::schema::jobs::dsl::jobs
        .find(job_id)
        .get_result::<Job>(conn)?;
    let pipeline = crate::schema::pipelines::dsl::pipelines
        .find(job.pipeline_id)
        .get_result::<Pipeline>(conn)?;

    // job must be failed
    if job.status != "failed" {
        bail!("Cannot restart the job unless it was failed");
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
        require_min_core: job.require_min_core,
        require_min_total_mem: job.require_min_total_mem,
        require_min_total_mem_per_core: job.require_min_total_mem_per_core,
        require_min_disk: job.require_min_disk,
    };

    // create new github check run if the restarted job has one
    if job.github_check_run_id.is_some() {
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
                        new_job.github_check_run_id = Some(check_run.id.0 as i64);
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
    }

    let new_job: Job = diesel::insert_into(jobs::table)
        .values(&new_job)
        .get_result(conn)
        .context("Failed to create job")?;
    Ok(new_job)
}

#[tracing::instrument(skip(pool))]
pub async fn job_restart(pool: DbPool, job_id: i32) -> anyhow::Result<Job> {
    let mut conn = pool
        .get()
        .context("Failed to get db connection from pool")?;

    // manually handle transaction, since we want to use async in transaction
    PoolTransactionManager::<AnsiTransactionManager>::begin_transaction(&mut conn)?;
    match job_restart_in_transaction(job_id, &mut conn).await {
        Ok(new_job) => {
            PoolTransactionManager::<AnsiTransactionManager>::commit_transaction(&mut conn)?;
            return Ok(new_job);
        }
        Err(err) => {
            match PoolTransactionManager::<AnsiTransactionManager>::rollback_transaction(&mut conn)
            {
                Ok(()) => {
                    return Err(err);
                }
                Err(rollback_err) => {
                    return Err(err.context(rollback_err));
                }
            }
        }
    }
}
