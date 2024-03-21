use crate::Args;
use chrono::Local;
use common::{JobOk, WorkerJobUpdateRequest, WorkerPollRequest, WorkerPollResponse};
use log::{error, info, warn};
use std::{
    path::Path,
    process::Output,
    time::{Duration, Instant},
};
use tokio::{fs, process::Command, time::sleep};

async fn get_output_logged(
    cmd: &str,
    args: &[&str],
    cwd: &Path,
    logs: &mut Vec<u8>,
) -> anyhow::Result<Output> {
    let begin = Instant::now();
    let msg = format!(
        "{}: Running `{} {}` in `{}`\n",
        Local::now(),
        cmd,
        args.join(" "),
        cwd.display()
    );
    logs.extend(msg.as_bytes());
    info!("{}", msg.trim());

    let output = Command::new(cmd)
        .args(args)
        .current_dir(cwd)
        .output()
        .await?;

    let elapsed = begin.elapsed();
    logs.extend(
        format!(
            "{}: `{} {}` finished in {:?} with {}\n",
            Local::now(),
            cmd,
            args.join(" "),
            elapsed,
            output.status
        )
        .as_bytes(),
    );
    logs.extend("STDOUT:\n".as_bytes());
    logs.extend(output.stdout.clone());
    logs.extend("STDERR:\n".as_bytes());
    logs.extend(output.stderr.clone());

    Ok(output)
}

/// Run command and retry until it succeeds
async fn run_logged_with_retry(
    cmd: &str,
    args: &[&str],
    cwd: &Path,
    logs: &mut Vec<u8>,
) -> anyhow::Result<bool> {
    for i in 0..5 {
        if i > 0 {
            info!("Attempt #{i} to run `{cmd} {}`", args.join(" "));
        }
        match get_output_logged(cmd, args, cwd, logs).await {
            Ok(output) => {
                if output.status.success() {
                    return Ok(true);
                } else {
                    warn!(
                        "Running `{cmd} {}` exited with {}",
                        args.join(" "),
                        output.status
                    );
                }
            }
            Err(err) => {
                warn!("Running `{cmd} {}` failed with {err}", args.join(" "));
            }
        }
        // exponential backoff
        sleep(Duration::from_secs(1 << i)).await;
    }
    warn!("Failed too many times running `{cmd} {}`", args.join(" "));
    Ok(false)
}

