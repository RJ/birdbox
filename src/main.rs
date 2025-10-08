use askama::Template;
use axum::extract::ws::{Message, WebSocket};
use axum::response::Html;
use axum::{Router, extract::ws::WebSocketUpgrade, response::IntoResponse, routing::get};
use futures_util::{SinkExt, StreamExt};
use std::net::SocketAddr;
use tokio::sync::mpsc;
use tracing::{error, info};

mod webrtc;

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let app = Router::new()
        .route("/intercom", get(intercom))
        .route("/ws", get(ws_handler));

    let addr = SocketAddr::from(([0, 0, 0, 0], 3000));
    info!("listening on http://{}", addr);
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

async fn ws_handler(ws: WebSocketUpgrade) -> impl IntoResponse {
    ws.on_upgrade(handle_socket)
}

async fn handle_socket(socket: WebSocket) {
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

    let session = match webrtc::WebRtcSession::new(ws_tx.clone()).await {
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
                    if let Err(e) = handle_signal_text(&session, &txt).await {
                        error!("signal handling error: {:#}", e);
                    }
                }
            }
            Message::Close(_) => break,
            Message::Ping(_) | Message::Pong(_) => {}
        }
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
