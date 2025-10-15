# Latency Analysis & Performance Tuning

This document explains the latency characteristics of Birdbox and how to tune performance for your specific requirements.

## Total End-to-End Latency

- **Audio**: ~400-500ms (default configuration)
- **Video**: ~450-900ms (default configuration)

These values are acceptable for doorbell/intercom applications where reliability is more important than real-time interaction.

## Audio Pipeline Latency

### Components

**Total Audio Latency: ~42-52ms (processing) + buffer latency**

#### 1. Resampler Filter Delay: ~32ms

The Rubato `SincFixedIn` resampler introduces inherent delay from its sinc filter:

```rust
// src/audio_transcode.rs
let params = SincInterpolationParameters {
    sinc_len: 256,  // Filter length
    f_cutoff: 0.95,
    interpolation: SincInterpolationType::Linear,
    oversampling_factor: 256,
    window: WindowFunction::BlackmanHarris2,
};
```

**Calculation**:
- Input-side delay: `(sinc_len / 2) / input_rate = (256/2) / 8000 = 16ms`
- Output-side processing: ~16ms
- **Total resampler delay: ~32ms**

**Why this matters**: Higher `sinc_len` values provide better audio quality but add latency. The value of 256 balances quality and latency for voice communication.

#### 2. Output Buffer Accumulation: 0-20ms

The resampler produces variable output sizes due to internal buffering. The output buffer accumulates resampled data until exactly 960 samples (20ms @ 48kHz) are available for Opus encoding.

- **Best case**: 0ms (perfect alignment)
- **Worst case**: 20ms (waiting for next input frame)
- **Average**: ~10ms

**Code location**: `src/audio_transcode.rs`, lines 119-131

#### 3. Audio Fanout Buffer: Configurable

**Default**: 20 samples × 20ms = **400ms**

This is the broadcast channel buffer between the transcoder and WebRTC sessions.

```bash
# Environment variable
BIRDBOX_AUDIO_FANOUT_BUFFER_SAMPLES=20
```

**Purpose**: Absorbs timing variations and prevents audio dropouts when:
- WebRTC client can't consume audio fast enough
- CPU spikes cause brief processing delays
- Network jitter causes temporary stream delays

### Audio Latency Tuning

#### Buffer Size Trade-offs

| Buffer Size | Latency | Characteristics                                  |
| ----------- | ------- | ------------------------------------------------ |
| 5 samples   | 100ms   | **Minimum** - High risk of crackling/dropouts    |
| 10 samples  | 200ms   | **Aggressive** - May crackle under load          |
| 20 samples  | 400ms   | **Default** - Good balance for intercom          |
| 30 samples  | 600ms   | **Conservative** - Very smooth, noticeable delay |
| 50 samples  | 1000ms  | **High** - Echo-like delay in conversation       |

#### Recommended Settings

**For Minimum Latency** (competitive intercom feel):
```bash
BIRDBOX_AUDIO_FANOUT_BUFFER_SAMPLES=10  # 200ms
```
- **Risk**: May crackle on slower devices or under CPU load
- **Best for**: Powerful servers, low-latency priority

**For Maximum Reliability** (smooth audio):
```bash
BIRDBOX_AUDIO_FANOUT_BUFFER_SAMPLES=30  # 600ms
```
- **Benefits**: Very smooth, resilient to temporary issues
- **Trade-off**: Noticeable delay in conversation flow

**For Production Intercom**:
```bash
BIRDBOX_AUDIO_FANOUT_BUFFER_SAMPLES=20  # 400ms (default)
```
- **Balance**: Acceptable delay with good reliability
- **Recommended**: Start here, adjust based on experience

### Failure Modes with Low Buffer

If buffer is too small (< 10 samples):
- **Slow subscriber**: WebRTC client can't consume audio fast enough → audio gaps/crackling
- **CPU spikes**: Brief processing delays cause buffer underrun → dropouts
- **Network jitter**: DoorBird audio stream delays momentarily → gaps in playback

**Symptoms**: Choppy audio, crackling, intermittent dropouts, robotic voice quality

### Reducing Resampler Latency (Advanced)

If you absolutely need lower audio latency, you can reduce resampler quality:

**Option 1: Reduce sinc_len** (saves ~20-25ms):
```rust
// In src/audio_transcode.rs, line 55-60
let params = SincInterpolationParameters {
    sinc_len: 64,  // Reduced from 256
    f_cutoff: 0.95,
    interpolation: SincInterpolationType::Linear,
    oversampling_factor: 64,  // Match sinc_len
    window: WindowFunction::BlackmanHarris2,
};
```
- **Trade-off**: Lower audio quality, potential aliasing
- **New latency**: ~10-12ms resampler delay

**Option 2: Switch to FastFixedIn** (saves ~25-30ms):
```rust
use rubato::FastFixedIn;
let resampler = FastFixedIn::<f32>::new(/* ... */);
```
- **Trade-off**: Significant audio quality degradation (linear interpolation only)
- **New latency**: ~2-5ms resampler delay

