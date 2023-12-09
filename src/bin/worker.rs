use buildit::{ensure_job_queue, Job, JobResult, WorkerHeartbeat};
use chrono::Local;
use clap::Parser;
use futures::StreamExt;
use lapin::{
    options::{BasicAckOptions, BasicConsumeOptions, BasicNackOptions, BasicPublishOptions},
    types::FieldTable,
    BasicProperties, ConnectionProperties,
};
use log::{error, info, warn};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    process::Output,
    time::{Duration, Instant},
};
use tokio::process::Command;

#[derive(Parser, Debug, Clone)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// AMQP address to access message queue
    #[arg(short, long)]
    amqp_addr: String,

    /// Architecture that can build
    #[arg(short = 'A', long)]
    arch: String,

    /// Path to ciel workspace
    #[arg(short, long)]
    ciel_path: PathBuf,

    /// Ciel instant name
    #[arg(short = 'I', long, default_value = "main")]
    ciel_instance: String,
}

async fn get_output_logged(
    cmd: &str,
    args: &[&str],
    cwd: &Path,
    logs: &mut Vec<u8>,
) -> anyhow::Result<Output> {
    let begin = Instant::now();
    logs.extend(format!("{}: Running `{} {}`\n", Local::now(), cmd, args.join(" ")).as_bytes());

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

    // switch to git ref
    let mut logs = vec![];
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
        let output = get_output_logged(
            "git",
            &["reset", "FETCH_HEAD", "--hard"],
            &tree_path,
            &mut logs,
        )
        .await?;

        if output.status.success() {
            // update container
            get_output_logged("sudo", &["ciel", "update-os"], &args.ciel_path, &mut logs).await?;

            // build packages
            let mut sudo_args = vec!["ciel", "build", "-i", &args.ciel_instance];
            sudo_args.extend(job.packages.iter().map(String::as_str));
            let output = get_output_logged("sudo", &sudo_args, &args.ciel_path, &mut logs).await?;

            // parse output
            let mut found_build_summary = false;
            for line in String::from_utf8_lossy(&output.stdout).lines() {
                if !found_build_summary && line.contains("--- Build Summary ---") {
                    found_build_summary = true;
                } else if found_build_summary && line.is_empty() {
                    found_build_summary = false;
                } else if found_build_summary {
                    // e.g. bash (amd64 @ 5.2.15-0)
                    if let Some(package_name) = line.split(" ").next() {
                        successful_packages.push(package_name.to_string());
                    }
                }
            }

            // find the first package not in successful_packages
            for package in &job.packages {
                if !successful_packages.contains(package) {
                    failed_package = Some(package.clone());
                    break;
                }
            }
        }
    }

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

    let result = JobResult {
        job: job.clone(),
        successful_packages,
        failed_package,
        log: log_url.map(String::from),
        worker_hostname: gethostname::gethostname().to_string_lossy().to_string(),
        elapsed: begin.elapsed(),
    };
    Ok(result)
}

async fn worker(args: &Args) -> anyhow::Result<()> {
    let mut tree_path = args.ciel_path.clone();
    tree_path.push("TREE");

    let conn = lapin::Connection::connect(&args.amqp_addr, ConnectionProperties::default()).await?;

    let channel = conn.create_channel().await?;
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

async fn heartbeat_worker_inner(args: &Args) -> anyhow::Result<()> {
    let conn = lapin::Connection::connect(&args.amqp_addr, ConnectionProperties::default()).await?;

    let channel = conn.create_channel().await?;
    let queue_name = "worker-heartbeat";
    ensure_job_queue(&queue_name, &channel).await?;

    loop {
        channel
            .basic_publish(
                "",
                "worker-heartbeat",
                BasicPublishOptions::default(),
                &serde_json::to_vec(&WorkerHeartbeat {
                    worker_hostname: gethostname::gethostname().to_string_lossy().to_string(),
                })
                .unwrap(),
                BasicProperties::default(),
            )
            .await?
            .await?;
        tokio::time::sleep(Duration::from_secs(3600)).await;
    }
}

async fn heartbeat_worker(args: Args) -> anyhow::Result<()> {
    loop {
        if let Err(err) = heartbeat_worker_inner(&args).await {
            warn!("Got error running heartbeat worker: {}", err);
        }
        tokio::time::sleep(Duration::from_secs(5)).await;
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenv::dotenv().ok();
    env_logger::init();
    let args = Args::parse();
    info!("Starting AOSC BuildIt! worker");

    tokio::spawn(heartbeat_worker(args.clone()));

    loop {
        if let Err(err) = worker(&args).await {
            warn!("Got error running worker: {}", err);
        }
        tokio::time::sleep(Duration::from_secs(5)).await;
    }
}
