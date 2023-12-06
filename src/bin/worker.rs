use buildit::{Job, JobResult};
use clap::Parser;
use futures::StreamExt;
use lapin::{
    options::{BasicAckOptions, BasicConsumeOptions, BasicPublishOptions, QueueDeclareOptions},
    types::FieldTable,
    BasicProperties, ConnectionProperties,
};
use log::{error, info, warn};
use std::{path::PathBuf, process::Command};

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

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    env_logger::init();
    let args = Args::parse();
    info!("Starting AOSC BuildIt! worker");

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

            // switch to git ref
            let _status = Command::new("git")
                .args([
                    "fetch",
                    "https://github.com/AOSC-Dev/aosc-os-abbs.git",
                    &job.git_ref,
                ])
                .current_dir(&tree_path)
                .status()
                .unwrap();

            let _status = Command::new("git")
                .args([
                    "fetch",
                    "https://github.com/AOSC-Dev/aosc-os-abbs.git",
                    &job.git_ref,
                ])
                .current_dir(&tree_path)
                .status()
                .unwrap();

            let _status = Command::new("sudo")
                .args(["ciel", "build", "-i", &args.ciel_instance])
                .args(&job.packages)
                .current_dir(&args.ciel_path)
                .status()
                .unwrap();

            let result = JobResult {
                sucessful_packages: job.packages,
                arch: job.arch,
                git_ref: job.git_ref,
                failed_package: None,
                failure_log: None,
                tg_chatid: job.tg_chatid,
            };

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
        }

        // finish
        if let Err(err) = delivery.ack(BasicAckOptions::default()).await {
            warn!("Failed to delete job {:?} with err {:?}", delivery, err);
        } else {
            info!("Finish processing job {:?}", delivery.delivery_tag);
        }
    }
    Ok(())
}
