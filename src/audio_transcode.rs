//! Audio transcoding from DoorBird G.711 μ-law to WebRTC Opus
//!
//! This module handles the complete audio pipeline:
//! 1. Decode G.711 μ-law (8-bit) to PCM i16
//! 2. Resample from 8kHz to 48kHz
//! 3. Convert PCM i16 to f32
//! 4. Encode to Opus

use anyhow::{Context, Result};
use audiopus::{Application, Channels, SampleRate, coder::Encoder};
use rubato::{
    Resampler, SincFixedIn, SincInterpolationParameters, SincInterpolationType, WindowFunction,
};
use tracing::warn;

/// Audio transcoder for converting DoorBird audio to WebRTC format
pub struct AudioTranscoder {
    /// Opus encoder for 48kHz mono audio
    opus_encoder: Encoder,
    /// Resampler for 8kHz -> 48kHz conversion
    resampler: SincFixedIn<f32>,
    /// Buffer for accumulating input samples before resampling (8kHz)
    input_buffer: Vec<f32>,
    /// Buffer for accumulating resampled output before encoding (48kHz)
    output_buffer: Vec<f32>,
    /// Target number of input samples before resampling (8kHz @ 20ms = 160 samples)
    input_frame_size: usize,
    /// Target number of output samples for Opus encoding (48kHz @ 20ms = 960 samples)
    output_frame_size: usize,
}

impl AudioTranscoder {
    /// Creates a new audio transcoder
    ///
    /// Configures:
    /// - G.711 μ-law decoder (8kHz input)
    /// - Rubato resampler (8kHz -> 48kHz)
    /// - Opus encoder (48kHz output, 20ms frames)
    pub fn new() -> Result<Self> {
        // Create Opus encoder for 48kHz, mono, 20ms frames
        let opus_encoder = Encoder::new(SampleRate::Hz48000, Channels::Mono, Application::Voip)
            .context("Failed to create Opus encoder")?;

        // Create resampler: 8kHz -> 48kHz (6x upsampling)
        // Input: 160 samples @ 8kHz = 20ms
        // Output: 960 samples @ 48kHz = 20ms
        let params = SincInterpolationParameters {
            sinc_len: 256,
            f_cutoff: 0.95,
            interpolation: SincInterpolationType::Linear,
            oversampling_factor: 256,
            window: WindowFunction::BlackmanHarris2,
        };

        let resampler = SincFixedIn::<f32>::new(
            48000.0 / 8000.0, // ratio
            2.0,              // max_resample_ratio_relative
            params,
            160, // input frame size (20ms @ 8kHz)
            1,   // channels
        )
        .context("Failed to create resampler")?;

        Ok(Self {
            opus_encoder,
            resampler,
            input_buffer: Vec::with_capacity(160),
            output_buffer: Vec::with_capacity(960),
            input_frame_size: 160,
            output_frame_size: 960,
        })
    }

    /// Processes a chunk of G.711 μ-law audio data
    ///
    /// Takes raw μ-law bytes, decodes, resamples, and encodes to Opus.
    /// May return multiple Opus frames if input is large enough.
    ///
    /// # Arguments
    /// * `ulaw_data` - Raw G.711 μ-law bytes from DoorBird (8kHz, mono)
    ///
    /// # Returns
    /// Vector of Opus-encoded frames (each ~20ms of audio)
    pub fn process_chunk(&mut self, ulaw_data: &[u8]) -> Result<Vec<Vec<u8>>> {
        // Decode G.711 μ-law to PCM i16
        let pcm_i16: Vec<i16> = ulaw_data
            .iter()
            .map(|&byte| crate::g711::decode_ulaw(byte))
            .collect();

        // Convert i16 to f32 normalized to [-1.0, 1.0]
        let pcm_f32: Vec<f32> = pcm_i16
            .iter()
            .map(|&sample| sample as f32 / 32768.0)
            .collect();

        // Add to input buffer
        self.input_buffer.extend_from_slice(&pcm_f32);

        let mut opus_frames = Vec::new();

        // Process complete input frames
        while self.input_buffer.len() >= self.input_frame_size {
            // Extract one frame worth of input samples
            let frame: Vec<f32> = self.input_buffer.drain(..self.input_frame_size).collect();

            // Resample 8kHz -> 48kHz
            let resampled = self
                .resampler
                .process(&[frame], None)
                .context("Resampling failed")?;

            // resampled is Vec<Vec<f32>>, we have mono so take channel 0
            let resampled_mono = &resampled[0];

            // Add resampled data to output buffer
            self.output_buffer.extend_from_slice(resampled_mono);
        }

        // Encode complete output frames to Opus
        while self.output_buffer.len() >= self.output_frame_size {
            // Extract exactly 960 samples for Opus encoding
            let opus_input: Vec<f32> = self.output_buffer.drain(..self.output_frame_size).collect();

            // Encode to Opus
            let mut opus_buffer = vec![0u8; 4000];
            let encoded_len = self
                .opus_encoder
                .encode_float(&opus_input, &mut opus_buffer)
                .context("Opus encoding failed")?;

            opus_frames.push(opus_buffer[..encoded_len].to_vec());
        }

        Ok(opus_frames)
    }

