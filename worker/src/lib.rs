use clap::Parser;
use std::path::PathBuf;

pub mod build;
pub mod heartbeat;

#[derive(Parser, Debug, Clone)]
#[command(author, version, about, long_about = None)]
pub struct Args {
    /// buildit server url e.g. https://buildit.aosc.io
    #[arg(short = 'H', long, env = "BUILDIT_SERVER")]
    pub server: String,

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
}
