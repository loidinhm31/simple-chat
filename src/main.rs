mod camera;

use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        State,
    },
    response::IntoResponse,
    routing::get,
    Router,
};
use camera::CameraServer;
use futures::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, net::SocketAddr, sync::Arc};
use std::time::Duration;
use tokio::sync::{mpsc, RwLock};
use tower_http::services::ServeDir;

// Shared state between all connections
type Users = Arc<RwLock<HashMap<String, mpsc::UnboundedSender<Message>>>>;

#[derive(Debug, Serialize, Deserialize, Clone)]
struct WebRTCMessage {
    event: String,
    data: String,
    room: String,
    from: String,
    to: Option<String>,
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt::init();

    let users: Users = Arc::new(RwLock::new(HashMap::new()));
    let users_clone = users.clone();

    // Set up camera server
    let (frame_tx, mut frame_rx) = mpsc::unbounded_channel();
    let camera = CameraServer::new(frame_tx);

    // Spawn camera capture task with auto-restart
    tokio::spawn(async move {
        loop {
            match camera.start_capture().await {
                Ok(_) => println!("Camera capture ended normally"),
                Err(e) => eprintln!("Camera error: {}", e),
            }
            println!("Restarting camera capture in 5 seconds...");
            tokio::time::sleep(Duration::from_secs(5)).await;
        }
    });

    // Spawn frame broadcasting task
    tokio::spawn(async move {
        while let Some(frame) = frame_rx.recv().await {
            let frame_msg = WebRTCMessage {
                event: "camera-frame".to_string(),
                data: frame,
                room: "default-room".to_string(),
                from: "server-camera".to_string(),
                to: None,
            };

            if let Ok(frame_str) = serde_json::to_string(&frame_msg) {
                broadcast_message(&users_clone, &frame_str, None).await;
            }
        }
    });

    let app = Router::new()
        .route("/ws", get(ws_handler))
        .nest_service("/", ServeDir::new("static"))
        .with_state(users);

    let addr = SocketAddr::from(([0, 0, 0, 0], 3000));
    println!("listening on {}", addr);
    axum::Server::bind(&addr)
        .serve(app.into_make_service())
        .await
        .unwrap();
}

async fn ws_handler(
    ws: WebSocketUpgrade,
    State(state): State<Users>,
) -> impl IntoResponse {
    ws.on_upgrade(|socket| handle_socket(socket, state))
}

async fn handle_socket(socket: WebSocket, state: Users) {
    let (mut sender, mut receiver) = socket.split();
    let (tx, mut rx) = mpsc::unbounded_channel();

    // Task for sending messages to this client
    let mut send_task = tokio::spawn(async move {
        while let Some(msg) = rx.recv().await {
            if sender.send(msg).await.is_err() {
                break;
            }
        }
    });

    // Task for receiving messages from this client
    let mut recv_task = tokio::spawn(async move {
        let mut user_id = String::new();

        while let Some(Ok(msg)) = receiver.next().await {
            if let Message::Text(text) = msg {
                if let Ok(mut msg) = serde_json::from_str::<WebRTCMessage>(&text) {
                    println!("Received message: {:?}", msg.event);
                    match msg.event.as_str() {
                        "join" => {
                            user_id = msg.from.clone();
                            let user_joined_msg = serde_json::to_string(&WebRTCMessage {
                                event: "user_joined".to_string(),
                                data: user_id.clone(),
                                room: msg.room.clone(),
                                from: "server".to_string(),
                                to: None,
                            }).unwrap();

                            // Store the sender
                            state.write().await.insert(user_id.clone(), tx.clone());

                            // Broadcast to all users except the sender
                            broadcast_message(&state, &user_joined_msg, Some(&user_id)).await;
                        }
                        "message" => {
                            // Broadcast chat messages to all users
                            broadcast_message(&state, &text, None).await;
                        }
                        "offer" | "answer" | "ice-candidate" => {
                            // For WebRTC signaling, send to specific peer if specified
                            if let Some(to) = &msg.to {
                                if let Some(peer_tx) = state.read().await.get(to) {
                                    let _ = peer_tx.send(Message::Text(text));
                                }
                            } else {
                                // If no specific peer, broadcast to all except sender
                                broadcast_message(&state, &text, Some(&msg.from)).await;
                            }
                        }
                        _ => {}
                    }
                }
            }
        }

        // Remove user when they disconnect
        if !user_id.is_empty() {
            state.write().await.remove(&user_id);

            // Broadcast user disconnection
            let user_left_msg = serde_json::to_string(&WebRTCMessage {
                event: "user_left".to_string(),
                data: user_id.clone(),
                room: "default-room".to_string(),
                from: "server".to_string(),
                to: None,
            }).unwrap();
            broadcast_message(&state, &user_left_msg, None).await;
        }
    });

    tokio::select! {
        _ = (&mut send_task) => recv_task.abort(),
        _ = (&mut recv_task) => send_task.abort(),
    };
}

async fn broadcast_message(state: &Users, message: &str, exclude_user: Option<&str>) {
    let users = state.read().await;
    for (user_id, tx) in users.iter() {
        if exclude_user.map_or(true, |excluded| user_id != excluded) {
            let _ = tx.send(Message::Text(message.to_string()));
        }
    }
}