//! G.711 μ-law audio codec
//!
//! This module implements G.711 μ-law encoding and decoding for converting
//! between DoorBird's 8-bit compressed format and 16-bit PCM.

/// G.711 μ-law decoding lookup table
/// This table converts 8-bit μ-law values to 16-bit linear PCM
static ULAW_TO_LINEAR: [i16; 256] = [
    -32124, -31100, -30076, -29052, -28028, -27004, -25980, -24956, -23932, -22908, -21884, -20860,
    -19836, -18812, -17788, -16764, -15996, -15484, -14972, -14460, -13948, -13436, -12924, -12412,
    -11900, -11388, -10876, -10364, -9852, -9340, -8828, -8316, -7932, -7676, -7420, -7164, -6908,
    -6652, -6396, -6140, -5884, -5628, -5372, -5116, -4860, -4604, -4348, -4092, -3900, -3772,
    -3644, -3516, -3388, -3260, -3132, -3004, -2876, -2748, -2620, -2492, -2364, -2236, -2108,
    -1980, -1884, -1820, -1756, -1692, -1628, -1564, -1500, -1436, -1372, -1308, -1244, -1180,
    -1116, -1052, -988, -924, -876, -844, -812, -780, -748, -716, -684, -652, -620, -588, -556,
    -524, -492, -460, -428, -396, -372, -356, -340, -324, -308, -292, -276, -260, -244, -228, -212,
    -196, -180, -164, -148, -132, -120, -112, -104, -96, -88, -80, -72, -64, -56, -48, -40, -32,
    -24, -16, -8, 0, 32124, 31100, 30076, 29052, 28028, 27004, 25980, 24956, 23932, 22908, 21884,
    20860, 19836, 18812, 17788, 16764, 15996, 15484, 14972, 14460, 13948, 13436, 12924, 12412,
    11900, 11388, 10876, 10364, 9852, 9340, 8828, 8316, 7932, 7676, 7420, 7164, 6908, 6652, 6396,
    6140, 5884, 5628, 5372, 5116, 4860, 4604, 4348, 4092, 3900, 3772, 3644, 3516, 3388, 3260, 3132,
    3004, 2876, 2748, 2620, 2492, 2364, 2236, 2108, 1980, 1884, 1820, 1756, 1692, 1628, 1564, 1500,
    1436, 1372, 1308, 1244, 1180, 1116, 1052, 988, 924, 876, 844, 812, 780, 748, 716, 684, 652,
    620, 588, 556, 524, 492, 460, 428, 396, 372, 356, 340, 324, 308, 292, 276, 260, 244, 228, 212,
    196, 180, 164, 148, 132, 120, 112, 104, 96, 88, 80, 72, 64, 56, 48, 40, 32, 24, 16, 8, 0,
];

/// Decodes a single G.711 μ-law sample to linear PCM
///
/// # Arguments
/// * `ulaw` - 8-bit μ-law encoded sample
///
/// # Returns
/// 16-bit linear PCM sample
#[inline]
pub fn decode_ulaw(ulaw: u8) -> i16 {
    ULAW_TO_LINEAR[ulaw as usize]
}

/// Decodes a buffer of G.711 μ-law samples to linear PCM
///
/// # Arguments
/// * `input` - Slice of 8-bit μ-law encoded samples
///
/// # Returns
/// Vector of 16-bit linear PCM samples
#[allow(dead_code)]
pub fn decode_ulaw_buffer(input: &[u8]) -> Vec<i16> {
    input.iter().map(|&byte| decode_ulaw(byte)).collect()
}

/// Encodes a single linear PCM sample to G.711 μ-law
///
/// # Arguments
/// * `pcm` - 16-bit linear PCM sample
///
/// # Returns
/// 8-bit μ-law encoded sample
#[inline]
pub fn encode_ulaw(pcm: i16) -> u8 {
    const BIAS: i32 = 0x84;
    const CLIP: i32 = 32635;

    // Get the sign and absolute value
    let sign: u8 = if pcm < 0 { 0x80 } else { 0x00 };
    let mut sample = if pcm < 0 {
        (-pcm as i32).min(CLIP)
    } else {
        (pcm as i32).min(CLIP)
    };

    // Add bias
    sample += BIAS;

    // Find the exponent (segment) - count leading zeros to find position
    let mut exponent = 7;
    for i in 0..8 {
        if sample <= (0xFF << i) {
            exponent = i;
            break;
        }
    }

    // Extract mantissa (top 4 bits of the segment)
    let mantissa = ((sample >> (exponent + 3)) & 0x0F) as u8;

    // Combine sign, exponent, and mantissa, then invert (complement)
    let encoded = sign | ((exponent as u8) << 4) | mantissa;
    !encoded
}

/// Encodes a buffer of linear PCM samples to G.711 μ-law
///
/// # Arguments
/// * `input` - Slice of 16-bit linear PCM samples
///
/// # Returns
/// Vector of 8-bit μ-law encoded samples
pub fn encode_ulaw_buffer(input: &[i16]) -> Vec<u8> {
    input.iter().map(|&sample| encode_ulaw(sample)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_decode_silence() {
        // μ-law silence value is 0xFF
        let silence = decode_ulaw(0xFF);
        assert_eq!(silence, 0);
    }

    #[test]
    fn test_decode_positive_max() {
        let max = decode_ulaw(0x80);
        assert_eq!(max, 32124);
    }

    #[test]
    fn test_decode_negative_max() {
        let min = decode_ulaw(0x00);
        assert_eq!(min, -32124);
    }

    #[test]
    fn test_decode_buffer() {
        let input = vec![0xFF, 0xFF, 0xFF, 0xFF];
        let output = decode_ulaw_buffer(&input);
        assert_eq!(output, vec![0, 0, 0, 0]);
    }

    #[test]
    fn test_encode_silence() {
        // Silence (0) should encode to 0xFF
        let encoded = encode_ulaw(0);
        assert_eq!(encoded, 0xFF);
    }

    #[test]
    fn test_encode_positive() {
        // Test some positive values
        let encoded = encode_ulaw(1000);
        // Decode it back to verify it's close
        let decoded = decode_ulaw(encoded);
        assert!((decoded - 1000).abs() < 200); // Allow some quantization error
    }

    #[test]
    fn test_encode_negative() {
        // Test some negative values
        let encoded = encode_ulaw(-1000);
        let decoded = decode_ulaw(encoded);
        assert!((decoded + 1000).abs() < 200);
    }

    #[test]
    fn test_encode_decode_roundtrip() {
        // Test that encoding and decoding are consistent
        let test_values = vec![0, 100, -100, 1000, -1000, 5000, -5000, 10000, -10000];
        for val in test_values {
            let encoded = encode_ulaw(val);
            let decoded = decode_ulaw(encoded);
            // μ-law is lossy, so allow some error proportional to magnitude
            let max_error = val.abs() / 10 + 100;
            assert!(
                (decoded - val).abs() < max_error,
                "Roundtrip failed for {}: got {}, error {}",
                val,
                decoded,
                (decoded - val).abs()
            );
        }
    }

    #[test]
    fn test_encode_buffer() {
        let input = vec![0, 0, 0, 0];
        let output = encode_ulaw_buffer(&input);
        assert_eq!(output, vec![0xFF, 0xFF, 0xFF, 0xFF]);
    }
}
