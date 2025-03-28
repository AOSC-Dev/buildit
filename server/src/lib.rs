use anyhow::{bail, Context};
use axum::{extract::connect_info, serve::IncomingStream};
use chrono::{Datelike, Days};
use clap::Parser;
use diesel::{
    r2d2::{ConnectionManager, Pool},
    PgConnection,
};
use once_cell::sync::Lazy;
use reqwest::ClientBuilder;
use std::{net::SocketAddr, path::PathBuf, sync::Arc};
use tokio::net::{unix::UCred, UnixStream};

pub mod api;
pub mod bot;
pub mod formatter;
pub mod github;
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

    /// Listen to unix socket if set
    #[arg(env = "BUILDIT_LISTEN_SOCKET_PATH")]
    pub unix_socket: Option<PathBuf>,
}

pub static ARGS: Lazy<Args> = Lazy::new(Args::parse);
pub const HEARTBEAT_TIMEOUT: i64 = 600; // 10 minutes

// follow https://github.com/AOSC-Dev/autobuild3/blob/master/sets/arch_groups/mainline
pub(crate) const ALL_ARCH: &[&str] = &[
    "amd64",
    "arm64",
    "loongarch64",
    "loongson3",
    "ppc64el",
    "riscv64",
];

// https://github.com/tokio-rs/axum/blob/main/examples/unix-domain-socket/src/main.rs
#[derive(Clone, Debug)]
pub enum RemoteAddr {
    Uds(UdsSocketAddr),
    Inet(SocketAddr),
}

#[derive(Clone, Debug)]
#[allow(dead_code)]
pub struct UdsSocketAddr {
    peer_addr: Arc<tokio::net::unix::SocketAddr>,
    peer_cred: UCred,
}

impl connect_info::Connected<&UnixStream> for RemoteAddr {
    fn connect_info(target: &UnixStream) -> Self {
        let peer_addr = target.peer_addr().unwrap();
        let peer_cred = target.peer_cred().unwrap();

        Self::Uds(UdsSocketAddr {
            peer_addr: Arc::new(peer_addr),
            peer_cred,
        })
    }
}

impl connect_info::Connected<IncomingStream<'_>> for RemoteAddr {
    fn connect_info(target: IncomingStream) -> Self {
        Self::Inet(target.remote_addr())
    }
}

pub async fn paste_to_aosc_io(title: &str, text: &str) -> anyhow::Result<String> {
    if text.len() > 10485760 {
        bail!("text is too large to be pasted to https://aosc.io/paste")
    }
    let client = ClientBuilder::new().user_agent("buildit").build()?;
    let exp_date = chrono::Utc::now()
        .checked_add_days(Days::new(7))
        .context("failed to generate expDate")?;
    let exp_date = format!(
        "{:04}-{:02}-{:02}",
        exp_date.year(),
        exp_date.month(),
        exp_date.day()
    );
    let resp = client
        .post("https://aosc.io/pasteApi/paste")
        .form(&[
            ("title", title),
            ("language", "plaintext"),
            ("content", text),
            ("expDate", &exp_date),
        ])
        .send()
        .await?
        .error_for_status()?
        .json::<serde_json::Value>()
        .await?;
    if resp.get("code").and_then(|v| v.as_u64()) != Some(0) {
        let msg = resp
            .get("message")
            .and_then(|v| v.as_str())
            .unwrap_or("(no message field)");
        bail!("aosc.io/paste error: {}", msg)
    } else {
        let id = resp
            .get("data")
            .and_then(|v| v.get("id"))
            .and_then(|v| v.as_str())
            .context("$.data.id not found from paste response")?;
        Ok(id.to_string())
    }
}

#[tokio::test]
async fn test_paste_to_aosc_io() {
    let id = paste_to_aosc_io(
        "Test message for test_paste_to_aosc_io",
        "Some random texts here",
    )
    .await
    .unwrap();
    dbg!(id);
}