async fn build(
    job: &WorkerPollResponse,
    tree_path: &Path,
    args: &Args,
) -> anyhow::Result<WorkerJobUpdateRequest> {
    let begin = Instant::now();
    let mut successful_packages = vec![];
    let mut failed_package = None;
    let mut skipped_packages = vec![];
    let mut build_success = false;
    let mut logs = vec![];

    let mut output_path = args.ciel_path.clone();
    output_path.push(format!("OUTPUT-{}", job.git_branch));

    // clear output directory
    if output_path.exists() {
        get_output_logged("rm", &["-rf", "debs"], &output_path, &mut logs).await?;
    }

    // switch to git ref
    let git_fetch_succeess = run_logged_with_retry(
        "git",
        &[
            "fetch",
            "https://github.com/AOSC-Dev/aosc-os-abbs.git",
            &job.git_branch,
        ],
        tree_path,
        &mut logs,
    )
    .await?;

    let mut pushpkg_success = false;

    if git_fetch_succeess {
        // try to switch branch, but allow it to fail:
        // ensure branch exists
        get_output_logged(
            "git",
            &["checkout", "-b", &job.git_branch],
            tree_path,
            &mut logs,
        )
        .await?;
        // checkout to branch
        get_output_logged("git", &["checkout", &job.git_branch], tree_path, &mut logs).await?;

        // switch to the commit by sha
        // to avoid race condition, resolve branch name to sha in server
        let output = get_output_logged(
            "git",
            &["reset", &job.git_sha, "--hard"],
            tree_path,
            &mut logs,
        )
        .await?;

        if output.status.success() {
            // update container
            get_output_logged("ciel", &["update-os"], &args.ciel_path, &mut logs).await?;

            // build packages
            let mut ciel_args = vec!["build", "-i", &args.ciel_instance];
            ciel_args.extend(job.packages.split(','));
            let output = get_output_logged("ciel", &ciel_args, &args.ciel_path, &mut logs).await?;

            build_success = output.status.success();

            // parse output
            // match acbs/acbs/util.py
            let mut found_banner = false;
            let mut found_acbs_build = false;
            let mut found_failed_package = false;
            let mut found_packages_built = false;
            let mut found_packages_not_built = false;

            for line in String::from_utf8_lossy(&output.stdout).lines() {
                if line.contains("========================================") {
                    found_banner = true;
                } else if line.contains("ACBS Build") {
                    found_acbs_build = true;
                } else if found_banner && found_acbs_build {
                    if line.starts_with("Failed package:") {
                        found_failed_package = true;
                        found_packages_built = false;
                        found_packages_not_built = false;
                    } else if line.starts_with("Package(s) built:") {
                        found_failed_package = false;
                        found_packages_built = true;
                        found_packages_not_built = false;
                    } else if line
                        .starts_with("Package(s) not built due to previous build failure:")
                    {
                        found_failed_package = false;
                        found_packages_built = false;
                        found_packages_not_built = true;
                    } else if line.contains('(') {
                        // e.g. bash (amd64 @ 5.2.15-0)
                        if let Some(package_name) = line.split(' ').next() {
                            if found_packages_built {
                                successful_packages.push(package_name.to_string());
                            } else if found_failed_package {
                                failed_package = Some(package_name.to_string());
                            } else if found_packages_not_built {
                                skipped_packages.push(package_name.to_string());
                            }
                        }
                    } else if line.is_empty() {
                        found_failed_package = false;
                        found_packages_built = false;
                        found_packages_not_built = false;
                    }
                }
            }

            if build_success {
                if let Some(upload_ssh_key) = &args.upload_ssh_key {
                    pushpkg_success = run_logged_with_retry(
                        "pushpkg",
                        &[
                            "--host",
                            &args.rsync_host,
                            "-i",
                            upload_ssh_key,
                            "maintainers",
                            &job.git_branch,
                        ],
                        &output_path,
                        &mut logs,
                    )
                    .await?;
                }
            }
        }
    }

    let file_name = format!(
        "{}-{}-{}-{}-{}.txt",
        job.job_id,
        job.git_branch,
        args.arch,
        gethostname::gethostname().to_string_lossy(),
        Local::now().format("%Y-%m-%d-%H:%M:%S")
    );

    let path = format!("/tmp/{file_name}");
    fs::write(&path, logs).await?;

    let mut log_url = None;
    if let Some(upload_ssh_key) = &args.upload_ssh_key {
        let mut scp_log = vec![];
        if run_logged_with_retry(
            "scp",
            &[
                "-i",
                &upload_ssh_key,
                &path,
                &format!("maintainers@{}:/buildit/logs", args.rsync_host),
            ],
            &output_path,
            &mut scp_log,
        )
        .await?
        {
            fs::remove_file(&path).await?;
            log_url = Some(format!("https://buildit.aosc.io/logs/{file_name}"));
        } else {
            error!(
                "Failed to scp log to repo: {}",
                String::from_utf8_lossy(&scp_log)
            );
        };
    }

    if log_url.is_none() {
        let dir = Path::new("./push_failed_logs");
        let to = dir.join(file_name);
        fs::create_dir_all(dir).await?;
        fs::copy(&path, to).await?;
    }

    let result = WorkerJobUpdateRequest {
        hostname: gethostname::gethostname().to_string_lossy().to_string(),
        arch: args.arch.clone(),
        worker_secret: args.worker_secret.clone(),
        job_id: job.job_id,
        result: common::JobResult::Ok(JobOk {
            build_success: build_success,
            successful_packages,
            failed_package,
            skipped_packages,
            log_url,
            elapsed_secs: begin.elapsed().as_secs() as i64,
            pushpkg_success,
        }),
    };

    Ok(result)
}

async fn build_worker_inner(args: &Args) -> anyhow::Result<()> {
    let mut tree_path = args.ciel_path.clone();
    tree_path.push("TREE");

    info!("Receiving new messages");

    let client = reqwest::Client::new();
    let req = WorkerPollRequest {
        hostname: gethostname::gethostname().to_string_lossy().to_string(),
        arch: args.arch.clone(),
        worker_secret: args.worker_secret.clone(),
        memory_bytes: sysinfo::System::new_all().total_memory() as i64,
        disk_free_space_bytes: fs2::free_space(std::env::current_dir()?)? as i64,
        logical_cores: num_cpus::get() as i32,
    };

    loop {
        if let Some(job) = client
            .post(format!("{}/api/worker/poll", args.server))
            .json(&req)
            .send()
            .await?
            .json::<Option<WorkerPollResponse>>()
            .await?
        {
            info!("Processing job {:?}", job);

            match build(&job, &tree_path, args).await {
                Ok(result) => {
                    // post result
                    info!("Finished to run job {:?} with result {:?}", job, result);
                    client
                        .post(format!("{}/api/worker/job_update", args.server))
                        .json(&result)
                        .send()
                        .await?;
                }
                Err(err) => {
                    warn!("Failed to run job {:?} with err {:?}", job, err);
                    client
                        .post(format!("{}/api/worker/job_update", args.server))
                        .json(&WorkerJobUpdateRequest {
                            hostname: gethostname::gethostname().to_string_lossy().to_string(),
                            arch: args.arch.clone(),
                            worker_secret: args.worker_secret.clone(),
                            job_id: job.job_id,
                            result: common::JobResult::Error(err.to_string()),
                        })
                        .send()
                        .await?;
                }
            }
        }
        tokio::time::sleep(Duration::from_secs(5)).await;
    }
}

pub async fn build_worker(args: Args) -> ! {
    loop {
        info!("Starting build worker");
        if let Err(err) = build_worker_inner(&args).await {
            warn!("Got error running heartbeat worker: {}", err);
        }
        tokio::time::sleep(Duration::from_secs(5)).await;
    }
}
