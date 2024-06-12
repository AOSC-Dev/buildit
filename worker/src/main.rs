use clap::Parser;
use log::info;
use sysinfo::System;
use worker::{build::build_worker, heartbeat::heartbeat_worker, Args};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenv::dotenv().ok();
    env_logger::init();
    let args = Args::parse();
    info!("Starting AOSC BuildIt! worker");

    // Refresh memory usage for get_memory_bytes()
    let mut s = System::new();
    s.refresh_memory();

    tokio::spawn(heartbeat_worker(args.clone()));

    build_worker(args.clone()).await;
    Ok(())
}
