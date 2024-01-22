use lapin::{
    options::QueueDeclareOptions,
    types::{AMQPValue, FieldTable},
    Channel, Queue,
};
use serde::{Deserialize, Serialize};
use std::time::Duration;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Job {
    /// List of packages to build
    pub packages: Vec<String>,
    /// Git reference, e.g. branch name
    pub git_ref: String,
    /// SHA hash of the commit point by `git_ref`
    pub sha: String,
    /// Architecture to build
    pub arch: String,
    /// From where this job was triggered, and response should be posted. Note
    /// that it is possible to trigger PR build from Telegram, where source is
    /// JobSource::Telegram, and github_pr is not None
    pub source: JobSource,
    /// Associated GitHub PR
    pub github_pr: Option<u64>,
    /// If built for `noarch` packages
    pub noarch: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum JobSource {
    /// Telegram user/group
    Telegram(i64),
    /// GitHub PR Number
    Github(u64),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum JobResult {
    Ok(JobOk),
    Error(JobError),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobOk {
    /// Original job description
    pub job: Job,
    /// List of packages successfully built
    pub successful_packages: Vec<String>,
    /// List of packages failed to build
    pub failed_package: Option<String>,
    /// List of packages skipped
    pub skipped_packages: Vec<String>,
    /// URL to build log
    pub log: Option<String>,
    /// The identifier of worker
    pub worker: WorkerIdentifier,
    /// Elapsed time of the job
    pub elapsed: Duration,
    /// SHA hash of the commit of ABBS tree
    pub git_commit: Option<String>,
    /// If pushpkg succeeded
    pub pushpkg_success: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobError {
    /// Original job description
    pub job: Job,
    /// The identifier of worker
    pub worker: WorkerIdentifier,
    /// Error message
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
    /// The git commit of buildit
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
