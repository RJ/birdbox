//! # DoorBird API Client
//!
//! This crate provides a Rust client for interacting with DoorBird smart doorbell devices
//! via their LAN-2-LAN HTTP API.
//!
//! The DoorBird API allows you to retrieve device information, stream audio, capture images,
//! control relays, and more. This library currently implements the essential features needed
//! for audio streaming integration.
//!
//! ## API Reference
//!
//! This implementation is based on the DoorBird LAN-2-LAN API documentation, Revision 0.36
//! (November 13, 2023). For full API details, refer to the official DoorBird API documentation.
//!
//! ## Authentication
//!
//! The client uses HTTP Basic Authentication as defined in RFC 2617. You must provide
//! valid DoorBird credentials (username and password) when creating a client instance.
//!
//! ## Example
//!
//! ```no_run
//! use doorbird::Client;
//!
//! # async fn example() -> anyhow::Result<()> {
//! let client = Client::new(
//!     "http://192.168.1.100".to_string(),
//!     "user0001".to_string(),
//!     "password".to_string(),
//! );
//!
//! // Get device information
//! let info = client.info().await?;
//! if let Some(device_type) = &info.device_type {
//!     println!("Device: {}", device_type);
//! }
//! println!("Firmware: {}", info.firmware);
//!
//! // Stream audio (returns raw G.711 μ-law bytes at 8kHz)
//! let mut audio_stream = client.audio_receive().await?;
//! # Ok(())
//! # }
//! ```

use anyhow::{Context, Result};
use bytes::Bytes;
use futures_util::{Stream, StreamExt};
use serde::Deserialize;
use std::pin::Pin;
use tracing::{debug, info};

/// A client for interacting with DoorBird devices via their HTTP API.
///
/// The client maintains connection information and credentials for authenticating
/// with a DoorBird device on the local network.
#[derive(Clone)]
pub struct Client {
    /// Base URL of the DoorBird device (e.g., "http://192.168.1.100")
    base_url: String,
    /// Username for HTTP Basic Authentication
    username: String,
    /// Password for HTTP Basic Authentication
    password: String,
    /// Internal HTTP client
    client: reqwest::Client,
}

/// Video quality/resolution options for RTSP streaming
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VideoQuality {
    /// Default resolution (device-specific)
    Default,
    /// 720p resolution (supported by D10x/D21x devices)
    P720,
    /// 1080p resolution (supported by D11x devices)
    P1080,
}

/// Event received from the DoorBird device's event monitor stream.
///
/// These events are produced by the `/bha-api/monitor.cgi` endpoint and represent
/// state changes of the doorbell button and motion sensor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MonitorEvent {
    /// Doorbell button pressed (state H)
    /// Released events (state L) are ignored
    Doorbell,

    /// Motion sensor event
    /// - `active: true` means motion detected (state H)
    /// - `active: false` means motion cleared (state L)
    MotionSensor { active: bool },
}

/// Device information returned from the `/bha-api/info.cgi` endpoint.
///
/// Contains firmware version, build number, MAC address, and available relays.
#[derive(Debug, Clone, Deserialize)]
pub struct DeviceInfo {
    /// Firmware version string (e.g., "000109")
    #[serde(rename = "FIRMWARE")]
    pub firmware: String,

    /// Build number string (e.g., "15120529")
    #[serde(rename = "BUILD_NUMBER")]
    pub build_number: String,

    /// Primary MAC address (e.g., "1CCAE3700000")
    #[serde(rename = "PRIMARY_MAC_ADDR")]
    pub primary_mac_addr: Option<String>,

    /// Available relays (physical and paired DoorController relays)
    /// Example: ["1", "2", "gggaaa@1", "gggaaa@2"]
    #[serde(rename = "RELAYS")]
    pub relays: Option<Vec<String>>,

    /// Device type string (e.g., "DoorBird D101")
    #[serde(rename = "DEVICE-TYPE")]
    pub device_type: Option<String>,
}

