use crate::{ensure_channel, Args};
use chrono::Local;
use common::{ensure_job_queue, Job, JobResult, WorkerIdentifier};
use futures::StreamExt;
use lapin::{
    options::{BasicAckOptions, BasicConsumeOptions, BasicNackOptions, BasicPublishOptions},
    types::FieldTable,
    BasicProperties,
};
use log::{error, info, warn};
use std::{
    collections::HashMap,
    path::Path,
    process::Output,
    time::{Duration, Instant},
};
use tokio::process::Command;

async fn get_output_logged(
    cmd: &str,
    args: &[&str],
    cwd: &Path,
    logs: &mut Vec<u8>,
) -> anyhow::Result<Output> {
    let begin = Instant::now();
    let msg = format!("{}: Running `{} {}`\n", Local::now(), cmd, args.join(" "));
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

async fn build(job: &Job, tree_path: &Path, args: &Args) -> anyhow::Result<JobResult> {
    let begin = Instant::now();
    let mut successful_packages = vec![];
    let mut failed_package = None;
    let mut skipped_packages = vec![];
    let mut git_commit = None;
    let mut logs = vec![];

    // assuming branch name == git_ref
    let mut output_path = args.ciel_path.clone();
    output_path.push(format!("OUTPUT-{}", job.git_ref));

    // clear output directory
    if output_path.exists() {
        get_output_logged("rm", &["-rf", "debs"], &output_path, &mut logs).await?;
    }

    // switch to git ref
    let output = get_output_logged(
        "git",
        &[
            "fetch",
            "https://github.com/AOSC-Dev/aosc-os-abbs.git",
            &job.git_ref,
        ],
        &tree_path,
        &mut logs,
    )
    .await?;

    if output.status.success() {
        let output =
            get_output_logged("git", &["rev-parse", "FETCH_HEAD"], &tree_path, &mut logs).await?;
        git_commit = Some(String::from_utf8_lossy(&output.stdout).to_string());

        // try to switch branch, but allow it to fail:
        // ensure branch exists
        get_output_logged(
            "git",
            &["checkout", "-b", &job.git_ref],
            &tree_path,
            &mut logs,
        )
        .await?;
        // checkout to branch
        get_output_logged("git", &["checkout", &job.git_ref], &tree_path, &mut logs).await?;

        let output = get_output_logged(
            "git",
            &["reset", "FETCH_HEAD", "--hard"],
            &tree_path,
            &mut logs,
        )
        .await?;

        if output.status.success() {
            // update container
            get_output_logged("ciel", &["update-os"], &args.ciel_path, &mut logs).await?;

            // build packages
            let mut ciel_args = vec!["build", "-i", &args.ciel_instance];
            ciel_args.extend(job.packages.iter().map(String::as_str));
            let output = get_output_logged("ciel", &ciel_args, &args.ciel_path, &mut logs).await?;

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
                    } else if line.contains("(") {
                        // e.g. bash (amd64 @ 5.2.15-0)
                        if let Some(package_name) = line.split(" ").next() {
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
        }
    }

    // upload to repo if succeeded
    if let Some(upload_ssh_key) = &args.upload_ssh_key {
        if failed_package.is_none() {
            get_output_logged(
                "pushpkg",
                &["-i", &upload_ssh_key, "maintainers", &job.git_ref],
                &output_path,
                &mut logs,
            )
            .await?;
        }
    }

    // update logs to pastebin
    let mut map = HashMap::new();
    map.insert("contents", String::from_utf8_lossy(&logs).to_string());
    map.insert("language", "log".to_string());

    let client = reqwest::Client::new();
    let res = client
        .post("https://pastebin.aosc.io/api/paste/submit")
        .json(&map)
        .send()
        .await?
        .json::<serde_json::Value>()
        .await?;
    let log_url = res
        .as_object()
        .and_then(|m| m.get("url"))
        .and_then(|v| v.as_str());

    let result = JobResult::Ok {
        job: job.clone(),
        successful_packages,
        failed_package,
        skipped_packages,
        log: log_url.map(String::from),
        worker: WorkerIdentifier {
            hostname: gethostname::gethostname().to_string_lossy().to_string(),
            arch: args.arch.clone(),
            pid: std::process::id(),
        },
        elapsed: begin.elapsed(),
        git_commit,
    };

    Ok(result)
}

async fn build_worker_inner(args: &Args) -> anyhow::Result<()> {
    let mut tree_path = args.ciel_path.clone();
    tree_path.push("TREE");

    let channel = ensure_channel(args).await?;
    let queue_name = format!("job-{}", &args.arch);
    ensure_job_queue(&queue_name, &channel).await?;

    let mut consumer = channel
        .basic_consume(
            &queue_name,
            "worker",
            BasicConsumeOptions::default(),
            FieldTable::default(),
        )
        .await?;

    while let Some(delivery) = consumer.next().await {
        let delivery = match delivery {
            Ok(delivery) => delivery,
            Err(err) => {
                error!("Got error in lapin delivery: {}", err);
                continue;
            }
        };

        if let Some(job) = serde_json::from_slice::<Job>(&delivery.data).ok() {
            info!("Processing job {:?}", job);

            match build(&job, &tree_path, &args).await {
                Ok(result) => {
                    channel
                        .basic_publish(
                            "",
                            "job-completion",
                            BasicPublishOptions::default(),
                            &serde_json::to_vec(&result).unwrap(),
                            BasicProperties::default(),
                        )
                        .await?
                        .await?;

                    // finish
                    if let Err(err) = delivery.ack(BasicAckOptions::default()).await {
                        warn!("Failed to ack job {:?} with err {:?}", delivery, err);
                    } else {
                        info!("Finish ack-ing job {:?}", delivery.delivery_tag);
                    }
                }
                Err(err) => {
                    warn!("Failed to run job {:?} with err {:?}", delivery, err);

                    channel
                        .basic_publish(
                            "",
                            "job-completion",
                            BasicPublishOptions::default(),
                            &serde_json::to_vec(&JobResult::Error {
                                job,
                                worker: WorkerIdentifier {
                                    hostname: gethostname::gethostname()
                                        .to_string_lossy()
                                        .to_string(),
                                    arch: args.arch.clone(),
                                    pid: std::process::id(),
                                },
                                error: err.to_string(),
                            })
                            .unwrap(),
                            BasicProperties::default(),
                        )
                        .await?
                        .await?;

                    // finish
                    if let Err(err) = delivery.nack(BasicNackOptions::default()).await {
                        warn!("Failed to nack job {:?} with err {:?}", delivery, err);
                    } else {
                        info!("Finish nack-ing job {:?}", delivery.delivery_tag);
                    }
                }
            }
        }
    }
    Ok(())
}

pub async fn build_worker(args: Args) -> ! {
    loop {
        if let Err(err) = build_worker_inner(&args).await {
            warn!("Got error running heartbeat worker: {}", err);
        }
        tokio::time::sleep(Duration::from_secs(5)).await;
    }
}