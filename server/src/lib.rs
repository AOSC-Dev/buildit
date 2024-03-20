use clap::Parser;
use diesel::{
    r2d2::{ConnectionManager, Pool},
    PgConnection,
};
use once_cell::sync::Lazy;
use std::path::PathBuf;

pub mod api;
pub mod bot;
pub mod formatter;
pub mod github;
pub mod github_webhooks;
pub mod models;
pub mod recycler;
pub mod routes;
pub mod schema;

pub type DbPool = Pool<ConnectionManager<PgConnection>>;

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
pub struct Args {
    /// Database connection url
    #[arg(env = "DATABASE_URL")]
    pub database_url: String,

    #[arg(env = "BUILDIT_ABBS_PATH")]
    pub abbs_path: PathBuf,

    /// GitHub access token
    #[arg(env = "BUILDIT_GITHUB_ACCESS_TOKEN")]
    pub github_access_token: String,

    #[arg(env = "BUILDIT_WORKER_SECRET")]
    pub worker_secret: String,

    /// Secret
    #[arg(env = "BUILDIT_GITHUB_SECRET")]
    pub github_secret: Option<String>,

    #[arg(env = "BUILDIT_GITHUB_APP_ID")]
    pub github_app_id: Option<String>,

    #[arg(env = "BUILDIT_GITHUB_APP_KEY_PEM_PATH")]
    pub github_app_key: Option<PathBuf>,

    /// Development mode
    #[arg(env = "BUILDIT_DEVELOPMENT")]
    pub development_mode: Option<bool>,

    /// OpenTelemetry
    #[arg(env = "BUILDIT_OTLP")]
    pub otlp_url: Option<String>,

    /// Local repo path if available
    #[arg(env = "BUILDIT_REPO_PATH")]
    pub local_repo: Option<PathBuf>,
}

pub static ARGS: Lazy<Args> = Lazy::new(Args::parse);
pub static ABBS_REPO_LOCK: Lazy<tokio::sync::Mutex<()>> = Lazy::new(|| tokio::sync::Mutex::new(()));

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
