use std::sync::Arc;

use super::{AppState, ViewerMap};
use crate::{routes::Viewer, RemoteAddr};
use axum::{
    extract::{ws::WebSocket, ConnectInfo, Path, State, WebSocketUpgrade},
    response::IntoResponse,
};
use futures::{channel::mpsc::unbounded, future, StreamExt, TryStreamExt};
use tracing::info;

pub async fn ws_worker_handler(
    Path(hostname): Path<String>,
    ws: WebSocketUpgrade,
    ConnectInfo(addr): ConnectInfo<RemoteAddr>,
    State(state): State<AppState>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_worker_socket(socket, addr, hostname, state.ws_viewer_map))
}

async fn handle_worker_socket(
    socket: WebSocket,
    who: RemoteAddr,
    hostname: String,
    viewer_map: ViewerMap,
) {
    info!("{:?} connected as worker", who);

    let (_outgoing, incoming) = socket.split();

    // forward websocket to tx
    if let Err(err) = incoming
        .try_for_each(|msg| {
            info!("Received a message from {:?}: {:?}", who, msg);

            // We want to broadcast the message to viewers subscribing to the hostname
            if let Some(viewers) = viewer_map.read().unwrap().get(&hostname) {
                for recp in viewers {
                    recp.sender.unbounded_send(msg.clone()).unwrap();
                }
            }

            future::ok(())
        })
        .await
    {
        info!("{:?} errored with {:?}", who, err);
    }

    info!("{:?} disconnected as worker", who);
}

pub async fn ws_viewer_handler(
    Path(hostname): Path<String>,
    ws: WebSocketUpgrade,
    ConnectInfo(addr): ConnectInfo<RemoteAddr>,
    State(state): State<AppState>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_viewer_socket(socket, addr, hostname, state.ws_viewer_map))
}

async fn handle_viewer_socket(
    socket: WebSocket,
    who: RemoteAddr,
    hostname: String,
    viewer_map: ViewerMap,
) {
    let (tx, rx) = unbounded();
    info!("{:?} connected as viewer", who);

    // register our tx to ViewerMap
    let viewer = Arc::new(Viewer {
        remote_addr: who.clone(),
        sender: tx,
    });
    viewer_map
        .write()
        .unwrap()
        .entry(hostname.clone())
        .or_default()
        .push(viewer.clone());

    let (outgoing, _incoming) = socket.split();
    // forward rx to websocket
    if let Err(err) = rx.map(Ok).forward(outgoing).await {
        info!("{:?} errored with {:?}", who, err);
    }

    info!("{:?} disconnected as viewer", who);

    // remove from viewer map
    viewer_map
        .write()
        .unwrap()
        .entry(hostname.clone())
        .or_default()
        .retain(|v| !Arc::ptr_eq(v, &viewer));
}
