use crate::{
    models::{NewJob, NewPipeline, Pipeline},
    ALL_ARCH, ARGS,
};
use anyhow::Context;
use buildit_utils::github::update_abbs;
use diesel::{
    r2d2::{ConnectionManager, Pool},
    PgConnection, RunQueryDsl, SelectableHelper,
};

pub async fn pipeline_new(
    pool: Pool<ConnectionManager<PgConnection>>,
    git_branch: &str,
    packages: &str,
    archs: &str,
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
    let new_pipeline = NewPipeline {
        packages: packages.to_string(),
        archs: archs.join(","),
        git_branch: git_branch.to_string(),
        git_sha: git_sha.clone(),
        creation_time: chrono::Utc::now(),
    };
    let pipeline = diesel::insert_into(pipelines::table)
        .values(&new_pipeline)
        .returning(Pipeline::as_returning())
        .get_result(&mut conn)
        .context("Failed to create pipeline")?;

    // for each arch, create a new job
    for arch in &archs {
        use crate::schema::jobs;
        let new_job = NewJob {
            pipeline_id: pipeline.id,
            packages: packages.to_string(),
            arch: arch.to_string(),
            creation_time: chrono::Utc::now(),
            status: "created".to_string(),
        };
        diesel::insert_into(jobs::table)
            .values(&new_job)
            .execute(&mut conn)
            .context("Failed to create job")?;
    }

    Ok(pipeline.id)
}