impl DeviceInfo {
    /// Returns `true` if the device supports 1080p video resolution.
    ///
    /// This is determined by checking if the device type contains "D11".
    ///
    /// # Example
    ///
    /// ```
    /// # use doorbird::DeviceInfo;
    /// let info = DeviceInfo {
    ///     firmware: "000109".to_string(),
    ///     build_number: "15120529".to_string(),
    ///     primary_mac_addr: None,
    ///     relays: None,
    ///     device_type: Some("DoorBird D1101".to_string()),
    /// };
    /// assert!(info.supports_1080p());
    /// ```
    pub fn supports_1080p(&self) -> bool {
        self.device_type
            .as_ref()
            .map(|dt| dt.to_uppercase().contains("D11"))
            .unwrap_or(false)
    }

    /// Returns `true` if the device supports 720p video resolution.
    ///
    /// This is determined by checking if the device supports 1080p (which implies 720p),
    /// or if the device type contains "D10" or "D21".
    ///
    /// # Example
    ///
    /// ```
    /// # use doorbird::DeviceInfo;
    /// let info = DeviceInfo {
    ///     firmware: "000109".to_string(),
    ///     build_number: "15120529".to_string(),
    ///     primary_mac_addr: None,
    ///     relays: None,
    ///     device_type: Some("DoorBird D1001".to_string()),
    /// };
    /// assert!(info.supports_720p());
    /// ```
    pub fn supports_720p(&self) -> bool {
        // 1080p implies 720p
        if self.supports_1080p() {
            return true;
        }

        self.device_type
            .as_ref()
            .map(|dt| {
                let dt_upper = dt.to_uppercase();
                dt_upper.contains("D10") || dt_upper.contains("D21")
            })
            .unwrap_or(false)
    }
}

/// Response wrapper for the info endpoint
#[derive(Debug, Deserialize)]
struct InfoResponse {
    #[serde(rename = "BHA")]
    bha: InfoResponseBha,
}

#[derive(Debug, Deserialize)]
struct InfoResponseBha {
    #[serde(rename = "VERSION")]
    version: Vec<DeviceInfo>,
}

impl Client {
    /// Creates a new DoorBird API client.
    ///
    /// # Arguments
    ///
    /// * `base_url` - The base URL of the DoorBird device (e.g., "http://192.168.1.100")
    /// * `username` - Username for HTTP Basic Authentication
    /// * `password` - Password for HTTP Basic Authentication
    ///
    /// # Example
    ///
    /// ```
    /// use doorbird::Client;
    ///
    /// let client = Client::new(
    ///     "http://192.168.1.100".to_string(),
    ///     "abcdef0001".to_string(),
    ///     "my_password".to_string(),
    /// );
    /// ```
    pub fn new(base_url: String, username: String, password: String) -> Self {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .unwrap();

        Self {
            base_url,
            username,
            password,
            client,
        }
    }

    /// Retrieves device information from the DoorBird.
    ///
    /// **API Endpoint:** `GET /bha-api/info.cgi`
    ///
    /// **Required Permission:** Valid user
    ///
    /// **Returns:** Device information including firmware version, build number,
    /// MAC address, available relays, and device type.
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use doorbird::Client;
    /// # async fn example() -> anyhow::Result<()> {
    /// # let client = Client::new("http://192.168.1.100".into(), "user".into(), "pass".into());
    /// let info = client.info().await?;
    /// println!("Firmware: {}", info.firmware);
    /// println!("Build: {}", info.build_number);
    /// if let Some(device_type) = &info.device_type {
    ///     println!("Device: {}", device_type);
    /// }
    /// # Ok(())
    /// # }
    /// ```
    pub async fn info(&self) -> Result<DeviceInfo> {
        let url = format!("{}/bha-api/info.cgi", self.base_url);
        debug!("Fetching device info from {}", url);

        let response = self
            .client
            .get(&url)
            .basic_auth(&self.username, Some(&self.password))
            .send()
            .await
            .context("Failed to send info request")?;

        let status = response.status();
        if !status.is_success() {
            anyhow::bail!("Info request failed with status: {}", status);
        }

        let info_response: InfoResponse = response
            .json()
            .await
            .context("Failed to parse info response")?;

        info_response
            .bha
            .version
            .into_iter()
            .next()
            .ok_or_else(|| anyhow::anyhow!("No device info in response"))
    }

