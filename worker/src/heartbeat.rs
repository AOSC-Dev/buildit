use crate::{get_memory_bytes, Args};
use backoff::ExponentialBackoff;
use common::WorkerHeartbeatRequest;
use log::{info, warn};
use std::{
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};

static INTERNET_CONNECTIVITY: AtomicBool = AtomicBool::new(false);

pub async fn internet_connectivity_worker() -> ! {
    info!("Starting internet connectivity worker");
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .unwrap();
    loop {
        let last = INTERNET_CONNECTIVITY.load(Ordering::SeqCst);
        let next = client.get("https://github.com/").send().await.is_ok();
        if last != next {
            info!("Internet connectivity changed from {} to {}", last, next);
        }
        INTERNET_CONNECTIVITY.store(next, Ordering::SeqCst);

        tokio::time::sleep(Duration::from_secs(60)).await;
    }
}

pub async fn heartbeat_worker_inner(args: &Args) -> anyhow::Result<()> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .unwrap();
    loop {
        // info!("Sending heartbeat");
        client
            .post(format!("{}/api/worker/heartbeat", args.server))
            .json(&WorkerHeartbeatRequest {
                hostname: gethostname::gethostname().to_string_lossy().to_string(),
                arch: args.arch.clone(),
                worker_secret: args.worker_secret.clone(),
                git_commit: env!("VERGEN_GIT_DESCRIBE").to_string(),
                memory_bytes: get_memory_bytes(),
                disk_free_space_bytes: fs2::free_space(std::env::current_dir()?)? as i64,
                logical_cores: num_cpus::get() as i32,
                performance: args.worker_performance,
                internet_connectivity: Some(INTERNET_CONNECTIVITY.load(Ordering::SeqCst)),
            })
            .send()
            .await?;
        tokio::time::sleep(Duration::from_secs(60)).await;
    }
}

pub async fn heartbeat_worker(args: Args) -> anyhow::Result<()> {
    tokio::spawn(internet_connectivity_worker());

    backoff::future::retry(ExponentialBackoff::default(), || async {
        warn!("Retry send heartbeat ...");
        Ok(heartbeat_worker_inner(&args).await?)
    })
    .await
}
