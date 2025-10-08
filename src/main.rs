use askama::Template;
use axum::extract::ws::{Message, WebSocket};
use axum::response::Html;
use axum::{Router, extract::ws::WebSocketUpgrade, response::IntoResponse, routing::get};
use futures_util::{SinkExt, StreamExt};
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::sync::mpsc;
use tracing::{error, info};

mod audio_fanout;
mod audio_transcode;
mod g711;
mod webrtc;

use audio_fanout::AudioFanout;

// Application state shared across all connections
#[derive(Clone)]
struct AppState {
    audio_fanout: Arc<AudioFanout>,
    webrtc_infra: Arc<webrtc::WebRtcInfra>,
}

#[tokio::main]
async fn main() {
    // Load .env file if present (for development)
    let _ = dotenvy::dotenv();

    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    // Read DoorBird configuration from environment
    let doorbird_url =
        std::env::var("DOORBIRD_URL").expect("DOORBIRD_URL environment variable must be set");
    let doorbird_user =
        std::env::var("DOORBIRD_USER").expect("DOORBIRD_USER environment variable must be set");
    let doorbird_password = std::env::var("DOORBIRD_PASSWORD")
        .expect("DOORBIRD_PASSWORD environment variable must be set");

    // Create DoorBird client
    let doorbird_client = doorbird::Client::new(
        doorbird_url.clone(),
        doorbird_user.clone(),
        doorbird_password.clone(),
    );

    // Fetch and display device information
    info!("Connecting to DoorBird at {}", doorbird_url);
    match doorbird_client.info().await {
        Ok(device_info) => {
            info!("═══════════════════════════════════════════════");
            info!("DoorBird Device Information:");
            info!("  Firmware: {}", device_info.firmware);
            info!("  Build: {}", device_info.build_number);
            if let Some(device_type) = &device_info.device_type {
                info!("  Device Type: {}", device_type);
            }
            if let Some(mac) = &device_info.primary_mac_addr {
                info!("  MAC Address: {}", mac);
            }
            if let Some(relays) = &device_info.relays {
                info!("  Available Relays: {}", relays.join(", "));
            }
            info!("═══════════════════════════════════════════════");
        }
        Err(e) => {
            error!("Failed to fetch DoorBird device info: {:#}", e);
            error!("Continuing anyway, but audio streaming may not work properly");
        }
    }

    // Create audio fanout system (buffer 100 samples = ~2 seconds)
    let audio_fanout = AudioFanout::new(doorbird_client, 100);

    // Initialize shared WebRTC infrastructure (UDP mux on port 50000)
    let webrtc_infra = webrtc::WebRtcInfra::new()
        .await
        .expect("Failed to initialize WebRTC infrastructure");

    let state = AppState {
        audio_fanout,
        webrtc_infra,
    };

    let app = Router::new()
        .route("/intercom", get(intercom))
        .route("/ws", get(ws_handler))
        .with_state(state);

    let addr = SocketAddr::from(([0, 0, 0, 0], 3000));
    info!("Listening on http://{}", addr);
    let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
    axum::serve(listener, app).await.unwrap();
}

#[derive(Template)]
#[template(path = "intercom.html")]
struct IntercomTemplate;

async fn intercom() -> impl IntoResponse {
    let template = IntercomTemplate;
    match template.render() {
        Ok(html) => Html(html).into_response(),
        Err(err) => (
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            format!("Template error: {}", err),
        )
            .into_response(),
    }
}

async fn ws_handler(
    ws: WebSocketUpgrade,
    axum::extract::State(state): axum::extract::State<AppState>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_socket(socket, state))
}

async fn handle_socket(socket: WebSocket, state: AppState) {
    let (ws_tx, mut ws_rx) = {
        let (mut sender, receiver) = socket.split();
        let (out_tx, mut out_rx) = mpsc::unbounded_channel::<Message>();

        tokio::spawn(async move {
            while let Some(msg) = out_rx.recv().await {
                if let Err(e) = sender.send(msg).await {
                    error!("ws send error: {}", e);
                    break;
                }
            }
        });

        (out_tx, receiver)
    };

    let session = match webrtc::WebRtcSession::new(
        state.webrtc_infra.clone(),
        ws_tx.clone(),
        state.audio_fanout.clone(),
    )
    .await
    {
        Ok(s) => s,
        Err(e) => {
            error!("failed to create WebRTC session: {:#}", e);
            return;
        }
    };

    // Process incoming signaling messages
    while let Some(Ok(msg)) = ws_rx.next().await {
        match msg {
            Message::Text(txt) => {
                if let Err(e) = handle_signal_text(&session, &txt).await {
                    error!("signal handling error: {:#}", e);
                }
            }
            Message::Binary(bin) => {
                if let Ok(txt) = String::from_utf8(bin.to_vec()) {
                    handle_signal_text(&session, &txt)
                        .await
                        .unwrap_or_else(|e| {
                            error!("signal handling error: {:#}", e);
                        });
                }
            }
            Message::Close(_) => break,
            Message::Ping(_) | Message::Pong(_) => {}
        }
    }

    // Clean up WebRTC connection when WebSocket closes
    info!("WebSocket closed, cleaning up WebRTC session");
    if let Err(e) = session.pc.close().await {
        error!("Error closing peer connection: {:#}", e);
    }
}

async fn handle_signal_text(session: &webrtc::WebRtcSession, txt: &str) -> anyhow::Result<()> {
    let v: serde_json::Value = serde_json::from_str(txt)?;
    let t = v.get("type").and_then(|t| t.as_str()).unwrap_or("");
    match t {
        "offer" => {
            let sdp = v
                .get("sdp")
                .and_then(|s| s.as_str())
                .unwrap_or("")
                .to_string();
            info!("received client offer, creating answer...");
            let answer = session.set_remote_offer_and_create_answer(sdp).await?;
            info!("sending answer to client");
            let msg = serde_json::json!({
                "type": "answer",
                "sdp": answer.sdp,
            });
            let _ = session.ws_out.send(Message::Text(msg.to_string().into()));
        }
        "candidate" => {
            let candidate = v
                .get("candidate")
                .and_then(|c| c.as_str())
                .unwrap_or("")
                .to_string();
            let sdp_mid = v
                .get("sdpMid")
                .and_then(|m| m.as_str())
                .map(|s| s.to_string());
            let sdp_mline_index = v
                .get("sdpMLineIndex")
                .and_then(|i| i.as_u64())
                .map(|u| u as u16);
            info!(
                "received client ICE candidate: {} (mid: {:?}, mline: {:?})",
                candidate, sdp_mid, sdp_mline_index
            );
            session
                .add_ice_candidate(candidate, sdp_mid, sdp_mline_index)
                .await?;
        }
        _ => {}
    }
    Ok(())
}