    /// Starts receiving live audio from the DoorBird device.
    ///
    /// **API Endpoint:** `GET /bha-api/audio-receive.cgi`
    ///
    /// **Required Permission:** Valid user with "watch always" permission or
    /// ring event in the past 5 minutes
    ///
    /// **Audio Format:** Returns raw G.711 μ-law encoded audio data at 8kHz sample rate,
    /// mono channel. The audio data is streamed continuously as raw bytes.
    ///
    /// **Note:** The DoorBird device handles only one audio consumer at a time.
    /// The connection can be interrupted if the official DoorBird app requests the stream,
    /// as it has precedence over LAN API users.
    ///
    /// # Returns
    ///
    /// A stream of `Bytes` containing raw G.711 μ-law audio data. Each chunk contains
    /// multiple audio samples that need to be decoded using a G.711 μ-law decoder.
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use doorbird::Client;
    /// # use futures_util::StreamExt;
    /// # async fn example() -> anyhow::Result<()> {
    /// # let client = Client::new("http://192.168.1.100".into(), "user".into(), "pass".into());
    /// let mut audio_stream = client.audio_receive().await?;
    ///
    /// while let Some(chunk_result) = audio_stream.next().await {
    ///     match chunk_result {
    ///         Ok(bytes) => {
    ///             // Process raw G.711 μ-law bytes here
    ///             println!("Received {} bytes of audio data", bytes.len());
    ///         }
    ///         Err(e) => {
    ///             eprintln!("Stream error: {}", e);
    ///             break;
    ///         }
    ///     }
    /// }
    /// # Ok(())
    /// # }
    /// ```
    pub async fn audio_receive(&self) -> Result<Pin<Box<dyn Stream<Item = Result<Bytes>> + Send>>> {
        let url = format!("{}/bha-api/audio-receive.cgi", self.base_url);
        info!("Connecting to DoorBird audio stream at {}", url);

        let response = self
            .client
            .get(&url)
            .basic_auth(&self.username, Some(&self.password))
            .timeout(std::time::Duration::from_secs(3600)) // 1 hour timeout for streaming
            .send()
            .await
            .context("Failed to send audio receive request")?;

        let status = response.status();
        if !status.is_success() {
            anyhow::bail!("Audio receive request failed with status: {}", status);
        }

        let stream = response.bytes_stream();
        let error_mapped_stream = futures_util::StreamExt::map(stream, |result| {
            result.context("Error reading audio stream")
        });

        Ok(Box::pin(error_mapped_stream))
    }

