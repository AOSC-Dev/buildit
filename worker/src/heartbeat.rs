use crate::Args;
use common::{ensure_job_queue, WorkerHeartbeat, WorkerIdentifier};
use lapin::{options::BasicPublishOptions, BasicProperties, ConnectionProperties};
use log::{info, warn};
use std::time::Duration;

pub async fn heartbeat_worker_inner(args: &Args) -> anyhow::Result<()> {
    let conn = lapin::Connection::connect(&args.amqp_addr, ConnectionProperties::default()).await?;
    let channel = conn.create_channel().await?;
    let queue_name = "worker-heartbeat";
    ensure_job_queue(queue_name, &channel).await?;

    loop {
        info!("Sending heartbeat");
        channel
            .basic_publish(
                "",
                "worker-heartbeat",
                BasicPublishOptions::default(),
                &serde_json::to_vec(&WorkerHeartbeat {
                    identifier: WorkerIdentifier {
                        hostname: gethostname::gethostname().to_string_lossy().to_string(),
                        arch: args.arch.clone(),
                        pid: std::process::id(),
                    },
                    git_commit: option_env!("VERGEN_GIT_DESCRIBE").map(String::from),
                    memory_bytes: sysinfo::System::new_all().total_memory(),
                    logical_cores: num_cpus::get() as u64,
                })
                .unwrap(),
                BasicProperties::default(),
            )
            .await?
            .await?;
        tokio::time::sleep(Duration::from_secs(3600)).await;
    }
}

pub async fn heartbeat_worker(args: Args) -> ! {
    info!("Starting heartbeat worker");
    loop {
        if let Err(err) = heartbeat_worker_inner(&args).await {
            warn!("Got error running heartbeat worker: {}", err);
        }
        tokio::time::sleep(Duration::from_secs(5)).await;
    }
}
