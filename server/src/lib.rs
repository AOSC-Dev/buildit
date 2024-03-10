use chrono::{DateTime, Local};
use clap::Parser;
use common::WorkerIdentifier;
use once_cell::sync::Lazy;
use std::{
    collections::BTreeMap,
    path::PathBuf,
    sync::{Arc, Mutex},
};

pub mod bot;
pub mod formatter;
pub mod github;
pub mod github_webhooks;
pub mod heartbeat;
pub mod job;
pub mod models;
pub mod schema;

pub struct WorkerStatus {
    pub last_heartbeat: DateTime<Local>,
    pub git_commit: Option<String>,
    pub logical_cores: u64,
    pub memory_bytes: u64,
}

pub static WORKERS: Lazy<Arc<Mutex<BTreeMap<WorkerIdentifier, WorkerStatus>>>> =
    Lazy::new(|| Arc::new(Mutex::new(BTreeMap::new())));

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
pub struct Args {
    /// AMQP address to access message queue
    #[arg(env = "BUILDIT_AMQP_ADDR")]
    pub amqp_addr: String,

    /// Database connection url
    #[arg(env = "DATABASE_URL")]
    pub database_url: String,

    /// GitHub access token
    #[arg(env = "BUILDIT_GITHUB_ACCESS_TOKEN")]
    pub github_access_token: String,

    #[arg(env = "ABBS_PATH")]
    pub abbs_path: PathBuf,

    /// RabbitMQ address to access queue api e.g. http://user:password@host:port/api/queues/vhost/
    #[arg(env = "BUILDIT_RABBITMQ_QUEUE_API")]
    pub rabbitmq_queue_api: Option<String>,

    /// Secret
    #[arg(env = "SECRET")]
    pub secret: Option<String>,

    #[arg(env = "GITHUB_APP_ID")]
    pub github_app_id: Option<String>,

    #[arg(env = "GITHUB_APP_KEY_PEM_PATH")]
    pub github_app_key: Option<PathBuf>,
}

pub static ARGS: Lazy<Args> = Lazy::new(Args::parse);

// follow https://github.com/AOSC-Dev/autobuild3/blob/master/sets/arch_groups/mainline
pub(crate) const ALL_ARCH: &[&str] = &[
    "amd64",
    "arm64",
    "loongarch64",
    "loongson3",
    "mips64r6el",
    "ppc64el",
    "riscv64",
];