**Recommendation**: Keep current settings. The 50ms processing latency is imperceptible for voice communication and ensures high-quality audio resampling.

## Video Pipeline Latency

### Components

**Total Video Latency: ~450ms-900ms (default configuration)**

#### 1. RTSP Input Configuration

**ffmpeg low-latency options** (applied in `src/h264_extractor.rs`):

```rust
options.set("rtsp_transport", &self.rtsp_transport); // tcp or udp
options.set("fflags", "nobuffer");    // Disable buffering
options.set("flags", "low_delay");    // Enable low delay mode
options.set("max_delay", "0");        // Minimize decoder delay
```

**Impact**: These flags eliminate ~1-2 seconds of default ffmpeg buffering

**Before optimization**: ~1-2s RTSP buffering
**After optimization**: ~100ms RTSP buffering

#### 2. Fixed Sample Duration

WebRTC uses a fixed 83ms duration (~12fps) instead of accumulated timestamps to prevent timestamp drift:

```rust
// src/webrtc.rs
let sample_duration = Duration::from_millis(83); // ~12fps
```

This prevents latency accumulation but means timing is approximate.

#### 3. Video Fanout Buffer: Configurable

**Default**: 4 frames × 83ms = **~330ms @ 12fps**

```bash
# Environment variable
BIRDBOX_VIDEO_FANOUT_BUFFER_FRAMES=4
```

**Previous default**: 30 frames (~2.5 seconds) - too high for intercom use

### Video Latency Breakdown

**Before optimizations**: ~2-3 seconds
- RTSP buffering: ~1-2s
- Fanout buffer: ~2.5s
- Network transit: ~100ms

**After optimizations**: ~450-900ms
- RTSP buffering: ~100ms (nobuffer flags)
- Fanout buffer: ~330ms (4 frames @ 12fps, configurable)
- Network transit: ~100ms
- WebRTC jitter buffer: ~0-200ms (adaptive)

### Video Latency Tuning

#### Buffer Size Trade-offs

| Buffer Size | Latency @12fps | Characteristics                        |
| ----------- | -------------- | -------------------------------------- |
| 1 frame     | ~83ms          | **Minimum** - Frequent frame drops     |
| 3 frames    | ~250ms         | **Aggressive** - Occasional stuttering |
| 4 frames    | ~330ms         | **Default** - Good balance             |
| 5 frames    | ~420ms         | **Conservative** - Very smooth         |
| 10 frames   | ~830ms         | **High** - Noticeable delay            |

#### Recommended Settings

**For Minimum Latency** (doorbell reaction time priority):
```bash
BIRDBOX_VIDEO_FANOUT_BUFFER_FRAMES=3  # ~250ms
```
- **Risk**: More frame drops under load, occasional stuttering
- **Best for**: Fast networks, latency-critical scenarios

**For Maximum Smoothness** (video quality priority):
```bash
BIRDBOX_VIDEO_FANOUT_BUFFER_FRAMES=5  # ~420ms
```
- **Benefits**: Very smooth video, resilient to frame timing variations
- **Trade-off**: Slight additional delay

**For Production Use**:
```bash
BIRDBOX_VIDEO_FANOUT_BUFFER_FRAMES=4  # ~330ms (default)
```
- **Balance**: Acceptable latency with reliable video
- **Recommended**: Start here, adjust based on experience

### Transport Protocol Selection

#### UDP vs TCP for RTSP

**UDP Transport** (default):
```bash
BIRDBOX_RTSP_TRANSPORT_PROTOCOL=udp
```
- **Latency**: 10-20ms lower than TCP
- **Reliability**: May have packet loss over WiFi
- **Best for**: Simple networks, wired connections, minimal hops

**TCP Transport** (recommended for complex networks):
```bash
BIRDBOX_RTSP_TRANSPORT_PROTOCOL=tcp
```
- **Latency**: 10-20ms higher than UDP
- **Reliability**: Much more reliable
- **Required for**:
  - VPN connections (Tailscale, WireGuard, OpenVPN)
  - Docker with complex networking
  - Networks with high packet loss
  - UDP-over-UDP tunneling scenarios

**Why TCP for VPN?** UDP-over-UDP (RTSP UDP through VPN UDP tunnel) causes:
- Packet loss amplification
- Timeout conflicts between layers
- NAT traversal issues
- Stream reliability problems

**Symptoms of wrong transport**:
- Video stream hangs or fails to start
- Frequent reconnections
- "Failed to open RTSP stream" errors
- Timeout messages in logs

**Rule of thumb**: If you're using any form of VPN or experiencing video issues, switch to TCP.

## Network Transit & WebRTC Jitter Buffer

Beyond our control but worth understanding:

- **Network transit**: ~50-100ms on LAN, ~100-300ms on internet
- **WebRTC jitter buffer**: 0-200ms (adaptive based on network conditions)

