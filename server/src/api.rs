use crate::{
    job::get_crab_github_installation,
    models::{NewJob, NewPipeline, Pipeline},
    ALL_ARCH, ARGS,
};
use anyhow::Context;
use buildit_utils::github::update_abbs;
use common::JobSource;
use diesel::{
    r2d2::{ConnectionManager, Pool},
    PgConnection, RunQueryDsl, SelectableHelper,
};
use tracing::warn;

pub async fn pipeline_new(
    pool: Pool<ConnectionManager<PgConnection>>,
    git_branch: &str,
    packages: &str,
    archs: &str,
    source: &JobSource,
) -> anyhow::Result<i32> {
    // resolve branch name to commit hash
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
    let git_sha = String::from_utf8_lossy(&output.stdout).trim().to_string();

    // sanitize archs arg
    let mut archs: Vec<&str> = archs.split(",").collect();
    if archs.contains(&"mainline") {
        // archs
        archs.extend(ALL_ARCH.iter());
        archs.retain(|arch| *arch != "mainline");
    }
    archs.sort();
    archs.dedup();

    // create a new pipeline
    let mut conn = pool
        .get()
        .context("Failed to get db connection from pool")?;
    use crate::schema::pipelines;
    let (source, github_pr, telegram_user) = match source {
        JobSource::Telegram(id) => ("telegram", None, Some(id)),
        JobSource::Github(id) => ("github", Some(id), None),
        JobSource::Manual => ("manual", None, None),
    };
    let new_pipeline = NewPipeline {
        packages: packages.to_string(),
        archs: archs.join(","),
        git_branch: git_branch.to_string(),
        git_sha: git_sha.clone(),
        creation_time: chrono::Utc::now(),
        source: source.to_string(),
        github_pr: github_pr.map(|id| *id as i64),
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

    Ok(pipeline.id)
}