    /// Transmits live audio to the DoorBird device.
    ///
    /// **API Endpoint:** `POST /bha-api/audio-transmit.cgi`
    ///
    /// **Required Permission:** Valid user with "watch always" permission or
    /// ring event in the past 5 minutes
    ///
    /// **Audio Format:** Expects raw G.711 μ-law encoded audio data at 8kHz sample rate,
    /// mono channel. The audio data should be streamed continuously as raw bytes.
    ///
    /// **Important:** Only one consumer can transmit audio (talk) at the same time.
    /// If another client is already transmitting, this request will be rejected by
    /// the DoorBird device (typically with HTTP 204 or connection refusal).
    ///
    /// **Note:** The audio connection can be interrupted at any time if the official
    /// DoorBird app requests the stream, as it has precedence over LAN API users.
    ///
    /// # Arguments
    ///
    /// * `audio_stream` - A stream of `Bytes` containing raw G.711 μ-law audio data.
    ///   The stream should provide audio at approximately 8000 bytes per second (8kHz sample rate).
    ///
    /// # Returns
    ///
    /// Returns `Ok(())` when the stream completes successfully, or an error if the
    /// connection fails or is rejected.
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use doorbird::Client;
    /// # use bytes::Bytes;
    /// # use futures_util::stream;
    /// # async fn example() -> anyhow::Result<()> {
    /// # let client = Client::new("http://192.168.1.100".into(), "user".into(), "pass".into());
    /// // Create a stream of G.711 μ-law audio data
    /// let audio_data = vec![0xFF; 8000]; // 1 second of silence
    /// let audio_stream = stream::once(async { Ok(Bytes::from(audio_data)) });
    ///
    /// // Transmit to DoorBird
    /// client.audio_transmit(audio_stream).await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn audio_transmit(
        &self,
        audio_stream: impl futures_util::Stream<Item = Result<Bytes>> + Send + 'static,
    ) -> Result<()> {
        let url = format!("{}/bha-api/audio-transmit.cgi", self.base_url);
        info!("Starting audio transmission to DoorBird at {}", url);

        // Convert stream to reqwest Body
        let body = reqwest::Body::wrap_stream(audio_stream);

        let response = self
            .client
            .post(&url)
            .basic_auth(&self.username, Some(&self.password))
            .header("Content-Type", "audio/basic")
            .header("Content-Length", "9999999")
            .header("Connection", "Keep-Alive")
            .header("Cache-Control", "no-cache")
            .body(body)
            .send()
            .await
            .context("Failed to send audio transmit request")?;

        let status = response.status();
        if status.is_success() {
            info!("Audio transmission completed successfully");
            Ok(())
        } else if status.as_u16() == 204 {
            anyhow::bail!(
                "Audio transmission rejected: no permission (204 No Content). \
                User may not have 'watch always' permission or no recent ring event."
            )
        } else {
            anyhow::bail!(
                "Audio transmission failed with status: {}. \
                Another client may already be transmitting.",
                status
            )
        }
    }

    /// Returns an RTSP URL for streaming live video from the DoorBird device.
    ///
    /// **API Endpoint:** RTSP streaming on port 8557 (RTSP-over-HTTP)
    ///
    /// **Required Permission:** Valid user with "watch always" permission or
    /// ring event in the past 5 minutes
    ///
    /// **Video Format:** H.264 encoded video at up to 12fps. Resolution depends
    /// on the quality parameter:
    /// - `VideoQuality::Default`: Device default resolution
    /// - `VideoQuality::P720`: 720p (supported by D10x/D21x and higher)
    /// - `VideoQuality::P1080`: 1080p (supported by D11x only)
    ///
    /// **Note:** The video connection can be interrupted at any time if the official
    /// DoorBird app requests the stream, as it has precedence over LAN API users.
    ///
    /// # Arguments
    ///
    /// * `quality` - Desired video quality/resolution
    ///
    /// # Returns
    ///
    /// RTSP URL string with embedded credentials in format:
    /// `rtsp://username:password@ip:8557/mpeg/[quality]/media.amp`
    ///
    /// # Example
    ///
    /// ```
    /// # use doorbird::{Client, VideoQuality};
    /// # let client = Client::new("http://192.168.1.100".into(), "user".into(), "pass".into());
    /// let rtsp_url = client.video_receive(VideoQuality::P1080);
    /// println!("RTSP URL: {}", rtsp_url);
    /// ```
    pub fn video_receive(&self, quality: VideoQuality) -> String {
        // Extract IP address from base_url (strip http:// or https://)
        let ip = self
            .base_url
            .trim_start_matches("http://")
            .trim_start_matches("https://");

        // Determine path based on quality
        let path = match quality {
            VideoQuality::Default => "mpeg/media.amp",
            VideoQuality::P720 => "mpeg/720p/media.amp",
            VideoQuality::P1080 => "mpeg/1080p/media.amp",
        };

        // Build RTSP URL with embedded credentials
        format!(
            "rtsp://{}:{}@{}:8557/{}",
            self.username, self.password, ip, path
        )
    }

    /// Opens a door/gate by triggering a relay on the DoorBird device.
    ///
    /// **API Endpoint:** `GET /bha-api/open-door.cgi`
    ///
    /// **Required Permission:** Valid user with "watch always" permission or
    /// ring event in the past 5 minutes
    ///
    /// Energizes the door opener/alarm output relay of the device. The API assumes
    /// that the user watches the live image in order to open the door or trigger relays.
    ///
    /// # Arguments
    ///
    /// * `relay` - Optional relay identifier (e.g., "1", "2", "gggaaa@1" for paired
    ///   DoorController). If `None`, defaults to physical relay 1.
    ///
    /// # Returns
    ///
    /// Returns `Ok(())` on success, or an error if the request fails.
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use doorbird::Client;
    /// # async fn example() -> anyhow::Result<()> {
    /// # let client = Client::new("http://192.168.1.100".into(), "user".into(), "pass".into());
    /// // Trigger default relay (relay 1)
    /// client.open_door(None).await?;
    ///
    /// // Trigger specific relay
    /// client.open_door(Some("2")).await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn open_door(&self, relay: Option<&str>) -> Result<()> {
        let mut url = format!("{}/bha-api/open-door.cgi", self.base_url);

        // Add relay parameter if specified
        if let Some(r) = relay {
            url.push_str(&format!("?r={}", r));
        }

        debug!("Opening door/gate via {}", url);

        let response = self
            .client
            .get(&url)
            .basic_auth(&self.username, Some(&self.password))
            .send()
            .await
            .context("Failed to send open door request")?;

        let status = response.status();
        if status.is_success() {
            info!("Door/gate opened successfully");
            Ok(())
        } else if status.as_u16() == 204 {
            anyhow::bail!(
                "Open door request rejected: no permission (204 No Content). \
                User may not have 'watch always' permission or no recent ring event."
            )
        } else {
            anyhow::bail!("Open door request failed with status: {}", status)
        }
    }

    /// Monitors for doorbell and motion sensor events from the DoorBird device.
    ///
    /// **API Endpoint:** `GET /bha-api/monitor.cgi?ring=doorbell,motionsensor`
    ///
    /// **Required Permission:** Valid user
    ///
    /// This method returns a continuous multipart stream that yields events as they occur
    /// on the DoorBird device. Events are sent when the doorbell button is pressed/released
    /// or when motion is detected/cleared.
    ///
    /// **Note:** The stream can be interrupted at any time. The caller is responsible for
    /// reconnecting if needed. Up to 8 concurrent monitor streams are allowed per device.
    ///
    /// # Returns
    ///
    /// A stream of `MonitorEvent` results. The stream will continue indefinitely until
    /// the connection is closed or an error occurs.
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use doorbird::{Client, MonitorEvent};
    /// # use futures_util::StreamExt;
    /// # async fn example() -> anyhow::Result<()> {
    /// # let client = Client::new("http://192.168.1.100".into(), "user".into(), "pass".into());
    /// let mut event_stream = client.monitor_events().await?;
    ///
    /// while let Some(event_result) = event_stream.next().await {
    ///     match event_result {
    ///         Ok(MonitorEvent::Doorbell) => {
    ///             println!("Doorbell pressed!");
    ///         }
    ///         Ok(MonitorEvent::MotionSensor { active }) => {
    ///             println!("Motion: {}", if active { "detected" } else { "cleared" });
    ///         }
    ///         Err(e) => {
    ///             eprintln!("Stream error: {}", e);
    ///             break;
    ///         }
    ///     }
    /// }
    /// # Ok(())
    /// # }
    /// ```
    pub async fn monitor_events(
        &self,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<MonitorEvent>> + Send>>> {
        let url = format!(
            "{}/bha-api/monitor.cgi?ring=doorbell,motionsensor",
            self.base_url
        );
        info!("Connecting to DoorBird event monitor at {}", url);

        let response = self
            .client
            .get(&url)
            .basic_auth(&self.username, Some(&self.password))
            .timeout(std::time::Duration::from_secs(3600)) // 1 hour timeout for streaming
            .send()
            .await
            .context("Failed to send monitor request")?;

        let status = response.status();
        if !status.is_success() {
            if status.as_u16() == 509 {
                anyhow::bail!(
                    "Monitor request failed: all monitor streams are busy (509). \
                    Maximum 8 concurrent streams allowed."
                );
            }
            anyhow::bail!("Monitor request failed with status: {}", status);
        }

        // Create a stream that parses the multipart response
        let byte_stream = response.bytes_stream();
        let event_stream = parse_monitor_stream(byte_stream);

        Ok(Box::pin(event_stream))
    }
}

