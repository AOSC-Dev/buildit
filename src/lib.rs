use lapin::{
    options::QueueDeclareOptions,
    types::{AMQPValue, FieldTable},
    Channel, Queue,
};

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
    pub job: Job,
    pub successful_packages: Vec<String>,
    pub failed_package: Option<String>,
    pub log: Option<String>,
    pub worker_hostname: String,
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