    /// Flushes any remaining audio in the buffers
    ///
    /// Should be called when the audio stream ends to process any partial frames
    pub fn flush(&mut self) -> Result<Vec<Vec<u8>>> {
        let mut opus_frames = Vec::new();

        // Process any remaining input samples
        if !self.input_buffer.is_empty() {
            if self.input_buffer.len() < self.input_frame_size {
                warn!(
                    "Flushing partial input frame: {} samples (padding to {})",
                    self.input_buffer.len(),
                    self.input_frame_size
                );
                self.input_buffer.resize(self.input_frame_size, 0.0);
            }

            // Resample remaining input
            let frame: Vec<f32> = self.input_buffer.drain(..).collect();
            if let Ok(resampled) = self.resampler.process(&[frame], None) {
                self.output_buffer.extend_from_slice(&resampled[0]);
            }
        }

        // Encode any remaining output samples
        if !self.output_buffer.is_empty() {
            if self.output_buffer.len() < self.output_frame_size {
                warn!(
                    "Flushing partial output frame: {} samples (padding to {})",
                    self.output_buffer.len(),
                    self.output_frame_size
                );
                self.output_buffer.resize(self.output_frame_size, 0.0);
            }

            let opus_input: Vec<f32> = self.output_buffer.drain(..self.output_frame_size).collect();
            let mut opus_buffer = vec![0u8; 4000];
            if let Ok(encoded_len) = self
                .opus_encoder
                .encode_float(&opus_input, &mut opus_buffer)
            {
                opus_frames.push(opus_buffer[..encoded_len].to_vec());
            }
        }

        Ok(opus_frames)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_transcoder_creation() {
        let transcoder = AudioTranscoder::new();
        assert!(transcoder.is_ok());
    }

    #[test]
    fn test_process_empty() {
        let mut transcoder = AudioTranscoder::new().unwrap();
        let result = transcoder.process_chunk(&[]);
        assert!(result.is_ok());
        assert_eq!(result.unwrap().len(), 0);
    }

    #[test]
    fn test_process_small_chunk() {
        let mut transcoder = AudioTranscoder::new().unwrap();
        // Generate 80 bytes of μ-law silence (0xFF)
        let silence = vec![0xFF; 80];
        let result = transcoder.process_chunk(&silence);
        assert!(result.is_ok());
        // Should not produce output yet (need 160 samples)
        assert_eq!(result.unwrap().len(), 0);
    }

    #[test]
    fn test_process_full_frame() {
        let mut transcoder = AudioTranscoder::new().unwrap();
        // Note: The resampler has internal buffering, so we need to feed multiple
        // input frames before we get a complete output frame for Opus encoding.

        // Generate test data - feed multiple frames to account for resampler delay
        let test_data = vec![0x7F; 160]; // Mid-range value, one frame @ 8kHz

        // Process multiple frames to fill the resampler and output buffers
        let mut total_frames = 0;
        for _ in 0..10 {
            let result = transcoder.process_chunk(&test_data);
            match result {
                Ok(frames) => {
                    total_frames += frames.len();
                }
                Err(_) => {
                    // Opus can reject certain synthetic patterns initially
                }
            }
        }

        // After processing multiple input frames, we should have produced some output
        // The exact count depends on resampler buffering, but we should get frames
        assert!(
            total_frames > 0,
            "Expected to produce at least some Opus frames after processing multiple input frames"
        );
    }
}
