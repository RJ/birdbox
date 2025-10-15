//! Birdbox - WebRTC Gateway for DoorBird Smart Doorbells
//!
//! This application streams audio and video from DoorBird smart doorbells to web browsers
//! via WebRTC, enabling real-time viewing and two-way push-to-talk communication.
//!
//! # Architecture
//!
//! - HTTP/WebSocket server (Axum) for web interface and signaling
//! - WebRTC server for audio/video streaming  
//! - DoorBird API client for device communication
//! - Fanout system for distributing one DoorBird connection to N viewers
//! - Audio transcoding pipeline (G.711 μ-law ↔ Opus)
//! - Video pass-through (H.264 forwarding, no transcoding)
//!
//! See `docs/ARCHITECTURE.md` for detailed system design.

use askama::Template;
use axum::extract::ws::{Message, WebSocket};
use axum::response::Html;
use axum::{extract::ws::WebSocketUpgrade, response::IntoResponse, routing::get, Router};
use futures_util::{SinkExt, StreamExt};
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::sync::{broadcast, mpsc, RwLock};
use tower_http::services::ServeDir;
use tracing::{debug, error, info, warn};
use uuid::Uuid;

mod audio_fanout;
mod audio_transcode;
mod g711;
mod h264_extractor;
mod video_fanout;
mod webrtc;

use audio_fanout::AudioFanout;
use video_fanout::VideoFanout;

/// Push-to-talk (PTT) state coordinator
///
/// Ensures only one client can transmit audio to the DoorBird at a time.
/// Broadcasts PTT state changes to all connected clients so they can update their UI.
struct PttState {
    /// Session ID of the client currently transmitting (if any)
    active_session: Arc<RwLock<Option<Uuid>>>,
    /// Broadcast channel for PTT state updates to all clients
    state_tx: broadcast::Sender<PttStateMessage>,
}

/// PTT state change notification sent to all connected clients
#[derive(Clone, Debug)]
struct PttStateMessage {
    /// Whether someone is currently transmitting
    transmitting: bool,
    /// ID of the session that's transmitting (for debugging)
    #[allow(dead_code)]
    session_id: Option<Uuid>,
}

impl PttState {
    fn new() -> Self {
        let (state_tx, _) = broadcast::channel(100);
        Self {
            active_session: Arc::new(RwLock::new(None)),
            state_tx,
        }
    }

    /// Attempt to acquire PTT lock for a session
    async fn try_acquire(&self, session_id: Uuid) -> bool {
        let mut active = self.active_session.write().await;
        if active.is_none() {
            *active = Some(session_id);
            info!("PTT acquired by session {}", session_id);

            // Broadcast state change
            let _ = self.state_tx.send(PttStateMessage {
                transmitting: true,
                session_id: Some(session_id),
            });

            true
        } else {
            warn!(
                "PTT denied for session {} - already in use by {:?}",
                session_id, *active
            );
            false
        }
    }

    /// Release PTT lock for a session
    async fn release(&self, session_id: Uuid) {
        let mut active = self.active_session.write().await;
        if *active == Some(session_id) {
            *active = None;
            info!("PTT released by session {}", session_id);

            // Broadcast state change
            let _ = self.state_tx.send(PttStateMessage {
                transmitting: false,
                session_id: None,
            });
        }
    }

    /// Subscribe to PTT state changes
    fn subscribe(&self) -> broadcast::Receiver<PttStateMessage> {
        self.state_tx.subscribe()
    }

    /// Check if currently transmitting
    async fn is_transmitting(&self) -> bool {
        let active = self.active_session.read().await;
        active.is_some()
    }
}

