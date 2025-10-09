use crate::audio_fanout::AudioFanout;
use crate::audio_transcode::ReverseAudioTranscoder;
use crate::video_fanout::VideoFanout;
use anyhow::Result;
use axum::extract::ws::Message;
use bytes::Bytes;
use futures_util::stream::StreamExt;
use std::net::IpAddr;
use std::sync::Arc;
use tokio::net::UdpSocket;
use tokio::sync::{Mutex, mpsc::UnboundedSender};
use tracing::{error, info, warn};
use uuid::Uuid;
use webrtc::api::API;
use webrtc::api::APIBuilder;
use webrtc::api::media_engine::{MIME_TYPE_H264, MIME_TYPE_OPUS, MediaEngine};
use webrtc::ice::udp_mux::*;
use webrtc::ice::udp_network::UDPNetwork;
use webrtc::ice_transport::ice_candidate::RTCIceCandidateInit;
use webrtc::interceptor::registry::Registry;
use webrtc::media::Sample;
use webrtc::peer_connection::RTCPeerConnection;
use webrtc::peer_connection::configuration::RTCConfiguration;
use webrtc::peer_connection::peer_connection_state::RTCPeerConnectionState;
use webrtc::peer_connection::sdp::session_description::RTCSessionDescription;
use webrtc::track::track_local::TrackLocal;
use webrtc::track::track_local::track_local_static_sample::TrackLocalStaticSample;

/// Auto-detect the local LAN IP address
/// Returns the first non-loopback IPv4 address found on any network interface
fn get_local_ip() -> Option<String> {
    use std::net::UdpSocket as StdUdpSocket;

    // Trick: Create a UDP socket connected to a public IP (doesn't actually send data)
    // The OS will select the appropriate local interface for routing
    let socket = StdUdpSocket::bind("0.0.0.0:0").ok()?;
    socket.connect("8.8.8.8:80").ok()?;
    let local_addr = socket.local_addr().ok()?;

    match local_addr.ip() {
        IpAddr::V4(ip) if !ip.is_loopback() => Some(ip.to_string()),
        _ => None,
    }
}

/// Bind a UDP socket with SO_REUSEADDR to allow quick rebinding after close
async fn bind_udp_socket(addr: &str) -> Result<UdpSocket> {
    use socket2::{Domain, Protocol, Socket, Type};
    use std::net::SocketAddr;

    let addr: SocketAddr = addr.parse()?;
    let domain = if addr.is_ipv4() {
        Domain::IPV4
    } else {
        Domain::IPV6
    };

    let socket = Socket::new(domain, Type::DGRAM, Some(Protocol::UDP))?;

    // Enable SO_REUSEADDR to allow quick rebinding after connection close
    socket.set_reuse_address(true)?;

    // On Unix systems, also set SO_REUSEPORT for better behavior
    #[cfg(unix)]
    socket.set_reuse_port(true)?;

    socket.bind(&addr.into())?;
    socket.set_nonblocking(true)?;

    Ok(UdpSocket::from_std(socket.into())?)
}

/// Shared WebRTC infrastructure - created once at startup and shared across all sessions
pub struct WebRtcInfra {
    api: API,
}

