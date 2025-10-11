//! H.264 packet extraction from DoorBird RTSP stream
//!
//! This module extracts raw H.264 packets directly from the RTSP stream
//! without decoding, for efficient WebRTC video transmission.

use anyhow::{Context, Result};
use bytes::Bytes;
use ffmpeg_next as ffmpeg;
use std::time::{Duration, Instant};
use tracing::{debug, error, info, warn};

/// H.264 packet ready for WebRTC transmission
#[derive(Clone, Debug)]
pub struct H264Packet {
    /// Raw H.264 packet data
    pub data: Bytes,
    #[allow(unused)]
    /// Packet timestamp
    pub timestamp: Duration,
    /// Whether this is a keyframe
    pub is_keyframe: bool,
}

/// H.264 packet extractor from RTSP stream
pub struct H264Extractor {
    rtsp_url: String,
    rtsp_transport: String,
    input_context: Option<ffmpeg::format::context::Input>,
    video_stream_index: Option<usize>,
    time_base: Option<ffmpeg::Rational>,
    is_reconnecting: bool,
    last_reconnect_attempt: Instant,
}

impl H264Extractor {
    /// Creates a new H.264 packet extractor
    ///
    /// # Arguments
    /// * `rtsp_url` - RTSP URL with embedded credentials
    /// * `rtsp_transport` - Transport protocol: "tcp" or "udp"
    pub fn new(rtsp_url: String, rtsp_transport: &str) -> Result<Self> {
        info!("Initializing ffmpeg for H.264 extraction");
        ffmpeg::init().context("Failed to initialize ffmpeg")?;

        let mut extractor = Self {
            rtsp_url,
            rtsp_transport: rtsp_transport.to_string(),
            input_context: None,
            video_stream_index: None,
            time_base: None,
            is_reconnecting: false,
            last_reconnect_attempt: Instant::now(),
        };

        extractor.connect()?;
        Ok(extractor)
    }

    /// Establishes connection to RTSP stream
    fn connect(&mut self) -> Result<()> {
        let censored_url = if let Some(at_pos) = self.rtsp_url.find('@') {
            format!("rtsp://*****@{}", &self.rtsp_url[at_pos + 1..])
        } else {
            self.rtsp_url.clone()
        };
        info!(
            "Connecting to RTSP stream: {} (transport: {})",
            censored_url, self.rtsp_transport
        );

        // Open RTSP input with low-latency options
        let mut options = ffmpeg::Dictionary::new();
        options.set("rtsp_transport", &self.rtsp_transport); // Use configured transport (tcp/udp)
        options.set("fflags", "nobuffer"); // Disable buffering
        options.set("flags", "low_delay"); // Enable low delay mode
        options.set("max_delay", "0"); // Minimize delay

        let input = ffmpeg::format::input_with_dictionary(&self.rtsp_url, options)
            .context("Failed to open RTSP stream")?;

        // Find video stream
        let video_stream = input
            .streams()
            .best(ffmpeg::media::Type::Video)
            .context("No video stream found in RTSP stream")?;

        let video_stream_index = video_stream.index();
        let time_base = video_stream.time_base();

        info!(
            "Video stream found: index={}, time_base={}/{}",
            video_stream_index,
            time_base.numerator(),
            time_base.denominator()
        );

        // Verify it's H.264
        let codec_id = video_stream.parameters().id();
        if codec_id != ffmpeg::codec::Id::H264 {
            anyhow::bail!(
                "Expected H.264 codec, but got {:?}. Cannot proceed with WebRTC streaming.",
                codec_id
            );
        }

        self.input_context = Some(input);
        self.video_stream_index = Some(video_stream_index);
        self.time_base = Some(time_base);
        self.is_reconnecting = false;

        info!("Successfully connected to RTSP stream");
        Ok(())
    }

    /// Attempts to reconnect to the RTSP stream
    fn reconnect(&mut self) -> Result<()> {
        warn!("Attempting to reconnect to RTSP stream...");

        // Clean up existing connections
        self.input_context = None;
        self.video_stream_index = None;
        self.time_base = None;

        // Try to reconnect
        self.connect()
    }

    /// Returns the next H.264 packet
    ///
    /// On error, attempts reconnection
    pub fn next_packet(&mut self) -> Result<Option<H264Packet>> {
        // If we're reconnecting, check if it's time to retry
        if self.is_reconnecting {
            let elapsed = self.last_reconnect_attempt.elapsed();
            if elapsed >= Duration::from_secs(2) {
                self.last_reconnect_attempt = Instant::now();
                match self.reconnect() {
                    Ok(_) => {
                        info!("Reconnection successful");
                        self.is_reconnecting = false;
                        // Try to get a packet immediately
                        return self.get_next_packet();
                    }
                    Err(e) => {
                        warn!("Reconnection failed: {:#}", e);
                        // Return None while reconnecting
                        return Ok(None);
                    }
                }
            } else {
                // Still in reconnect cooldown
                return Ok(None);
            }
        }

        // Try to get next packet
        match self.get_next_packet() {
            Ok(packet) => Ok(packet),
            Err(e) => {
                error!("Error getting packet: {:#}", e);
                self.is_reconnecting = true;
                self.last_reconnect_attempt = Instant::now();
                Ok(None)
            }
        }
    }

    /// Gets the next packet from the stream (internal, can fail)
    fn get_next_packet(&mut self) -> Result<Option<H264Packet>> {
        let input = self.input_context.as_mut().context("No input context")?;
        let video_stream_index = self.video_stream_index.context("No video stream index")?;
        let time_base = self.time_base.context("No time base")?;

        // Read packets until we get a video packet
        for (stream, packet) in input.packets() {
            if stream.index() != video_stream_index {
                continue;
            }

            // Get packet data
            let packet_data = packet.data().unwrap_or(&[]).to_vec();

            if packet_data.is_empty() {
                continue;
            }

            // Calculate timestamp
            let pts = packet.pts().unwrap_or(0);
            let timestamp_secs =
                (pts as f64) * time_base.numerator() as f64 / time_base.denominator() as f64;
            let timestamp = Duration::from_secs_f64(timestamp_secs);

            // Check if this is a keyframe
            let is_keyframe = packet.is_key();

            debug!(
                "Extracted H.264 packet: {} bytes, keyframe={}, timestamp={:.3}s",
                packet_data.len(),
                is_keyframe,
                timestamp_secs
            );

            return Ok(Some(H264Packet {
                data: Bytes::from(packet_data),
                timestamp,
                is_keyframe,
            }));
        }

        // No more packets
        Ok(None)
    }
}