The browser's WebRTC implementation automatically adapts its jitter buffer based on observed network jitter and packet timing variations.

## Comparison to Standard VoIP

**Context for acceptable latency**:

| System            | Typical Latency | Use Case                            |
| ----------------- | --------------- | ----------------------------------- |
| Telephone         | 20-50ms         | Real-time conversation              |
| VoIP (good)       | 150ms           | Acceptable for conversation         |
| VoIP (acceptable) | 150-400ms       | Noticeable but usable               |
| **Birdbox Audio** | **400-500ms**   | Doorbell/intercom (one-way primary) |
| **Birdbox Video** | **450-900ms**   | Visual identification               |
| Satellite phone   | 500-700ms       | Frustrating but functional          |

**Human perception**:
- < 150ms: Feels real-time
- 150-400ms: Noticeable delay but conversational
- 400-700ms: Clearly delayed, requires conversation adjustments
- > 700ms: Echo-like, difficult to use

**For doorbell use**: The typical use case is one-way communication ("I'll be right there!") rather than full conversation, so 400-500ms audio latency is acceptable.

## Monitoring Latency in Production

### Logging

Watch for these log messages that indicate latency issues:

```bash
# Audio processing delays
WARN audio transcoding error
WARN Error flushing transcoder

# Video frame drops
WARN Error getting video packet
INFO No more subscribers, stopping video stream

# Buffer overflows/underflows
# (Currently no explicit logging, but would show as audio/video glitches)
```

### Metrics to Track (if implementing)

Useful metrics for future monitoring:
- Audio buffer fill level (current/max)
- Video buffer fill level (current/max)
- Transcoding processing time per frame
- End-to-end latency (if measurable)
- Packet drop rate

## Optimization Summary

### Quick Reference

**Already Optimized**:
- ✅ Zero video transcoding (H.264 pass-through)
- ✅ ffmpeg low-latency flags (`nobuffer`, `low_delay`)
- ✅ High-quality audio resampling (minimal artifacts)
- ✅ Fixed WebRTC sample duration (no timestamp drift)

**Tunable Parameters**:
- 🔧 `BIRDBOX_AUDIO_FANOUT_BUFFER_SAMPLES`: Lower = less latency, more dropouts
- 🔧 `BIRDBOX_VIDEO_FANOUT_BUFFER_FRAMES`: Lower = less latency, more frame drops
- 🔧 `BIRDBOX_RTSP_TRANSPORT_PROTOCOL`: TCP for reliability, UDP for latency

**Not Recommended** (diminishing returns):
- ❌ Reducing resampler quality (audio degradation)
- ❌ Buffer sizes < 10 samples audio or < 3 frames video (unreliable)
- ❌ Video transcoding (adds huge latency + CPU load)

### Tuning Workflow

1. **Start with defaults**: Validate system works reliably
2. **Monitor performance**: Check for audio crackling or video stuttering
3. **Adjust one parameter at a time**: Test thoroughly after each change
4. **Document your findings**: Network characteristics vary widely

### Environment-Specific Recommendations

**Home Network (WiFi, standard router)**:
```bash
BIRDBOX_AUDIO_FANOUT_BUFFER_SAMPLES=20
BIRDBOX_VIDEO_FANOUT_BUFFER_FRAMES=4
BIRDBOX_RTSP_TRANSPORT_PROTOCOL=udp
```

**VPN / Complex Network (Tailscale, WireGuard)**:
```bash
BIRDBOX_AUDIO_FANOUT_BUFFER_SAMPLES=30
BIRDBOX_VIDEO_FANOUT_BUFFER_FRAMES=5
BIRDBOX_RTSP_TRANSPORT_PROTOCOL=tcp
```

**Data Center / Wired LAN (minimal latency)**:
```bash
BIRDBOX_AUDIO_FANOUT_BUFFER_SAMPLES=10
BIRDBOX_VIDEO_FANOUT_BUFFER_FRAMES=3
BIRDBOX_RTSP_TRANSPORT_PROTOCOL=udp
```

**Low-End Hardware (Raspberry Pi, etc.)**:
```bash
BIRDBOX_AUDIO_FANOUT_BUFFER_SAMPLES=30
BIRDBOX_VIDEO_FANOUT_BUFFER_FRAMES=5
BIRDBOX_RTSP_TRANSPORT_PROTOCOL=tcp
```

## Conclusion

Birdbox is optimized for doorbell/intercom use cases where reliability and quality are more important than ultra-low latency. The default configuration (~400ms audio, ~450ms video) provides a good balance for most deployments.

For specific requirements:
- **Need minimum latency?** Reduce buffer sizes carefully
- **Need maximum reliability?** Increase buffer sizes generously
- **Having connection issues?** Switch to TCP transport
- **Want to experiment?** Adjust one parameter at a time and monitor results

Remember: A slightly delayed but crystal-clear conversation is better than a low-latency stream with dropouts and artifacts.