/// Parses the multipart monitor stream into individual events.
///
/// The stream format is:
/// ```text
/// --ioboundary\r\n
/// Content-Type: text/plain\r\n
/// \r\n
/// doorbell:H\r\n
/// \r\n
/// --ioboundary\r\n
/// ...
/// ```
fn parse_monitor_stream(
    byte_stream: impl Stream<Item = std::result::Result<Bytes, reqwest::Error>> + Send + 'static,
) -> impl Stream<Item = Result<MonitorEvent>> + Send {
    // Pin the stream so we can poll it in the async closure
    let pinned_stream = Box::pin(byte_stream);

    // Use try_unfold to maintain state and yield events as they're parsed
    futures_util::stream::try_unfold(
        (pinned_stream, Vec::new()),
        |(mut stream, mut buffer)| async move {
            loop {
                // Try to extract an event from the current buffer
                if let Some(event) = extract_event_from_buffer(&mut buffer) {
                    return Ok(Some((event, (stream, buffer))));
                }

                // Need more data - fetch next chunk
                match stream.next().await {
                    Some(Ok(chunk)) => {
                        buffer.extend_from_slice(&chunk);
                        // Continue loop to try extracting again
                    }
                    Some(Err(e)) => {
                        return Err(anyhow::anyhow!("Stream error: {}", e));
                    }
                    None => {
                        // Stream ended
                        return Ok(None);
                    }
                }
            }
        },
    )
}

