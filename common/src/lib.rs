use lapin::{
    options::QueueDeclareOptions,
    types::{AMQPValue, FieldTable},
    Channel, Queue,
};
use serde::{Deserialize, Serialize};
use std::time::Duration;
use teloxide::types::ChatId;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Job {
    pub packages: Vec<String>,
    pub git_ref: String,
    pub arch: String,
    pub tg_chatid: ChatId,
    pub github_pr: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum JobResult {
    Ok {
        job: Job,
        successful_packages: Vec<String>,
        failed_package: Option<String>,
        skipped_packages: Vec<String>,
        log: Option<String>,
        worker: WorkerIdentifier,
        elapsed: Duration,
        git_commit: Option<String>,
    },
    Error {
        job: Job,
        worker: WorkerIdentifier,
        error: String,
    }
}


#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct WorkerIdentifier {
    // sort by (arch, hostname, pid)
    pub arch: String,
    pub hostname: String,
    pub pid: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkerHeartbeat {
    pub identifier: WorkerIdentifier,
}

pub async fn ensure_job_queue(queue_name: &str, channel: &Channel) -> anyhow::Result<Queue> {
    let mut arguments = FieldTable::default();
    // extend consumer timeout because we may have long running tasks
    arguments.insert(
        "x-consumer-timeout".into(),
        AMQPValue::LongInt(24 * 3600 * 1000),
    );
    Ok(channel
        .queue_declare(
            &queue_name,
            QueueDeclareOptions {
                durable: true,
                ..QueueDeclareOptions::default()
            },
            arguments,
        )
        .await?)
}
