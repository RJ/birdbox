use crate::audio_fanout::AudioFanout;
use anyhow::Result;
use axum::extract::ws::Message;
use std::sync::Arc;
use tokio::net::UdpSocket;
use tokio::sync::mpsc::UnboundedSender;
use tracing::{error, info};
use webrtc::api::APIBuilder;
use webrtc::api::media_engine::{MIME_TYPE_OPUS, MediaEngine};
use webrtc::ice::udp_mux::*;
use webrtc::ice::udp_network::UDPNetwork;
use webrtc::ice_transport::ice_candidate::RTCIceCandidateInit;
use webrtc::ice_transport::ice_server::RTCIceServer;
use webrtc::interceptor::registry::Registry;
use webrtc::media::Sample;
use webrtc::peer_connection::RTCPeerConnection;
use webrtc::peer_connection::configuration::RTCConfiguration;
use webrtc::peer_connection::peer_connection_state::RTCPeerConnectionState;
use webrtc::peer_connection::sdp::session_description::RTCSessionDescription;
use webrtc::track::track_local::TrackLocal;
use webrtc::track::track_local::track_local_static_sample::TrackLocalStaticSample;

pub struct WebRtcSession {
    pub pc: Arc<RTCPeerConnection>,
    pub ws_out: UnboundedSender<Message>,
}

impl WebRtcSession {
    pub async fn new(
        ws_out: UnboundedSender<Message>,
        audio_fanout: Arc<AudioFanout>,
    ) -> Result<Self> {
        // MediaEngine with Opus
        let mut m = MediaEngine::default();
        m.register_default_codecs()?;

        let registry = Registry::new();

        // Configure NAT 1:1 mapping and UDP mux for Docker deployment
        let mut setting_engine = webrtc::api::setting_engine::SettingEngine::default();

        // Use UDP mux to multiplex all WebRTC traffic over a single UDP port
        let udp_port = std::env::var("UDP_PORT")
            .ok()
            .and_then(|s| s.parse::<u16>().ok())
            .unwrap_or(50000);
        info!("Configuring UDP mux on port {}", udp_port);
        let udp_socket = UdpSocket::bind(("0.0.0.0", udp_port)).await?;
        let udp_mux = UDPMuxDefault::new(UDPMuxParams::new(udp_socket));
        setting_engine.set_udp_network(UDPNetwork::Muxed(udp_mux));

        // Configure NAT 1:1 IP mapping
        let host_ip = if let Ok(ip) = std::env::var("HOST_IP") {
            info!("Using HOST_IP from environment: {}", ip);
            Some(ip)
        } else {
            // Auto-detect LAN IP when HOST_IP not set (for non-Docker deployments)
            match get_local_ip() {
                Some(ip) => {
                    info!("Auto-detected local IP: {}", ip);
                    Some(ip)
                }
                None => {
                    info!("Could not auto-detect local IP, using all interfaces");
                    None
                }
            }
        };

        if let Some(ip) = host_ip {
            setting_engine.set_nat_1to1_ips(
                vec![ip],
                webrtc::ice_transport::ice_candidate_type::RTCIceCandidateType::Host,
            );
        }

        let api = APIBuilder::new()
            .with_media_engine(m)
            .with_interceptor_registry(registry)
            .with_setting_engine(setting_engine)
            .build();

        let cfg = RTCConfiguration {
            ice_servers: vec![RTCIceServer {
                urls: vec!["stun:stun.l.google.com:19302".into()],
                ..Default::default()
            }],
            ..Default::default()
        };

        let pc = Arc::new(api.new_peer_connection(cfg).await?);

        // ICE candidates from server -> client
        let ws_out_clone = ws_out.clone();
        pc.on_ice_candidate(Box::new(move |c| {
            let ws_out = ws_out_clone.clone();
            Box::pin(async move {
                if let Some(c) = c {
                    match c.to_json() {
                        Ok(json) => {
                            info!(
                                "server ICE candidate: {} (mid: {:?}, mline: {:?})",
                                json.candidate, json.sdp_mid, json.sdp_mline_index
                            );
                            let msg = serde_json::json!({
                                "type": "candidate",
                                "candidate": json.candidate,
                                "sdpMid": json.sdp_mid,
                                "sdpMLineIndex": json.sdp_mline_index,
                            });
                            let _ = ws_out.send(Message::Text(msg.to_string().into()));
                        }
                        Err(e) => error!("candidate to_json failed: {:#}", e),
                    }
                } else {
                    info!("server ICE gathering complete");
                }
            })
        }));

        // Log connection state changes
        pc.on_peer_connection_state_change(Box::new(move |s: RTCPeerConnectionState| {
            info!("peer connection state changed: {:?}", s);
            Box::pin(async {})
        }));

        // Log ICE connection state changes
        pc.on_ice_connection_state_change(Box::new(move |s| {
            info!("ICE connection state changed: {:?}", s);
            Box::pin(async {})
        }));

        // Log ICE gathering state changes
        pc.on_ice_gathering_state_change(Box::new(move |s| {
            info!("ICE gathering state changed: {:?}", s);
            Box::pin(async {})
        }));

        // Prepare audio track (Opus samples) and tone task
        let track = Arc::new(TrackLocalStaticSample::new(
            webrtc::rtp_transceiver::rtp_codec::RTCRtpCodecCapability {
                mime_type: MIME_TYPE_OPUS.to_string(),
                clock_rate: 48000,
                channels: 1,
                sdp_fmtp_line: "minptime=10;useinbandfec=1".to_string(),
                rtcp_feedback: vec![],
            },
            "audio".to_string(),
            "intercom".to_string(),
        ));

        let sender = pc
            .add_track(Arc::clone(&track) as Arc<dyn TrackLocal + Send + Sync>)
            .await?;

        // Read RTCP in background as required to avoid congestion
        tokio::spawn(async move {
            let mut buf = vec![0u8; 1500];
            loop {
                match sender.read(&mut buf).await {
                    Ok(_) => {}
                    Err(err) => {
                        error!("rtcp read error: {:#}", err);
                        break;
                    }
                }
            }
        });

        // Start audio streaming from DoorBird fanout
        start_audio_stream_task(track.clone(), audio_fanout);

        Ok(Self { pc, ws_out })
    }