impl WebRtcInfra {
    /// Initialize the shared WebRTC infrastructure with a persistent UDP mux
    pub async fn new() -> Result<Arc<Self>> {
        // MediaEngine with Opus
        let mut m = MediaEngine::default();
        m.register_default_codecs()?;

        let registry = Registry::new();

        // Configure NAT 1:1 mapping and UDP mux for Docker deployment
        let mut setting_engine = webrtc::api::setting_engine::SettingEngine::default();

        // Determine which IP to use for WebRTC
        let host_ip = if let Ok(ip) = std::env::var("HOST_IP") {
            info!("🌐 Using HOST_IP from environment: {}", ip);
            ip
        } else {
            // Auto-detect LAN IP when HOST_IP not set (for non-Docker deployments)
            match get_local_ip() {
                Some(ip) => {
                    info!("🌐 Auto-detected local IP: {}", ip);
                    ip
                }
                None => {
                    info!("🌐 Could not auto-detect local IP, binding to all interfaces");
                    "0.0.0.0".to_string()
                }
            }
        };

        // Use UDP mux to multiplex all WebRTC traffic over a single UDP port
        let udp_port = std::env::var("UDP_PORT")
            .ok()
            .and_then(|s| s.parse::<u16>().ok())
            .unwrap_or(50000);

        // Bind to specific IP to ensure only that interface is used for ICE candidates
        // Try to bind to the specific IP first (for native/host deployments)
        // If that fails (e.g., in Docker where container doesn't have HOST_IP), bind to 0.0.0.0
        let bind_addr = format!("{}:{}", host_ip, udp_port);
        let (udp_socket, actual_bind_ip) = match bind_udp_socket(&bind_addr).await {
            Ok(socket) => {
                info!(
                    "🌐 Bound WebRTC UDP socket to {} (shared across all sessions)",
                    bind_addr
                );
                (socket, host_ip.clone())
            }
            Err(e) => {
                info!(
                    "🌐 {}:{} is unbindable, using 0.0.0.0 instead – probably in docker. [{}]",
                    bind_addr, udp_port, e
                );
                let fallback_addr = format!("0.0.0.0:{}", udp_port);
                let socket = bind_udp_socket(&fallback_addr).await?;
                (socket, host_ip.clone())
            }
        };

        let udp_mux = UDPMuxDefault::new(UDPMuxParams::new(udp_socket));
        setting_engine.set_udp_network(UDPNetwork::Muxed(udp_mux));

        // Disable mDNS to prevent .local candidates (we want specific IP only)
        if actual_bind_ip != "0.0.0.0" {
            setting_engine
                .set_ice_multicast_dns_mode(webrtc::ice::mdns::MulticastDnsMode::Disabled);
            info!("🌐 Disabled mDNS candidates (using specific IP only)");
        }

        // Set NAT 1:1 mapping for non-0.0.0.0 IPs (especially important for Docker)
        if actual_bind_ip != "0.0.0.0" {
            info!(
                "🌐 Setting NAT 1:1 mapping to advertise IP: {}",
                actual_bind_ip
            );
            setting_engine.set_nat_1to1_ips(
                vec![actual_bind_ip.clone()],
                webrtc::ice_transport::ice_candidate_type::RTCIceCandidateType::Host,
            );

            // Filter ICE candidates to only allow the specific IP we want
            /*
            // TOO STRICT FOR MULTI INTERFACE SETUPS WITH TAILSCALE
            let filter_ip = actual_bind_ip.clone();
            setting_engine.set_ip_filter(Box::new(move |ip: IpAddr| {
                let ip_str = ip.to_string();
                let allowed = ip_str == filter_ip;
                if !allowed {
                    info!(
                        "🌐 Filtered out ICE candidate IP: {} (only allowing {})",
                        ip_str, filter_ip
                    );
                }
                allowed
            }));
            info!("🌐 Set IP filter to only allow: {}", actual_bind_ip);
            */
        }

        let api = APIBuilder::new()
            .with_media_engine(m)
            .with_interceptor_registry(registry)
            .with_setting_engine(setting_engine)
            .build();

        Ok(Arc::new(Self { api }))
    }
}

/// PTT transmission handle - when dropped, stops transmission
struct PttTransmitHandle {
    stop_tx: Option<tokio::sync::oneshot::Sender<()>>,
}

impl Drop for PttTransmitHandle {
    fn drop(&mut self) {
        if let Some(tx) = self.stop_tx.take() {
            let _ = tx.send(());
        }
    }
}

pub struct WebRtcSession {
    pub pc: Arc<RTCPeerConnection>,
    pub ws_out: UnboundedSender<Message>,
    #[allow(dead_code)]
    ptt_state: Arc<crate::PttState>,
    doorbird_client: doorbird::Client,
    session_id: Uuid,
    /// Channel for sending Opus audio from client to PTT transcoder
    ptt_audio_tx: Arc<Mutex<Option<tokio::sync::mpsc::UnboundedSender<Bytes>>>>,
    /// Handle for current PTT transmission (if active)
    ptt_handle: Arc<Mutex<Option<PttTransmitHandle>>>,
}

