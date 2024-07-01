use crate::Args;
use flume::Receiver;
use futures_util::StreamExt;
use log::{info, warn};
use reqwest::Url;
use std::time::Duration;
use tokio_tungstenite::{connect_async, tungstenite::Message};

pub async fn websocket_worker(args: Args, rx: Receiver<Message>) -> anyhow::Result<()> {
    // wss://hostname/api/ws/worker/:hostname
    let hostname = gethostname::gethostname().to_string_lossy().to_string();
    let ws = Url::parse(&args.server.replace("http", "ws"))?
        .join("api/")?
        .join("ws/")?
        .join("worker/")?
        .join(&hostname)?;

    loop {
        info!("Starting websocket connect to {:?}", ws);
        match connect_async(ws.as_str()).await {
            Ok((ws_stream, _)) => {
                let (write, _) = ws_stream.split();
                let rx = rx.clone().into_stream();
                if let Err(e) = rx.map(Ok).forward(write).await {
                    warn!("Failed to forward message to websocket: {e}");
                }
            }
            Err(err) => {
                warn!("Got error connecting to websocket: {}", err);
            }
        }

        tokio::time::sleep(Duration::from_secs(5)).await;
    }
}
