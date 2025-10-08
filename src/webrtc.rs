use anyhow::Result;
use axum::extract::ws::Message;
use std::sync::Arc;
use tokio::sync::mpsc::UnboundedSender;
use tokio::time::{self, Duration};
use tracing::{error, info};
use webrtc::api::APIBuilder;
use webrtc::api::media_engine::{MIME_TYPE_OPUS, MediaEngine};
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
    pub async fn new(ws_out: UnboundedSender<Message>) -> Result<Self> {
        // MediaEngine with Opus
        let mut m = MediaEngine::default();
        m.register_default_codecs()?;

        let registry = Registry::new();
        let api = APIBuilder::new()
            .with_media_engine(m)
            .with_interceptor_registry(registry)
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

        // Start tone generator task
        start_tone_sample_task(track.clone());

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

fn start_tone_sample_task(track: Arc<TrackLocalStaticSample>) {
    tokio::spawn(async move {
        // Generate PCM f32 and encode as Opus using audiopus
        let sample_rate = 48_000f32;
        let channels = audiopus::Channels::Mono;
        let encoder = match audiopus::coder::Encoder::new(
            audiopus::SampleRate::Hz48000,
            channels,
            audiopus::Application::Audio,
        ) {
            Ok(e) => e,
            Err(e) => {
                error!("opus encoder init failed: {:#}", e);
                return;
            }
        };

        let tone_freq = 440f32; // A4
        let frame_ms = 20f32;
        let samples_per_frame = (sample_rate * (frame_ms / 1000.0)) as usize; // 960
        let mut phase: f32 = 0.0;
        let two_pi = std::f32::consts::PI * 2.0;

        let mut interval = time::interval(Duration::from_millis(frame_ms as u64));
        interval.set_missed_tick_behavior(time::MissedTickBehavior::Delay);

        loop {
            interval.tick().await;

            let pcm: Vec<f32> = (0..samples_per_frame)
                .map(|i| {
                    let t = (phase + i as f32) / sample_rate;
                    (two_pi * tone_freq * t).sin() * 0.2
                })
                .collect();
            phase = (phase + samples_per_frame as f32) % sample_rate;

            let mut opus_buf = vec![0u8; 4000];
            let encoded_len = match encoder.encode_float(&pcm, &mut opus_buf) {
                Ok(n) => n,
                Err(e) => {
                    error!("opus encode failed: {:#}", e);
                    continue;
                }
            };

            let sample = Sample {
                data: opus_buf[..encoded_len].to_vec().into(),
                duration: Duration::from_millis(frame_ms as u64),
                ..Default::default()
            };

            if let Err(e) = track.write_sample(&sample).await {
                error!("track write_sample failed: {:#}", e);
                break;
            }
        }
    });
}
