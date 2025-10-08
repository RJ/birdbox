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
use futures_util::Stream;
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
}
