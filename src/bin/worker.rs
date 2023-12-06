use buildit::{Job, JobResult};
use clap::Parser;
use futures::StreamExt;
use lapin::{
    options::{
        BasicAckOptions, BasicConsumeOptions, BasicNackOptions, BasicPublishOptions,
        QueueDeclareOptions,
    },
    types::FieldTable,
    BasicProperties, ConnectionProperties,
};
use log::{error, info, warn};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    process::Command,
    time::Duration,
};

#[derive(Parser, Debug)]
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

async fn build(job: &Job, tree_path: &Path, args: &Args) -> anyhow::Result<JobResult> {
    // switch to git ref
    let mut logs = vec![];
    let output = Command::new("git")
        .args([
            "fetch",
            "https://github.com/AOSC-Dev/aosc-os-abbs.git",
            &job.git_ref,
        ])
        .current_dir(&tree_path)
        .output()?;
    logs.extend(format!("Git fetch exited with status {}:\n", output.status).as_bytes());
    logs.extend("STDOUT:\n".as_bytes());
    logs.extend(output.stdout);
    logs.extend("STDERR:\n".as_bytes());
    logs.extend(output.stderr);

    if output.status.success() {
        let output = Command::new("git")
            .args(["reset", "FETCH_HEAD", "--hard"])
            .current_dir(&tree_path)
            .output()?;
        logs.extend(format!("Git reset exited with status {}:\n", output.status).as_bytes());
        logs.extend("STDOUT:\n".as_bytes());
        logs.extend(output.stdout);
        logs.extend("STDERR:\n".as_bytes());
        logs.extend(output.stderr);

        if output.status.success() {
            let output = Command::new("sudo")
                .args(["ciel", "update-os"])
                .current_dir(&args.ciel_path)
                .output()?;
            logs.extend(format!("Ciel exited with status {}:\n", output.status).as_bytes());
            logs.extend("STDOUT:\n".as_bytes());
            logs.extend(output.stdout);
            logs.extend("STDERR:\n".as_bytes());
            logs.extend(output.stderr);

            let output = Command::new("sudo")
                .args(["ciel", "build", "-i", &args.ciel_instance])
                .args(&job.packages)
                .current_dir(&args.ciel_path)
                .output()?;
            logs.extend(format!("Ciel exited with status {}:\n", output.status).as_bytes());
            logs.extend("STDOUT:\n".as_bytes());
            logs.extend(output.stdout);
            logs.extend("STDERR:\n".as_bytes());
            logs.extend(output.stderr);
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
        sucessful_packages: job.packages.clone(),
        arch: job.arch.clone(),
        git_ref: job.git_ref.clone(),
        failed_package: None,
        log: log_url.map(String::from),
        tg_chatid: job.tg_chatid,
    };
    Ok(result)
}

async fn worker(args: &Args) -> anyhow::Result<()> {
    let mut tree_path = args.ciel_path.clone();
    tree_path.push("TREE");

    let conn = lapin::Connection::connect(&args.amqp_addr, ConnectionProperties::default()).await?;

    let channel = conn.create_channel().await?;
    let queue_name = format!("job-{}", &args.arch);
    let _queue = channel
        .queue_declare(
            &queue_name,
            QueueDeclareOptions {
                durable: true,
                ..QueueDeclareOptions::default()
            },
            FieldTable::default(),
        )
        .await?;

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
                            "job_completion",
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

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    env_logger::init();
    let args = Args::parse();
    info!("Starting AOSC BuildIt! worker");

    loop {
        if let Err(err) = worker(&args).await {
            warn!("Got error running worker: {}", err);
        }
        tokio::time::sleep(Duration::from_secs(5)).await;
    }
}
