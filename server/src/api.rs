use crate::{
    github::get_packages_from_pr,
    job::get_crab_github_installation,
    models::{NewJob, NewPipeline, Pipeline},
    DbPool, ALL_ARCH, ARGS,
};
use anyhow::anyhow;
use anyhow::Context;
use buildit_utils::github::{get_archs, update_abbs};
use common::JobSource;
use diesel::{RunQueryDsl, SelectableHelper};
use tracing::warn;

pub async fn pipeline_new(
    pool: DbPool,
    git_branch: &str,
    git_sha: Option<&str>,
    github_pr: Option<u64>,
    packages: &str,
    archs: &str,
    source: &JobSource,
) -> anyhow::Result<Pipeline> {
    // resolve branch name to commit hash if not specified
    let git_sha = match git_sha {
        Some(git_sha) => git_sha.to_string(),
        None => {
            update_abbs(git_branch, &ARGS.abbs_path)
                .await
                .context("Failed to update ABBS tree")?;

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

    // sanitize archs arg
    let mut archs: Vec<&str> = archs.split(",").collect();
    if archs.contains(&"mainline") {
        // archs
        archs.extend(ALL_ARCH.iter());
        archs.retain(|arch| *arch != "mainline");
    }
    for arch in &archs {
        if !ALL_ARCH.contains(arch) && arch != &"mainline" {
            return Err(anyhow!("Architecture {arch} is not supported"));
        }
    }
    archs.sort();
    archs.dedup();

    // create a new pipeline
    let mut conn = pool
        .get()
        .context("Failed to get db connection from pool")?;
    use crate::schema::pipelines;
    let (source, github_pr, telegram_user) = match source {
        JobSource::Telegram(id) => ("telegram", github_pr, Some(id)),
        JobSource::Github(id) => ("github", Some(*id), None),
        JobSource::Manual => ("manual", github_pr, None),
    };
    let new_pipeline = NewPipeline {
        packages: packages.to_string(),
        archs: archs.join(","),
        git_branch: git_branch.to_string(),
        git_sha: git_sha.clone(),
        creation_time: chrono::Utc::now(),
        source: source.to_string(),
        github_pr: github_pr.map(|pr| pr as i64),
        telegram_user: telegram_user.map(|id| *id),
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

    // for each arch, create a new job
    for arch in &archs {
        // create github check run
        let mut github_check_run_id = None;
        if let Some(crab) = &crab {
            match crab
                .checks("AOSC-Dev", "aosc-os-abbs")
                .create_check_run(format!("buildit {}", arch), &git_sha)
                .status(octocrab::params::checks::CheckRunStatus::InProgress)
                .send()
                .await
            {
                Ok(check_run) => {
                    github_check_run_id = Some(check_run.id.0);
                }
                Err(err) => {
                    warn!("Failed to create check run: {}", err);
                }
            }
        }

        // create a new job
        use crate::schema::jobs;
        let new_job = NewJob {
            pipeline_id: pipeline.id,
            packages: packages.to_string(),
            arch: arch.to_string(),
            creation_time: chrono::Utc::now(),
            status: "created".to_string(),
            github_check_run_id: github_check_run_id.map(|id| id as i64),
        };
        diesel::insert_into(jobs::table)
            .values(&new_job)
            .execute(&mut conn)
            .context("Failed to create job")?;
    }

    Ok(pipeline)
}

pub async fn pipeline_new_pr(
    pool: DbPool,
    pr: u64,
    archs: Option<&str>,
) -> anyhow::Result<Pipeline> {
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

            update_abbs(git_branch, &ARGS.abbs_path)
                .await
                .context("Failed to update ABBS tree")?;

            // find lines starting with #buildit
            let packages = get_packages_from_pr(&pr);
            if !packages.is_empty() {
                let archs = if let Some(archs) = archs {
                    archs.to_string()
                } else {
                    let path = &ARGS.abbs_path;

                    get_archs(path, &packages).join(",")
                };

                pipeline_new(
                    pool,
                    git_branch,
                    Some(&git_sha),
                    Some(pr.number),
                    &packages.join(","),
                    &archs,
                    &JobSource::Github(pr.number),
                )
                .await
            } else {
                return Err(anyhow!(
                    "Please list packages to build in pr info starting with '#buildit'"
                ));
            }
        }
        Err(err) => {
            return Err(anyhow!("Failed to get pr info: {err}"));
        }
    }
}