/// Extracts the next event from the buffer, removing consumed bytes.
///
/// Returns None if no complete event is available yet.
fn extract_event_from_buffer(buffer: &mut Vec<u8>) -> Option<MonitorEvent> {
    // Convert buffer to string for easier parsing
    let text = String::from_utf8_lossy(buffer);

    // Look for the event pattern: <type>:<state>
    // Events appear after the headers section (after \r\n\r\n)

    // Find pattern like "doorbell:H" or "motionsensor:L"
    if let Some(doorbell_pos) = text.find("doorbell:") {
        // Check if we have the complete event (should end with \r\n)
        if let Some(event_end) = text[doorbell_pos..].find("\r\n") {
            let event_line = &text[doorbell_pos..doorbell_pos + event_end];
            let state = event_line.chars().last()?;

            // Remove consumed bytes from buffer
            buffer.drain(0..doorbell_pos + event_end + 2);

            // Only emit event when doorbell is pressed (H), ignore released (L)
            if state == 'H' {
                return Some(MonitorEvent::Doorbell);
            }
            // For 'L' state, continue to check for more events
            return extract_event_from_buffer(buffer);
        }
    }

    if let Some(motion_pos) = text.find("motionsensor:") {
        // Check if we have the complete event (should end with \r\n)
        if let Some(event_end) = text[motion_pos..].find("\r\n") {
            let event_line = &text[motion_pos..motion_pos + event_end];
            let state = event_line.chars().last()?;

            // Remove consumed bytes from buffer
            buffer.drain(0..motion_pos + event_end + 2);

            return Some(MonitorEvent::MotionSensor {
                active: state == 'H',
            });
        }
    }

    // If buffer is getting too large without finding events, trim it
    if buffer.len() > 4096 {
        // Keep only the last 1KB in case we're in the middle of a boundary
        buffer.drain(0..buffer.len() - 1024);
    }

    None
}
