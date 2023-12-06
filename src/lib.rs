use serde::{Deserialize, Serialize};
use teloxide::types::ChatId;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Job {
    pub packages: Vec<String>,
    pub git_ref: String,
    pub arch: String,
    pub tg_chatid: ChatId,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobResult {
    pub sucessful_packages: Vec<String>,
    pub git_ref: String,
    pub arch: String,
    pub failed_package: Option<String>,
    pub failure_log: Option<String>,
    pub tg_chatid: ChatId,
}
