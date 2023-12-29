use crate::{WorkerStatus, WORKERS};
use chrono::Local;
use common::{ensure_job_queue, WorkerHeartbeat};
use futures::StreamExt;
use lapin::{
    options::{BasicAckOptions, BasicConsumeOptions},
    types::FieldTable,
    ConnectionProperties,
};
use log::{error, info, warn};
use std::time::Duration;

pub async fn heartbeat_worker_inner(amqp_addr: String) -> anyhow::Result<()> {
    let conn = lapin::Connection::connect(&amqp_addr, ConnectionProperties::default()).await?;

    let channel = conn.create_channel().await?;
    let queue_name = "worker-heartbeat";
    ensure_job_queue(queue_name, &channel).await?;

    let mut consumer = channel
        .basic_consume(
            queue_name,
            "worker-heartbeat",
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

        if let Ok(heartbeat) = serde_json::from_slice::<WorkerHeartbeat>(&delivery.data) {
            info!("Processing worker heartbeat {:?} ...", heartbeat);

            // update worker status
            if let Ok(mut lock) = WORKERS.lock() {
                if let Some(status) = lock.get_mut(&heartbeat.identifier) {
                    status.last_heartbeat = Local::now();
                    status.git_commit = heartbeat.git_commit;
                } else {
                    lock.insert(
                        heartbeat.identifier.clone(),
                        WorkerStatus {
                            last_heartbeat: Local::now(),
                            git_commit: heartbeat.git_commit,
                        },
                    );
                }
            }

            // finish
            if let Err(err) = delivery.ack(BasicAckOptions::default()).await {
                warn!("Failed to ack heartbeat {:?}, error: {:?}", delivery, err);
            } else {
                info!("Finished ack-ing heartbeat {:?}", delivery.delivery_tag);
            }
        }
    }

    Ok(())
}

pub async fn heartbeat_worker(amqp_addr: String) -> anyhow::Result<()> {
    loop {
        info!("Starting heartbeat worker ...");
        if let Err(err) = heartbeat_worker_inner(amqp_addr.clone()).await {
            error!("Got error while starting heartbeat worker: {}", err);
        }
        tokio::time::sleep(Duration::from_secs(5)).await;
    }
}
