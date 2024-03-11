use lapin::{
    options::QueueDeclareOptions,
    types::{AMQPValue, FieldTable},
    Channel, Queue,
};
use serde::{Deserialize, Serialize};
use std::time::Duration;

#[derive(Serialize, Deserialize)]
pub struct WorkerPollRequest {
    pub hostname: String,
    pub arch: String,
}

#[derive(Serialize, Deserialize)]
pub struct WorkerPollResponse {
    pub job_id: i32,
    pub git_branch: String,
    pub git_sha: String,
    pub packages: String,
}

#[derive(Serialize, Deserialize)]
pub struct WorkerHeartbeatRequest {
    pub hostname: String,
    pub arch: String,
    pub git_commit: String,
    pub memory_bytes: i64,
    pub logical_cores: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum JobResult {
    Ok(JobOk),
    Error(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobOk {
    /// Is the build successful?
    pub build_success: bool,
    /// List of packages successfully built
    pub successful_packages: Vec<String>,
    /// List of packages failed to build
    pub failed_package: Option<String>,
    /// List of packages skipped
    pub skipped_packages: Vec<String>,
    /// URL to build log
    pub log_url: Option<String>,
    /// Elapsed time of the job
    pub elapsed_secs: i64,
    /// If pushpkg succeeded
    pub pushpkg_success: bool,
}

#[derive(Deserialize)]
pub struct WorkerJobUpdateRequest {
    pub hostname: String,
    pub arch: String,
    pub job_id: i32,
    pub result: JobResult,
}