/// Application state shared across all connections
///
/// Holds all the shared resources that WebSocket handlers and HTTP endpoints need access to.
#[derive(Clone)]
struct AppState {
    /// Audio fanout for distributing DoorBird audio to multiple clients
    audio_fanout: Arc<AudioFanout>,
    /// Video fanout for distributing DoorBird video to multiple clients
    video_fanout: Arc<VideoFanout>,
    /// Shared WebRTC infrastructure (UDP mux, API)
    webrtc_infra: Arc<webrtc::WebRtcInfra>,
    /// Push-to-talk coordination
    ptt_state: Arc<PttState>,
    /// DoorBird API client for device control
    doorbird_client: doorbird::Client,
}

#[tokio::main]
async fn main() {
    // Load .env file if present (for development)
    if dotenvy::dotenv().is_ok() {
        info!("Loaded .env file");
    }

    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    // Read DoorBird configuration from environment
    let doorbird_url = std::env::var("BIRDBOX_DOORBIRD_URL")
        .expect("BIRDBOX_DOORBIRD_URL environment variable must be set");
    let doorbird_user = std::env::var("BIRDBOX_DOORBIRD_USER")
        .expect("BIRDBOX_DOORBIRD_USER environment variable must be set");
    let doorbird_password = std::env::var("BIRDBOX_DOORBIRD_PASSWORD")
        .expect("BIRDBOX_DOORBIRD_PASSWORD environment variable must be set");

    // Read video configuration from environment
    let video_buffer_frames = std::env::var("BIRDBOX_VIDEO_FANOUT_BUFFER_FRAMES")
        .ok()
        .and_then(|s| s.parse::<usize>().ok())
        .unwrap_or(4); // Default to 4 frames if not set or invalid
    info!("Video fanout buffer size: {} frames", video_buffer_frames);

    // Create DoorBird client
    let doorbird_client = doorbird::Client::new(
        doorbird_url.clone(),
        doorbird_user.clone(),
        doorbird_password.clone(),
    );

    // Spawn background task to monitor DoorBird events
    let monitor_client = doorbird_client.clone();
    tokio::spawn(async move {
        loop {
            info!("DoorBird event monitor connecting...");

            match monitor_client.monitor_events().await {
                Ok(mut event_stream) => {
                    info!("DoorBird event monitor connected");

                    // Process events as they arrive
                    while let Some(event_result) = event_stream.next().await {
                        match event_result {
                            Ok(doorbird::MonitorEvent::Doorbell) => {
                                info!("🔔 DoorBird event: Doorbell pressed!");
                            }
                            Ok(doorbird::MonitorEvent::MotionSensor { active }) => {
                                if active {
                                    warn!("👁️  DoorBird event: Motion detected!");
                                } else {
                                    info!("DoorBird event: Motion cleared");
                                }
                            }
                            Err(e) => {
                                warn!("DoorBird event stream error: {:#}", e);
                                break;
                            }
                        }
                    }

                    warn!("DoorBird event monitor disconnected, reconnecting in 5s...");
                }
                Err(e) => {
                    warn!(
                        "Failed to connect to DoorBird event monitor: {:#}, reconnecting in 5s...",
                        e
                    );
                }
            }

            // Wait before reconnecting
            tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
        }
    });

    // Fetch and display device information
    info!("Connecting to DoorBird at {}", doorbird_url);
    let device_info = match doorbird_client.info().await {
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
            Some(device_info)
        }
        Err(e) => {
            error!("Failed to fetch DoorBird device info: {:#}", e);
            error!("Continuing anyway, but features may be limited");
            None
        }
    };

    // Determine video quality based on device capabilities
    let video_quality = if let Some(ref info) = device_info {
        if info.supports_1080p() {
            info!("Device supports 1080p video");
            doorbird::VideoQuality::P1080
        } else if info.supports_720p() {
            info!("Device supports 720p video");
            doorbird::VideoQuality::P720
        } else {
            info!("Using default video resolution");
            doorbird::VideoQuality::Default
        }
    } else {
        info!("Using default video resolution (device info unavailable)");
        doorbird::VideoQuality::Default
    };

    // Get RTSP URL for video streaming
    let rtsp_url = doorbird_client.video_receive(video_quality);
    info!("RTSP URL configured for video streaming");

    // Create audio fanout system with configurable buffer size
    let audio_buffer_samples = std::env::var("BIRDBOX_AUDIO_FANOUT_BUFFER_SAMPLES")
        .ok()
        .and_then(|s| s.parse::<usize>().ok())
        .unwrap_or(20); // Default to 20 samples (~400ms) if not set or invalid
    info!(
        "Audio fanout buffer size: {} samples (~{}ms)",
        audio_buffer_samples,
        audio_buffer_samples * 20
    );
    let audio_fanout = AudioFanout::new(doorbird_client.clone(), audio_buffer_samples);

    // Read RTSP transport protocol configuration
    let rtsp_transport = std::env::var("BIRDBOX_RTSP_TRANSPORT_PROTOCOL")
        .unwrap_or_else(|_| "udp".to_string())
        .to_lowercase();

    // Validate and normalize the transport protocol
    let rtsp_transport = match rtsp_transport.as_str() {
        "tcp" => {
            info!("Using TCP transport for RTSP (more reliable for VPN/Docker scenarios)");
            "tcp"
        }
        _ => {
            info!("Using UDP transport for RTSP (lower latency for simple networks)");
            "udp"
        }
    };

    // Create video fanout system with configurable buffer size
    let video_fanout = VideoFanout::new(rtsp_url, video_buffer_frames, rtsp_transport);

    // Initialize shared WebRTC infrastructure (UDP mux on port 50000)
    let webrtc_infra = webrtc::WebRtcInfra::new()
        .await
        .expect("Failed to initialize WebRTC infrastructure");

    // Create PTT state manager
    let ptt_state = Arc::new(PttState::new());

    let state = AppState {
        audio_fanout,
        video_fanout,
        webrtc_infra,
        ptt_state,
        doorbird_client,
    };

    let app = Router::new()
        .route("/", get(intercom))
        .route("/intercom", get(intercom))
        .route("/ws", get(ws_handler))
        .route("/api/open-gates", axum::routing::post(open_gates))
        .nest_service("/static", ServeDir::new("static"))
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

