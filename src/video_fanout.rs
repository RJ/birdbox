//! Video fanout system for managing DoorBird video streaming
//!
//! This module implements a fanout queue that:
//! - Connects to DoorBird RTSP only when there are active subscribers
//! - Distributes H.264 packets to multiple WebRTC clients from a single DoorBird connection
//! - Automatically reconnects if interrupted by official DoorBird app
//! - Automatically disconnects after a grace period when all subscribers leave
//! - Passes raw H.264 packets without transcoding

use crate::h264_extractor::{H264Extractor, H264Packet};
use anyhow::{Context, Result};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{broadcast, RwLock};
use tokio::time::sleep;
use tracing::{debug, error, info};

/// Grace period before disconnecting from RTSP after last subscriber leaves (longer than audio due to reconnect overhead)
const VIDEO_GRACE_PERIOD_SECS: u64 = 5;

/// Delay before retrying after connection error
const RECONNECT_DELAY_SECS: u64 = 5;

/// Polling interval for checking subscriber count
const SUBSCRIBER_POLL_INTERVAL_MS: u64 = 100;

/// State of the video fanout connection
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ConnectionState {
    Disconnected,
    Connecting,
    Connected,
    Disconnecting,
}

/// Shared state for the video fanout
struct FanoutState {
    connection_state: ConnectionState,
    subscriber_count: usize,
}

/// Video fanout manager
///
/// Manages a single DoorBird RTSP connection and distributes the video
/// to multiple subscribers (WebRTC clients).
pub struct VideoFanout {
    rtsp_url: String,
    rtsp_transport: String,
    broadcast_tx: broadcast::Sender<H264Packet>,
    state: Arc<RwLock<FanoutState>>,
}

impl VideoFanout {
    /// Creates a new video fanout system
    ///
    /// # Arguments
    /// * `rtsp_url` - RTSP URL with embedded credentials
    /// * `buffer_size` - Size of the broadcast buffer (number of frames to buffer)
    /// * `rtsp_transport` - Transport protocol: "tcp" or "udp"
    pub fn new(rtsp_url: String, buffer_size: usize, rtsp_transport: &str) -> Arc<Self> {
        let (broadcast_tx, _) = broadcast::channel(buffer_size);

        let fanout = Arc::new(Self {
            rtsp_url,
            rtsp_transport: rtsp_transport.to_string(),
            broadcast_tx,
            state: Arc::new(RwLock::new(FanoutState {
                connection_state: ConnectionState::Disconnected,
                subscriber_count: 0,
            })),
        });

        // Start the management task
        let fanout_clone = Arc::clone(&fanout);
        tokio::spawn(async move {
            fanout_clone.manage_connection().await;
        });

        fanout
    }

    /// Subscribe to the video stream
    ///
    /// Returns a receiver that will get raw H.264 packets.
    /// The connection to DoorBird is automatically established when the first
    /// subscriber joins.
    pub async fn subscribe(&self) -> broadcast::Receiver<H264Packet> {
        let mut state = self.state.write().await;
        state.subscriber_count += 1;
        let count = state.subscriber_count;
        drop(state);

        info!("Video subscriber added (total: {})", count);

        self.broadcast_tx.subscribe()
    }

    /// Unsubscribe from the video stream
    ///
    /// Should be called when a subscriber is done. The connection to DoorBird
    /// will be closed after a grace period if this was the last subscriber.
    pub async fn unsubscribe(&self) {
        let mut state = self.state.write().await;
        if state.subscriber_count > 0 {
            state.subscriber_count -= 1;
        }
        let count = state.subscriber_count;
        drop(state);

        info!("Video subscriber removed (remaining: {})", count);
    }