    pub async fn set_remote_offer_and_create_answer(
        &self,
        sdp: String,
    ) -> Result<RTCSessionDescription> {
        let offer = RTCSessionDescription::offer(sdp)?;
        self.pc.set_remote_description(offer).await?;
        let answer = self.pc.create_answer(None).await?;
        self.pc.set_local_description(answer).await?;
        let local = self
            .pc
            .local_description()
            .await
            .ok_or_else(|| anyhow::anyhow!("missing local description"))?;
        Ok(local)
    }

    pub async fn add_ice_candidate(
        &self,
        candidate: String,
        sdp_mid: Option<String>,
        sdp_mline_index: Option<u16>,
    ) -> Result<()> {
        let init = RTCIceCandidateInit {
            candidate,
            sdp_mid,
            sdp_mline_index,
            username_fragment: None,
        };
        self.pc.add_ice_candidate(init).await?;
        Ok(())
    }
}

fn start_audio_stream_task(track: Arc<TrackLocalStaticSample>, audio_fanout: Arc<AudioFanout>) {
    tokio::spawn(async move {
        info!("WebRTC audio track subscribed to DoorBird fanout");

        // Subscribe to the audio fanout
        let mut audio_rx = audio_fanout.subscribe().await;

        loop {
            match audio_rx.recv().await {
                Ok(opus_sample) => {
                    // Create WebRTC sample from Opus data
                    let sample = Sample {
                        data: opus_sample.data,
                        duration: opus_sample.duration,
                        ..Default::default()
                    };

                    // Write to WebRTC track
                    if let Err(e) = track.write_sample(&sample).await {
                        error!("track write_sample failed: {:#}", e);
                        break;
                    }
                }
                Err(e) => {
                    error!("audio fanout receive error: {:#}", e);
                    // On broadcast error, try to resubscribe
                    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                    audio_rx = audio_fanout.subscribe().await;
                }
            }
        }

        // Unsubscribe when done
        audio_fanout.unsubscribe().await;
        info!("WebRTC audio track unsubscribed from DoorBird fanout");
    });
}