async fn open_gates(
    axum::extract::State(state): axum::extract::State<AppState>,
) -> impl IntoResponse {
    match state.doorbird_client.open_door(None).await {
        Ok(_) => Html(
            r#"<div class="alert alert-success alert-dismissible fade show" role="alert">
                Gates opened successfully!
                <button type="button" class="btn-close" data-bs-dismiss="alert"></button>
            </div>"#
                .to_string(),
        ),
        Err(e) => Html(format!(
            r#"<div class="alert alert-danger alert-dismissible fade show" role="alert">
                    Failed to open gates: {}
                    <button type="button" class="btn-close" data-bs-dismiss="alert"></button>
                </div>"#,
            e
        )),
    }
}

async fn ws_handler(
    ws: WebSocketUpgrade,
    axum::extract::State(state): axum::extract::State<AppState>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_socket(socket, state))
}

async fn handle_socket(socket: WebSocket, state: AppState) {
    // Generate unique session ID
    let session_id = Uuid::new_v4();
    info!("New WebSocket connection: session {}", session_id);

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

    // Subscribe to PTT state changes
    let mut ptt_state_rx = state.ptt_state.subscribe();
    let ws_tx_for_ptt = ws_tx.clone();

    // Spawn task to forward PTT state changes to this client
    let ptt_forward_task = tokio::spawn(async move {
        while let Ok(ptt_msg) = ptt_state_rx.recv().await {
            let json = serde_json::json!({
                "type": "ptt_state",
                "transmitting": ptt_msg.transmitting,
            });
            let _ = ws_tx_for_ptt.send(Message::Text(json.to_string().into()));
        }
    });

    let session = match webrtc::WebRtcSession::new(
        state.webrtc_infra.clone(),
        ws_tx.clone(),
        state.audio_fanout.clone(),
        state.video_fanout.clone(),
        state.ptt_state.clone(),
        state.doorbird_client.clone(),
        session_id,
    )
    .await
    {
        Ok(s) => s,
        Err(e) => {
            error!("failed to create WebRTC session: {:#}", e);
            return;
        }
    };

    // Send initial PTT state
    let initial_transmitting = state.ptt_state.is_transmitting().await;
    let initial_state_msg = serde_json::json!({
        "type": "ptt_state",
        "transmitting": initial_transmitting,
    });
    let _ = ws_tx.send(Message::Text(initial_state_msg.to_string().into()));

    // Process incoming signaling messages
    while let Some(Ok(msg)) = ws_rx.next().await {
        match msg {
            Message::Text(txt) => {
                if let Err(e) = handle_signal_text(&session, &state, session_id, &txt).await {
                    error!("signal handling error: {:#}", e);
                }
            }
            Message::Binary(bin) => {
                if let Ok(txt) = String::from_utf8(bin.to_vec()) {
                    handle_signal_text(&session, &state, session_id, &txt)
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
    info!("WebSocket closed, cleaning up session {}", session_id);

    // Release PTT if this session had it
    state.ptt_state.release(session_id).await;

    // Stop PTT forward task
    ptt_forward_task.abort();

    // Don't explicitly close the peer connection to avoid affecting the shared UDP mux
    // The peer connection will be garbage collected when the session is dropped
    info!(
        "Session {} cleanup complete, peer connection will be garbage collected",
        session_id
    );
}

/// Handle WebRTC signaling messages from the client
///
/// Processes JSON messages for:
/// - SDP offer/answer exchange
/// - ICE candidate exchange  
/// - Push-to-talk control (start/stop)
async fn handle_signal_text(
    session: &webrtc::WebRtcSession,
    state: &AppState,
    session_id: Uuid,
    json_text: &str,
) -> anyhow::Result<()> {
    let signal_msg: serde_json::Value = serde_json::from_str(json_text)?;
    let msg_type = signal_msg
        .get("type")
        .and_then(|t| t.as_str())
        .unwrap_or("");
    match msg_type {
        "offer" => {
            let sdp = signal_msg
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
            let candidate = signal_msg
                .get("candidate")
                .and_then(|c| c.as_str())
                .unwrap_or("")
                .to_string();
            let sdp_mid = signal_msg
                .get("sdpMid")
                .and_then(|m| m.as_str())
                .map(|s| s.to_string());
            let sdp_mline_index = signal_msg
                .get("sdpMLineIndex")
                .and_then(|i| i.as_u64())
                .map(|u| u as u16);

            // Log mDNS candidates at debug level to reduce spam (they're ignored anyway)
            if candidate.contains(".local") {
                debug!(
                    "received client ICE candidate (mDNS): {} (mid: {:?}, mline: {:?})",
                    candidate, sdp_mid, sdp_mline_index
                );
            } else {
                info!(
                    "received client ICE candidate: {} (mid: {:?}, mline: {:?})",
                    candidate, sdp_mid, sdp_mline_index
                );
            }
            session
                .add_ice_candidate(candidate, sdp_mid, sdp_mline_index)
                .await?;
        }
        "start_ptt" => {
            info!("PTT start requested by session {}", session_id);
            if state.ptt_state.try_acquire(session_id).await {
                info!("PTT granted to session {}", session_id);
                session.start_ptt().await?;
                let msg = serde_json::json!({
                    "type": "ptt_granted",
                });
                let _ = session.ws_out.send(Message::Text(msg.to_string().into()));
            } else {
                warn!("PTT denied to session {} - already in use", session_id);
                let msg = serde_json::json!({
                    "type": "ptt_denied",
                    "reason": "another_user",
                });
                let _ = session.ws_out.send(Message::Text(msg.to_string().into()));
            }
        }
        "stop_ptt" => {
            info!("PTT stop requested by session {}", session_id);
            session.stop_ptt().await;
            state.ptt_state.release(session_id).await;
        }
        _ => {}
    }
    Ok(())
}
