use super::{AppState, WSStateMap};
use crate::{routes::Viewer, RemoteAddr};
use axum::{
    extract::{ws::WebSocket, ConnectInfo, Path, State, WebSocketUpgrade},
    response::IntoResponse,
};
use futures::{channel::mpsc::unbounded, future, SinkExt, StreamExt, TryStreamExt};
use std::sync::Arc;
use tracing::info;

pub async fn ws_worker_handler(
    Path(hostname): Path<String>,
    ws: WebSocketUpgrade,
    ConnectInfo(addr): ConnectInfo<RemoteAddr>,
    State(state): State<AppState>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_worker_socket(socket, addr, hostname, state.ws_state_map))
}

async fn handle_worker_socket(
    socket: WebSocket,
    who: RemoteAddr,
    hostname: String,
    state_map: WSStateMap,
) {
    info!("{:?} connected as worker with hostname {}", who, hostname);

    let (_outgoing, incoming) = socket.split();

    // forward websocket to tx
    if let Err(err) = incoming
        .try_for_each(|msg| {
            // We want to broadcast the message to viewers subscribing to the hostname
            let mut map = state_map.lock().unwrap();
            if let Some(state) = map.get_mut(&hostname) {
                for recp in &state.viewers {
                    recp.sender.unbounded_send(msg.clone()).unwrap();
                }

                // save last 1000 entries
                state.last_logs.push_back(msg.clone());
                if state.last_logs.len() > 1000 {
                    state.last_logs.pop_front();
                }
            }

            future::ok(())
        })
        .await
    {
        info!(
            "{:?} finished with {:?} as worker with hostname {}",
            who, err, hostname
        );
    }

    info!(
        "{:?} disconnected as worker with hostname {}",
        who, hostname
    );
}

pub async fn ws_viewer_handler(
    Path(hostname): Path<String>,
    ws: WebSocketUpgrade,
    ConnectInfo(addr): ConnectInfo<RemoteAddr>,
    State(state): State<AppState>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_viewer_socket(socket, addr, hostname, state.ws_state_map))
}

async fn handle_viewer_socket(
    socket: WebSocket,
    who: RemoteAddr,
    hostname: String,
    state_map: WSStateMap,
) {
    let (tx, rx) = unbounded();
    info!("{:?} connected as viewer with hostname {}", who, hostname);
    let (mut outgoing, _incoming) = socket.split();

    // register our tx to WSStateMap
    // and return latest logs
    let viewer = Arc::new(Viewer {
        remote_addr: who.clone(),
        sender: tx,
    });
    let msgs = {
        let mut map = state_map.lock().unwrap();
        let state = map.entry(hostname.clone()).or_default();

        state.viewers.push(viewer.clone());

        // collect last logs
        state.last_logs.clone()
    };
    for msg in msgs {
        outgoing.send(msg).await.ok();
    }

    // forward rx to websocket
    if let Err(err) = rx.map(Ok).forward(outgoing).await {
        info!(
            "{:?} finished with {:?} as viewer with hostname {}",
            who, err, hostname
        );
    }

    info!(
        "{:?} disconnected as viewer with hostname {}",
        who, hostname
    );

    // remove from viewer map
    let mut map = state_map.lock().unwrap();
    let state = map.entry(hostname.clone()).or_default();
    state.viewers.retain(|v| !Arc::ptr_eq(v, &viewer));
}
