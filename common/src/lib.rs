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
    pub noarch: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum JobResult {
    Ok(JobOk),
    Error(JobError),
}


#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobOk {
    pub job: Job,
    pub successful_packages: Vec<String>,
    pub failed_package: Option<String>,
    pub skipped_packages: Vec<String>,
    pub log: Option<String>,
    pub worker: WorkerIdentifier,
    pub elapsed: Duration,
    pub git_commit: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobError {
    pub job: Job,
    pub worker: WorkerIdentifier,
    pub error: String,
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
    pub git_commit: Option<String>,
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
            queue_name,
            QueueDeclareOptions {
                durable: true,
                ..QueueDeclareOptions::default()
            },
            arguments,
        )
        .await?)
}
