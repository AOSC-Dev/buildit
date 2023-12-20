use clap::Parser;
use lapin::{Channel, Connection, ConnectionProperties};
use once_cell::sync::Lazy;
use std::{path::PathBuf, sync::Arc};
use tokio::sync::Mutex;

pub static CONNECTION: Lazy<Arc<Mutex<Option<Connection>>>> =
    Lazy::new(|| Arc::new(Mutex::new(None)));

pub mod build;
pub mod heartbeat;

// try to reuse amqp channel
pub async fn ensure_channel(args: &Args) -> anyhow::Result<Channel> {
    let mut lock = CONNECTION.lock().await;
    let conn = match &*lock {
        Some(conn) => {
            if conn.status().connected() {
                conn
            } else {
                // re-connect
                *lock = None;

                let conn =
                    lapin::Connection::connect(&args.amqp_addr, ConnectionProperties::default())
                        .await?;
                *lock = Some(conn);
                lock.as_ref().unwrap()
            }
        }
        None => {
            let conn = lapin::Connection::connect(&args.amqp_addr, ConnectionProperties::default())
                .await?;
            *lock = Some(conn);
            lock.as_ref().unwrap()
        }
    };

    let channel = conn.create_channel().await?;
    Ok(channel)
}

#[derive(Parser, Debug, Clone)]
#[command(author, version, about, long_about = None)]
pub struct Args {
    /// AMQP address to access message queue
    #[arg(short, long, env = "BUILDIT_AMQP_ADDR")]
    pub amqp_addr: String,

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
}