impl WebRtcSession {
    pub async fn new(
        infra: Arc<WebRtcInfra>,
        ws_out: UnboundedSender<Message>,
        audio_fanout: Arc<AudioFanout>,
        video_fanout: Arc<VideoFanout>,
        ptt_state: Arc<crate::PttState>,
        doorbird_client: doorbird::Client,
        session_id: Uuid,
    ) -> Result<Self> {
        // No STUN/TURN servers needed for client-server architecture
        // where server has known IP and client connects directly
        let cfg = RTCConfiguration::default();

        let pc = Arc::new(infra.api.new_peer_connection(cfg).await?);

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

        // Prepare audio track (Opus samples) for sending to client
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

        // Set up handler to read incoming audio from client for PTT
        let ptt_audio_tx: Arc<Mutex<Option<tokio::sync::mpsc::UnboundedSender<Bytes>>>> =
            Arc::new(Mutex::new(None));
        let ptt_audio_tx_clone = ptt_audio_tx.clone();

        // Set up on_track handler to receive audio from client when they start transmitting
        pc.on_track(Box::new(move |track, _receiver, _transceiver| {
            let ptt_audio_tx = ptt_audio_tx_clone.clone();
            Box::pin(async move {
                info!(
                    "Received remote audio track from client: kind={}",
                    track.kind()
                );

                tokio::spawn(async move {
                    info!("Starting to read incoming audio from client");
                    let mut packet_count = 0;
                    loop {
                        match track.read_rtp().await {
                            Ok((rtp_packet, _)) => {
                                packet_count += 1;
                                if packet_count % 50 == 0 {
                                    info!("Received {} RTP packets from client", packet_count);
                                }

                                // Extract Opus payload
                                let opus_data = Bytes::copy_from_slice(&rtp_packet.payload);

                                // Send to PTT transcoder if active
                                let tx_opt = ptt_audio_tx.lock().await;
                                if let Some(tx) = tx_opt.as_ref() {
                                    if tx.send(opus_data).is_err() {
                                        // Channel closed, stop reading
                                        info!(
                                            "PTT audio channel closed after {} packets",
                                            packet_count
                                        );
                                        break;
                                    }
                                }
                            }
                            Err(e) => {
                                if packet_count > 0 {
                                    info!(
                                        "Stopped reading audio after {} packets: {:#}",
                                        packet_count, e
                                    );
                                }
                                break;
                            }
                        }
                    }
                    info!(
                        "Stopped reading incoming audio track (received {} packets total)",
                        packet_count
                    );
                });
            })
        }));

        // Start audio streaming from DoorBird fanout
        start_audio_stream_task(track.clone(), audio_fanout);

        // Prepare video track (H.264) for sending to client
        let video_track = Arc::new(TrackLocalStaticSample::new(
            webrtc::rtp_transceiver::rtp_codec::RTCRtpCodecCapability {
                mime_type: MIME_TYPE_H264.to_string(),
                clock_rate: 90000, // Standard for H.264
                channels: 0,
                sdp_fmtp_line:
                    "level-asymmetry-allowed=1;packetization-mode=1;profile-level-id=42e01f"
                        .to_string(),
                rtcp_feedback: vec![],
            },
            "video".to_string(),
            "doorbird-video".to_string(),
        ));

        let video_sender = pc
            .add_track(Arc::clone(&video_track) as Arc<dyn TrackLocal + Send + Sync>)
            .await?;

        // Read RTCP for video track in background
        tokio::spawn(async move {
            let mut buf = vec![0u8; 1500];
            loop {
                match video_sender.read(&mut buf).await {
                    Ok(_) => {}
                    Err(err) => {
                        error!("video rtcp read error: {:#}", err);
                        break;
                    }
                }
            }
        });

        // Start video streaming from DoorBird fanout
        start_video_stream_task(video_track.clone(), video_fanout);

        Ok(Self {
            pc,
            ws_out,
            ptt_state,
            doorbird_client,
            session_id,
            ptt_audio_tx,
            ptt_handle: Arc::new(Mutex::new(None)),
        })
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

    /// Start push-to-talk audio transmission to DoorBird
    pub async fn start_ptt(&self) -> Result<()> {
        info!("Starting PTT for session {}", self.session_id);

        // Create channel for audio data
        let (audio_tx, mut audio_rx) = tokio::sync::mpsc::unbounded_channel::<Bytes>();

        // Set the channel so on_track can send to it
        {
            let mut tx_lock = self.ptt_audio_tx.lock().await;
            *tx_lock = Some(audio_tx);
        }

        // Create stop signal
        let (stop_tx, mut stop_rx) = tokio::sync::oneshot::channel();

        // Store handle
        {
            let mut handle_lock = self.ptt_handle.lock().await;
            *handle_lock = Some(PttTransmitHandle {
                stop_tx: Some(stop_tx),
            });
        }

        // Spawn task to transcode and transmit audio
        let doorbird_client = self.doorbird_client.clone();
        let session_id = self.session_id;

        tokio::spawn(async move {
            info!("PTT transmission task started for session {}", session_id);

            // Create reverse transcoder
            let mut transcoder = match ReverseAudioTranscoder::new() {
                Ok(t) => t,
                Err(e) => {
                    error!("Failed to create reverse transcoder: {:#}", e);
                    return;
                }
            };

            // Create stream of G.711 μ-law data
            let (ulaw_tx, ulaw_rx) = tokio::sync::mpsc::unbounded_channel::<Bytes>();

            // Spawn transcoding task
            let transcode_task = tokio::spawn(async move {
                let mut opus_count = 0;
                let mut ulaw_count = 0;
                while let Some(opus_data) = audio_rx.recv().await {
                    opus_count += 1;
                    // Transcode Opus to G.711 μ-law
                    match transcoder.process_chunk(&opus_data) {
                        Ok(ulaw_frames) => {
                            for frame in ulaw_frames {
                                ulaw_count += 1;
                                if ulaw_tx.send(Bytes::from(frame)).is_err() {
                                    // Channel closed
                                    info!(
                                        "µ-law channel closed after {} opus packets, {} µ-law frames",
                                        opus_count, ulaw_count
                                    );
                                    break;
                                }
                            }
                        }
                        Err(e) => {
                            warn!("Transcoding error: {:#}", e);
                        }
                    }
                }

                // Flush any remaining data
                if let Ok(ulaw_frames) = transcoder.flush() {
                    for frame in ulaw_frames {
                        ulaw_count += 1;
                        let _ = ulaw_tx.send(Bytes::from(frame));
                    }
                }

                info!(
                    "Transcoding task finished: {} opus packets -> {} µ-law frames",
                    opus_count, ulaw_count
                );
            });

            // Create stream for DoorBird
            let ulaw_stream = tokio_stream::wrappers::UnboundedReceiverStream::new(ulaw_rx);
            let result_stream = ulaw_stream.map(Ok::<Bytes, anyhow::Error>);

            // Transmit to DoorBird (this blocks until stream ends or error)
            let transmit_result = tokio::select! {
                result = doorbird_client.audio_transmit(result_stream) => {
                    result
                }
                _ = &mut stop_rx => {
                    info!("PTT transmission stopped by user");
                    Ok(())
                }
            };

            // Stop transcoding task
            transcode_task.abort();

            match transmit_result {
                Ok(_) => info!("PTT transmission completed for session {}", session_id),
                Err(e) => error!("PTT transmission error for session {}: {:#}", session_id, e),
            }
        });

        Ok(())
    }

    /// Stop push-to-talk audio transmission
    pub async fn stop_ptt(&self) {
        info!("Stopping PTT for session {}", self.session_id);

        // Clear the audio channel
        {
            let mut tx_lock = self.ptt_audio_tx.lock().await;
            *tx_lock = None;
        }

        // Drop the handle (triggers stop signal)
        {
            let mut handle_lock = self.ptt_handle.lock().await;
            *handle_lock = None;
        }
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

fn start_video_stream_task(track: Arc<TrackLocalStaticSample>, video_fanout: Arc<VideoFanout>) {
    tokio::spawn(async move {
        info!("WebRTC video track subscribed to DoorBird fanout");

        // Subscribe to the video fanout
        let mut video_rx = video_fanout.subscribe().await;

        loop {
            match video_rx.recv().await {
                Ok(h264_packet) => {
                    // Create WebRTC sample from H.264 packet
                    // Use a fixed duration for low latency - DoorBird typically streams at 10-12 fps
                    // Using 83ms (~12fps) as duration, actual timing handled by WebRTC
                    let sample = Sample {
                        data: h264_packet.data,
                        duration: std::time::Duration::from_millis(83),
                        ..Default::default()
                    };

                    // Write to WebRTC track immediately
                    if let Err(e) = track.write_sample(&sample).await {
                        error!("video track write_sample failed: {:#}", e);
                        break;
                    }
                }
                Err(e) => {
                    error!("video fanout receive error: {:#}", e);
                    // On broadcast error, try to resubscribe
                    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                    video_rx = video_fanout.subscribe().await;
                }
            }
        }

        // Unsubscribe when done
        video_fanout.unsubscribe().await;
        info!("WebRTC video track unsubscribed from DoorBird fanout");
    });
}
