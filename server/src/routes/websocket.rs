use std::net::SocketAddr;

use axum::{
    extract::{ws::WebSocket, ConnectInfo, Path, State, WebSocketUpgrade},
    response::IntoResponse,
};
use futures::{channel::mpsc::unbounded, future, pin_mut, StreamExt, TryStreamExt};
use tracing::info;

use super::{AppState, PeerMap};

/// The handler for the HTTP request (this gets called when the HTTP GET lands at the start
/// of websocket negotiation). After this completes, the actual switching from HTTP to
/// websocket protocol will occur.
/// This is the last point where we can extract TCP/IP metadata such as IP address of the client
/// as well as things from HTTP headers such as user-agent of the browser etc.
pub async fn ws_handler(
    Path(hostname): Path<String>,
    ws: WebSocketUpgrade,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    State(state): State<AppState>,
) -> impl IntoResponse {
    // finalize the upgrade process by returning upgrade callback.
    // we can customize the callback by sending additional info such as address.
    ws.on_upgrade(move |socket| handle_socket(socket, addr, hostname, state.ws_peer_map))
}

/// Actual websocket statemachine (one will be spawned per connection)
async fn handle_socket(socket: WebSocket, who: SocketAddr, hostname: String, peer_map: PeerMap) {
    let (tx, rx) = unbounded();
    peer_map
        .write()
        .unwrap()
        .insert(who, (tx, hostname.clone()));

    let (outgoing, incoming) = socket.split();

    let broadcast_incoming = incoming.try_for_each(|msg| {
        info!("Received a message from {}: {:?}", who, msg);

        let peers = peer_map.read().unwrap();

        // We want to broadcast the message to everyone except ourselves.
        let broadcast_recipients = peers
            .iter()
            .filter(|(peer_addr, _)| peer_addr != &&who)
            .map(|(_, (ws_sink, port))| (ws_sink, port));

        for recp in broadcast_recipients {
            let recp_path = recp.1;

            if *recp_path == hostname {
                recp.0.unbounded_send(msg.clone()).unwrap();
            }
        }

        future::ok(())
    });

    let receive_from_others = rx.map(Ok).forward(outgoing);

    pin_mut!(broadcast_incoming, receive_from_others);
    future::select(broadcast_incoming, receive_from_others).await;

    info!("{} disconnected", &who);
    peer_map.write().unwrap().remove(&who);
}
