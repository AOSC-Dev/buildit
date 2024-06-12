use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
pub struct WorkerPollRequest {
    pub hostname: String,
    pub arch: String,
    pub worker_secret: String,
    pub memory_bytes: i64,
    pub logical_cores: i32,
    pub disk_free_space_bytes: i64,
}

#[derive(Serialize, Deserialize, Debug)]
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
    pub disk_free_space_bytes: i64,
    pub worker_secret: String,
    pub performance: Option<i64>,
    pub internet_connectivity: Option<bool>,
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

#[derive(Serialize, Deserialize, Debug)]
pub struct WorkerJobUpdateRequest {
    pub hostname: String,
    pub arch: String,
    pub job_id: i32,
    pub result: JobResult,
    pub worker_secret: String,
}
