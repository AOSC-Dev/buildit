use crate::{ensure_channel, Args};
use common::{ensure_job_queue, WorkerHeartbeat, WorkerIdentifier};
use lapin::{options::BasicPublishOptions, BasicProperties};
use log::warn;
use std::time::Duration;

pub async fn heartbeat_worker_inner(args: &Args) -> anyhow::Result<()> {
    let channel = ensure_channel(args).await?;
    let queue_name = "worker-heartbeat";
    ensure_job_queue(queue_name, &channel).await?;

    loop {
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
                    git_commit: option_env!("VERGEN_GIT_SHA").map(String::from),
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
    loop {
        if let Err(err) = heartbeat_worker_inner(&args).await {
            warn!("Got error running heartbeat worker: {}", err);
        }
        tokio::time::sleep(Duration::from_secs(5)).await;
    }
}
