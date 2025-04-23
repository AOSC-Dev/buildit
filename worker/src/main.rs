use clap::Parser;
use flume::unbounded;
use log::info;
use sysinfo::System;
use worker::{Args, build::build_worker, heartbeat::heartbeat_worker, websocket::websocket_worker};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();
    env_logger::init();
    let args = Args::parse();
    info!("Starting AOSC BuildIt! worker");

    // Refresh memory usage for get_memory_bytes()
    let mut s = System::new();
    s.refresh_memory();

    let (tx, rx) = unbounded();
    tokio::spawn(websocket_worker(args.clone(), rx));
    tokio::spawn(heartbeat_worker(args.clone()));
    build_worker(args.clone(), tx).await;
}
