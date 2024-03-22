use crate::Args;
use common::WorkerHeartbeatRequest;
use log::{info, warn};
use std::time::Duration;

pub async fn heartbeat_worker_inner(args: &Args) -> anyhow::Result<()> {
    let client = reqwest::Client::new();
    loop {
        // info!("Sending heartbeat");
        client
            .post(format!("{}/api/worker/heartbeat", args.server))
            .json(&WorkerHeartbeatRequest {
                hostname: gethostname::gethostname().to_string_lossy().to_string(),
                arch: args.arch.clone(),
                worker_secret: args.worker_secret.clone(),
                git_commit: env!("VERGEN_GIT_DESCRIBE").to_string(),
                memory_bytes: sysinfo::System::new_all().total_memory() as i64,
                disk_free_space_bytes: fs2::free_space(std::env::current_dir()?)? as i64,
                logical_cores: num_cpus::get() as i32,
                performance: args.worker_performance,
            })
            .send()
            .await?;
        tokio::time::sleep(Duration::from_secs(60)).await;
    }
}

pub async fn heartbeat_worker(args: Args) -> ! {
    loop {
        info!("Starting heartbeat worker");
        if let Err(err) = heartbeat_worker_inner(&args).await {
            warn!("Got error running heartbeat worker: {}", err);
        }
        tokio::time::sleep(Duration::from_secs(5)).await;
    }
}
