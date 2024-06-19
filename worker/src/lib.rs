use clap::Parser;
use std::path::PathBuf;
use sysinfo::System;

pub mod build;
pub mod heartbeat;

#[derive(Parser, Debug, Clone)]
#[command(author, version, about, long_about = None)]
pub struct Args {
    /// buildit server url e.g. https://buildit.aosc.io
    #[arg(short = 'H', long, env = "BUILDIT_SERVER")]
    pub server: String,

    /// Worker secret
    #[arg(short = 'S', long, env = "BUILDIT_WORKER_SECRET")]
    pub worker_secret: String,

    /// Architecture that can build
    #[arg(short = 'A', long, env = "BUILDIT_ARCH")]
    pub arch: String,

    /// Path to ciel workspace
    #[arg(short, long, env = "BUILDIT_CIEL_PATH")]
    pub ciel_path: PathBuf,

    /// Ciel instance name
    #[arg(
        short = 'I',
        long,
        default_value = "main",
        env = "BUILDIT_CIEL_INSTANCE"
    )]
    pub ciel_instance: String,

    /// SSH key for repo uploading
    #[arg(short = 's', long, env = "BUILDIT_SSH_KEY")]
    pub upload_ssh_key: Option<String>,

    /// rsync host (server)
    #[arg(
        short,
        long,
        default_value = "repo.aosc.io",
        env = "BUILDIT_RSYNC_HOST"
    )]
    pub rsync_host: String,

    /// Performance number of the worker (smaller is better)
    #[arg(short = 'p', long, env = "BUILDIT_WORKER_PERFORMANCE")]
    pub worker_performance: Option<i64>,

    /// Websocket uri
    #[arg(short = 'w', long, env = "BUILDIT_WS")]
    pub websocket: String,
}

pub fn get_memory_bytes() -> i64 {
    let system = System::new_all();
    if let Some(limits) = system.cgroup_limits() {
        limits.total_memory as i64
    } else {
        system.total_memory() as i64
    }
}