    /// Main connection management loop
    async fn manage_connection(self: Arc<Self>) {
        loop {
            // Wait for at least one subscriber
            loop {
                let state = self.state.read().await;
                if state.subscriber_count > 0 {
                    break;
                }
                drop(state);
                sleep(Duration::from_millis(SUBSCRIBER_POLL_INTERVAL_MS)).await;
            }

            // Connect and stream
            info!("Connecting to DoorBird video stream...");
            {
                let mut state = self.state.write().await;
                state.connection_state = ConnectionState::Connecting;
            }

            match self.stream_video().await {
                Ok(_) => {
                    info!("DoorBird video stream ended normally");
                }
                Err(e) => {
                    error!("DoorBird video stream error: {:#}", e);
                    // Wait before retry
                    sleep(Duration::from_secs(RECONNECT_DELAY_SECS)).await;
                }
            }

            // Mark as disconnecting
            {
                let mut state = self.state.write().await;
                state.connection_state = ConnectionState::Disconnecting;
            }

            info!("Disconnected from DoorBird video stream");

            // Grace period: wait to see if subscribers come back
            debug!(
                "Starting {}-second grace period...",
                VIDEO_GRACE_PERIOD_SECS
            );
            sleep(Duration::from_secs(VIDEO_GRACE_PERIOD_SECS)).await;

            // Check if we should reconnect
            let state = self.state.read().await;
            if state.subscriber_count > 0 {
                info!(
                    "Subscribers still present ({}), reconnecting immediately",
                    state.subscriber_count
                );
                drop(state);
                continue;
            } else {
                info!("No subscribers after grace period, staying disconnected");
                let mut state_mut = self.state.write().await;
                state_mut.connection_state = ConnectionState::Disconnected;
                drop(state_mut);
            }
        }
    }

    /// Stream video from DoorBird and broadcast to subscribers
    async fn stream_video(&self) -> Result<()> {
        let rtsp_url = self.rtsp_url.clone();
        let rtsp_transport = self.rtsp_transport.clone();
        let broadcast_tx = self.broadcast_tx.clone();
        let state_clone = Arc::clone(&self.state);

        // Run packet extraction in a spawn_blocking task to avoid Send issues
        let handle = tokio::task::spawn_blocking(move || {
            // Create extractor (this establishes RTSP connection)
            let mut extractor = match H264Extractor::new(rtsp_url, &rtsp_transport) {
                Ok(e) => e,
                Err(e) => {
                    error!("Failed to create H.264 extractor: {:#}", e);
                    return Err(e);
                }
            };

            info!("Successfully connected to DoorBird video stream");

            // Process video packets
            loop {
                // Check if we still have subscribers (polling without await)
                {
                    let state = state_clone.blocking_read();
                    if state.subscriber_count == 0 {
                        info!("No more subscribers, stopping video stream");
                        break;
                    }
                }

                // Get next packet (handles reconnection internally)
                match extractor.next_packet() {
                    Ok(Some(packet)) => {
                        if packet.is_keyframe {
                            debug!("Broadcasting H.264 keyframe");
                        }
                        // Broadcast packet to all subscribers (ignore if no receivers)
                        let _ = broadcast_tx.send(packet);
                    }
                    Ok(None) => {
                        // No packet available (reconnecting or stream ended)
                        // Small sleep to prevent CPU spinning during reconnect
                        std::thread::sleep(std::time::Duration::from_millis(10));
                    }
                    Err(e) => {
                        error!("Error getting video packet: {:#}", e);
                        break;
                    }
                }

                // Small yield to prevent CPU spinning
                std::thread::sleep(std::time::Duration::from_millis(1));
            }

            Ok::<(), anyhow::Error>(())
        });

        {
            let mut state = self.state.write().await;
            state.connection_state = ConnectionState::Connected;
        }

        // Wait for the blocking task to complete
        handle.await.context("Packet extraction task panicked")??;

        Ok(())
    }

    /// Get current subscriber count
    ///
    /// Useful for debugging, monitoring endpoints, or metrics collection.
    #[allow(dead_code)]
    pub async fn subscriber_count(&self) -> usize {
        let state = self.state.read().await;
        state.subscriber_count
    }

    /// Check if currently connected to DoorBird
    ///
    /// Useful for debugging, monitoring endpoints, or health checks.
    #[allow(dead_code)]
    pub async fn is_connected(&self) -> bool {
        let state = self.state.read().await;
        state.connection_state == ConnectionState::Connected
    }
}
