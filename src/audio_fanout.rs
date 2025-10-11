//! Audio fanout system for managing DoorBird audio streaming
//!
//! This module implements a fanout queue that:
//! - Connects to DoorBird only when there are active subscribers
//! - Distributes audio to multiple WebRTC clients from a single DoorBird connection
//! - Automatically disconnects after a grace period when all subscribers leave
//! - Handles transcoding from G.711 μ-law to Opus

use crate::audio_transcode::AudioTranscoder;
use anyhow::{Context, Result};
use bytes::Bytes;
use doorbird::Client as DoorBirdClient;
use futures_util::StreamExt;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{broadcast, RwLock};
use tokio::time::sleep;
use tracing::{debug, error, info, warn};

/// Grace period before disconnecting from DoorBird after last subscriber leaves
const AUDIO_GRACE_PERIOD_SECS: u64 = 3;

/// Delay before retrying after connection error
const RECONNECT_DELAY_SECS: u64 = 5;

/// Polling interval for checking subscriber count
const SUBSCRIBER_POLL_INTERVAL_MS: u64 = 100;

/// Opus audio sample ready for WebRTC transmission
#[derive(Clone, Debug)]
pub struct OpusSample {
    /// Opus-encoded audio data
    pub data: Bytes,
    /// Duration of this audio sample (typically 20ms)
    pub duration: Duration,
}

/// State of the audio fanout connection
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ConnectionState {
    Disconnected,
    Connecting,
    Connected,
    Disconnecting,
}

/// Shared state for the audio fanout
struct FanoutState {
    connection_state: ConnectionState,
    subscriber_count: usize,
}

/// Audio fanout manager
///
/// Manages a single DoorBird audio connection and distributes the audio
/// to multiple subscribers (WebRTC clients).
pub struct AudioFanout {
    doorbird_client: DoorBirdClient,
    broadcast_tx: broadcast::Sender<OpusSample>,
    state: Arc<RwLock<FanoutState>>,
}

impl AudioFanout {
    /// Creates a new audio fanout system
    ///
    /// # Arguments
    /// * `doorbird_client` - Configured DoorBird API client
    /// * `buffer_size` - Size of the broadcast buffer (number of samples to buffer)
    pub fn new(doorbird_client: DoorBirdClient, buffer_size: usize) -> Arc<Self> {
        let (broadcast_tx, _) = broadcast::channel(buffer_size);

        let fanout = Arc::new(Self {
            doorbird_client,
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

    /// Subscribe to the audio stream
    ///
    /// Returns a receiver that will get Opus-encoded audio samples.
    /// The connection to DoorBird is automatically established when the first
    /// subscriber joins.
    pub async fn subscribe(&self) -> broadcast::Receiver<OpusSample> {
        let mut state = self.state.write().await;
        state.subscriber_count += 1;
        let count = state.subscriber_count;
        drop(state);

        info!("Audio subscriber added (total: {})", count);

        self.broadcast_tx.subscribe()
    }

    /// Unsubscribe from the audio stream
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

        info!("Audio subscriber removed (remaining: {})", count);
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
            info!("Connecting to DoorBird audio stream...");
            {
                let mut state = self.state.write().await;
                state.connection_state = ConnectionState::Connecting;
            }

            match self.stream_audio().await {
                Ok(_) => {
                    info!("DoorBird audio stream ended normally");
                }
                Err(e) => {
                    error!("DoorBird audio stream error: {:#}", e);
                    // Wait before retry
                    sleep(Duration::from_secs(RECONNECT_DELAY_SECS)).await;
                }
            }

            // Mark as disconnecting
            {
                let mut state = self.state.write().await;
                state.connection_state = ConnectionState::Disconnecting;
            }

            info!("Disconnected from DoorBird audio stream");

            // Grace period: wait to see if subscribers come back
            debug!(
                "Starting {}-second grace period...",
                AUDIO_GRACE_PERIOD_SECS
            );
            sleep(Duration::from_secs(AUDIO_GRACE_PERIOD_SECS)).await;

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

    /// Stream audio from DoorBird and broadcast to subscribers
    async fn stream_audio(&self) -> Result<()> {
        // Get the audio stream from DoorBird
        let mut audio_stream = self
            .doorbird_client
            .audio_receive()
            .await
            .context("Failed to start DoorBird audio stream")?;

        {
            let mut state = self.state.write().await;
            state.connection_state = ConnectionState::Connected;
        }
        info!("Successfully connected to DoorBird audio stream");

        // Create transcoder
        let mut transcoder = AudioTranscoder::new().context("Failed to create audio transcoder")?;

        // Process audio chunks
        while let Some(chunk_result) = audio_stream.next().await {
            // Check if we still have subscribers
            {
                let state = self.state.read().await;
                if state.subscriber_count == 0 {
                    info!("No more subscribers, stopping audio stream");
                    break;
                }
            }

            match chunk_result {
                Ok(chunk) => {
                    // Transcode the chunk
                    match transcoder.process_chunk(&chunk) {
                        Ok(opus_frames) => {
                            // Broadcast each Opus frame
                            for opus_data in opus_frames {
                                let sample = OpusSample {
                                    data: Bytes::from(opus_data),
                                    duration: Duration::from_millis(20),
                                };

                                // Send to all subscribers (ignore if no receivers)
                                let _ = self.broadcast_tx.send(sample);
                            }
                        }
                        Err(e) => {
                            warn!("Audio transcoding error: {:#}", e);
                        }
                    }
                }
                Err(e) => {
                    error!("Error receiving audio chunk: {:#}", e);
                    break;
                }
            }
        }

        // Flush any remaining audio
        match transcoder.flush() {
            Ok(opus_frames) => {
                for opus_data in opus_frames {
                    let sample = OpusSample {
                        data: Bytes::from(opus_data),
                        duration: Duration::from_millis(20),
                    };
                    let _ = self.broadcast_tx.send(sample);
                }
            }
            Err(e) => {
                warn!("Error flushing transcoder: {:#}", e);
            }
        }

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
