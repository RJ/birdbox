# Architecture

This document describes the system architecture, components, and design decisions behind Birdbox.

## System Overview

Birdbox is a WebRTC gateway that bridges DoorBird smart doorbells to web browsers, enabling real-time audio/video streaming and two-way communication.

## Component Architecture

```
┌──────────────────────────────────────────────────────────────┐
│                     DoorBird Device                          │
│  - G.711 μ-law audio @ 8kHz (HTTP stream)                   │
│  - H.264 video @ ~12fps (RTSP stream)                        │
│  - Door control API                                          │
│  - Event monitoring (doorbell, motion)                       │
└────────────────┬─────────────────────────────────────────────┘
                 │
                 │ HTTP/RTSP
                 │
┌────────────────▼─────────────────────────────────────────────┐
│                    Birdbox Server (Rust)                     │
│                                                              │
│  ┌──────────────────────────────────────────────────────┐  │
│  │  doorbird::Client                                    │  │
│  │  - HTTP API wrapper                                  │  │
│  │  - Audio streaming                                   │  │
│  │  - Video URL generation                              │  │
│  │  - Device control                                    │  │
│  │  - Event monitoring                                  │  │
│  └─────────────────┬────────────────────────────────────┘  │
│                    │                                         │
│  ┌─────────────────▼────────────────┐  ┌──────────────────┐ │
│  │  Audio Pipeline                  │  │  Video Pipeline  │ │
│  │                                   │  │                  │ │
│  │  1. G.711 μ-law decode           │  │  1. RTSP stream  │ │
│  │  2. 8kHz → 48kHz resample         │  │  2. H.264 extract│ │
│  │  3. Opus encode                   │  │  3. Pass-through │ │
│  └─────────────────┬────────────────┘  └────────┬─────────┘ │
│                    │                             │           │
│  ┌─────────────────▼────────────────────────────▼─────────┐ │
│  │              Fanout System                             │ │
│  │  - Single DoorBird connection                          │ │
│  │  - Broadcast to N WebRTC clients                       │ │
│  │  - Smart connect/disconnect                            │ │
│  │  - Grace period handling                               │ │
│  └─────────────────┬──────────────────────────────────────┘ │
│                    │                                         │
│  ┌─────────────────▼──────────────────────────────────────┐ │
│  │           WebRTC Infrastructure                        │ │
│  │  - UDP mux (single port for all sessions)             │ │
│  │  - ICE candidate management                            │ │
│  │  - Per-session peer connections                        │ │
│  │  - Push-to-talk coordination                           │ │
│  └─────────────────┬──────────────────────────────────────┘ │
│                    │                                         │
│  ┌─────────────────▼──────────────────────────────────────┐ │
│  │          Axum Web Server                               │ │
│  │  - HTTP server (port 3000)                             │ │
│  │  - WebSocket signaling                                 │ │
│  │  - Static file serving                                 │ │
│  │  - Door control API                                    │ │
│  └────────────────────────────────────────────────────────┘ │
└──────────────────────┬───────────────────────────────────────┘
                       │
                       │ WebSocket (signaling)
                       │ UDP (media - port 50000)
                       │
┌──────────────────────▼───────────────────────────────────────┐
│                   Web Browsers                               │
│  - JavaScript WebRTC client                                  │
│  - Opus audio @ 48kHz                                        │
│  - H.264 video                                               │
│  - Push-to-talk UI                                           │
└──────────────────────────────────────────────────────────────┘
```

## Core Components

### 1. DoorBird Client Library (`doorbird/`)

**Purpose**: Rust library for DoorBird LAN API interaction

**Key Features**:
- HTTP Basic Authentication (RFC 2617)
- Device information retrieval (`/bha-api/info.cgi`)
- Audio streaming (`/bha-api/audio-receive.cgi`)
- Audio transmission (`/bha-api/audio-transmit.cgi`)
- RTSP URL generation for video
- Event monitoring (`/bha-api/monitor.cgi`)
- Door control (`/bha-api/open-door.cgi`)

**Implementation**: `doorbird/src/lib.rs`
- Fully documented with rustdoc
- Returns raw audio bytes (no transcoding in library)
- Async/await with `reqwest` HTTP client

### 2. Audio Pipeline

#### Audio Transcoder (`src/audio_transcode.rs`)

**Forward Path (DoorBird → WebRTC)**:
1. **G.711 μ-law Decode**: 8-bit compressed → 16-bit PCM
2. **Resample**: 8kHz → 48kHz (using Rubato SincFixedIn)
3. **Float Conversion**: i16 → f32 normalized [-1.0, 1.0]
4. **Opus Encode**: PCM → Opus @ 48kHz, 20ms frames

**Reverse Path (WebRTC → DoorBird)**:
1. **Opus Decode**: Opus → PCM f32 @ 48kHz
2. **Resample**: 48kHz → 8kHz
3. **Int Conversion**: f32 → i16
4. **G.711 μ-law Encode**: PCM → 8-bit compressed

**Key Parameters**:
- Input: 160 samples @ 8kHz (20ms frames)
- Output: 960 samples @ 48kHz (20ms frames)
- Resampler: `sinc_len: 256` for quality (introduces ~32ms latency)

#### Audio Fanout (`src/audio_fanout.rs`)

**Purpose**: Manage single DoorBird audio connection for multiple viewers

**Lifecycle**:
1. **Idle**: No DoorBird connection, waiting for subscribers
2. **Connecting**: First subscriber joined, establishing connection
3. **Connected**: Streaming audio to all subscribers
4. **Disconnecting**: Last subscriber left, grace period active (3s)
5. **Reconnect or Idle**: Based on subscriber presence after grace period

**Features**:
- Broadcast channel for distributing Opus samples
- Configurable buffer size (default: 20 samples ~400ms)
- Automatic transcoding in background task
- Subscriber count tracking

### 3. Video Pipeline

#### H.264 Extractor (`src/h264_extractor.rs`)

**Purpose**: Extract raw H.264 packets from RTSP stream

**Process**:
1. Connect to DoorBird RTSP stream with ffmpeg
2. Extract H.264 packets without decoding
3. Forward packets to WebRTC (zero transcoding)
4. Auto-reconnect on stream interruption

**Optimizations**:
- `rtsp_transport`: TCP (reliable) or UDP (low latency)
- `fflags=nobuffer`: Disable buffering
- `flags=low_delay`: Enable low-delay mode
- `max_delay=0`: Minimize decoder delay

**These flags eliminate ~1-2 seconds of buffering**

#### Video Fanout (`src/video_fanout.rs`)

**Purpose**: Manage single RTSP connection for multiple viewers

**Implementation**:
- Similar lifecycle to audio fanout
- Longer grace period (5s) due to video stream reconnect overhead
- Runs in `spawn_blocking` due to ffmpeg non-Send types
- Broadcasts H.264 packets directly (no processing)

**Buffer Configuration**:
- Default: 4 frames (~330ms @ 12fps)
- Configurable via `BIRDBOX_VIDEO_FANOUT_BUFFER_FRAMES`

### 4. WebRTC Infrastructure (`src/webrtc.rs`)

#### WebRTC Infrastructure (`WebRtcInfra`)

**Purpose**: Shared WebRTC components for all sessions

**Features**:
- **UDP Multiplexing**: Single UDP port (50000) for all connections
- **ICE Configuration**: Host candidate management
- **NAT 1:1 Mapping**: Advertise specific IP(s) for Docker deployments
- **mDNS Control**: Disabled when specific IPs configured
- **Split-Brain DNS**: Optional dual-IP advertising (LAN + public)

**Socket Binding**:
- Defaults to `0.0.0.0` (all interfaces) for maximum compatibility
- Can be overridden with `BIRDBOX_BIND_IP` for specific interface binding
- Separate from IP advertising (`BIRDBOX_HOST_IP`/`BIRDBOX_HOST_IP_LAN`)
- Uses SO_REUSEADDR/SO_REUSEPORT for quick rebinding

#### WebRTC Session (`WebRtcSession`)

**Purpose**: Per-client WebRTC peer connection

**Responsibilities**:
- SDP offer/answer negotiation
- ICE candidate exchange
- Audio track management (receive from fanout)
- Video track management (receive from fanout)
- Push-to-talk audio transmission
- WebSocket communication

**Audio Tracks**:
- Outbound: Opus @ 48kHz from audio fanout
- Inbound: Opus @ 48kHz for push-to-talk

**Video Tracks**:
- Outbound: H.264 @ 12fps from video fanout
- Fixed 83ms sample duration (~12fps)

### 5. Push-to-Talk System (`src/main.rs`)

**Purpose**: Coordinate exclusive talk access across clients

**Components**:
- `PttState`: Tracks which session (if any) is transmitting
- Broadcast channel: Notifies all clients of PTT state changes
- Session-based locking: Only one client can transmit at a time

**Flow**:
1. Client requests PTT via WebSocket
2. Server attempts to acquire lock
3. If granted: Start reverse audio transcoding, notify all clients
4. If denied: Send "busy" message to requesting client
5. On release: Stop transcoding, notify all clients

### 6. Main Application (`src/main.rs`)

**Purpose**: Application entry point and orchestration

**Responsibilities**:
- Environment configuration loading
- DoorBird client initialization
- Fanout system creation
- WebRTC infrastructure setup
- Web server routing
- Event monitoring (doorbell, motion)

**Routes**:
- `GET /intercom`: Serve intercom web interface
- `GET /ws`: WebSocket signaling endpoint
- `POST /api/open-gates`: Door control API
- `GET /static/*`: Static assets (PWA manifest, icons)

## Data Flow

### Audio Streaming (DoorBird → Browser)

```
DoorBird Device
    ↓ HTTP stream (G.711 μ-law @ 8kHz)
doorbird::Client::audio_receive()
    ↓ Raw bytes
AudioFanout::stream_audio()
    ↓ Process in background
AudioTranscoder::process_chunk()
    ↓ Decode, resample, encode
OpusSample (Opus @ 48kHz, 20ms frames)
    ↓ Broadcast channel
WebRtcSession (N subscribers)
    ↓ RTP packets
Browser WebRTC (all connected clients)
```

### Video Streaming (DoorBird → Browser)

```
DoorBird Device
    ↓ RTSP stream (H.264 @ 12fps)
H264Extractor::next_packet()
    ↓ ffmpeg extraction
H264Packet (raw NAL units)
    ↓ Broadcast channel
VideoFanout (N subscribers)
    ↓ Zero processing
WebRtcSession (all connected clients)
    ↓ RTP packets
Browser WebRTC (all connected clients)
```

### Push-to-Talk (Browser → DoorBird)

```
Browser WebRTC
    ↓ RTP packets (Opus @ 48kHz)
WebRtcSession::on_track()
    ↓ Receive audio track
ReverseAudioTranscoder::process_chunk()
    ↓ Decode, resample, encode
G.711 μ-law @ 8kHz
    ↓ HTTP POST stream
doorbird::Client::audio_transmit()
    ↓ Continuous POST
DoorBird Device (speaker output)
```

## Key Design Decisions

### 1. Fanout Architecture

**Decision**: Single DoorBird connection shared across all clients

**Rationale**:
- DoorBird supports only one simultaneous audio/video consumer
- Official app has precedence and will interrupt API connections
- Efficient resource usage (one RTSP stream, one HTTP stream)
- Consistent quality across all viewers

**Implementation**:
- Broadcast channels distribute media to all subscribers
- Connection lifecycle tied to subscriber count
- Grace periods prevent flapping during viewer transitions

### 2. Zero Video Transcoding

**Decision**: Forward H.264 packets directly without re-encoding

**Rationale**:
- DoorBird already outputs H.264 (WebRTC compatible)
- Transcoding adds latency and CPU load
- Preserves original quality
- Simpler architecture

**Trade-off**: Locked to DoorBird's video parameters (resolution, bitrate)

### 3. Audio Transcoding

**Decision**: Transcode G.711 μ-law → Opus

**Rationale**:
- G.711 μ-law not well-supported in web browsers
- Opus provides better quality at lower bitrate
- Opus is WebRTC standard for audio
- Resampling required anyway (8kHz → 48kHz)

**Trade-off**: Adds ~42-52ms latency (acceptable for intercom use)

### 4. UDP Multiplexing

**Decision**: Single UDP port for all WebRTC sessions

**Rationale**:
- Simplifies firewall/Docker port mapping
- Reduces ephemeral port exhaustion risk
- Industry standard for WebRTC servers

**Implementation**: webrtc-rs `UDPMuxDefault`

### 5. No STUN/TURN Servers

**Decision**: Direct connection without STUN/TURN

**Rationale**:
- Server-client architecture (not peer-to-peer)
- Server IP explicitly configured (`BIRDBOX_HOST_IP`)
- Both server and clients on same LAN
- No NAT between server and clients
- STUN/TURN adds complexity and latency

**When it works**: Browser can reach HTTP server → can reach WebRTC UDP port

### 6. Split-Brain DNS Support

**Decision**: Optional dual-IP advertising

**Rationale**:
- Support both internal and external clients
- Internal clients use LAN IP (fast, direct)
- External clients use public IP (NAT forwarding)
- No dependency on router hairpin NAT

**Configuration**: `BIRDBOX_HOST_IP` + `BIRDBOX_HOST_IP_LAN` env vars

### 7. Configurable Buffers

**Decision**: Runtime-configurable buffer sizes

**Rationale**:
- Different networks have different characteristics
- VPN/Docker scenarios need larger buffers
- Low-latency scenarios can use smaller buffers
- Operator can tune for their specific deployment

**Parameters**:
- `BIRDBOX_AUDIO_FANOUT_BUFFER_SAMPLES`: Audio latency vs reliability
- `BIRDBOX_VIDEO_FANOUT_BUFFER_FRAMES`: Video latency vs smoothness
- `BIRDBOX_RTSP_TRANSPORT_PROTOCOL`: TCP vs UDP reliability

## Module Responsibilities

| Module               | Responsibility                     | Key Types                                   |
| -------------------- | ---------------------------------- | ------------------------------------------- |
| `main.rs`            | Application orchestration, routing | `AppState`, `PttState`                      |
| `doorbird/`          | DoorBird API client library        | `Client`, `DeviceInfo`                      |
| `audio_fanout.rs`    | Audio connection lifecycle         | `AudioFanout`, `OpusSample`                 |
| `video_fanout.rs`    | Video connection lifecycle         | `VideoFanout`, `H264Packet`                 |
| `audio_transcode.rs` | Bidirectional audio conversion     | `AudioTranscoder`, `ReverseAudioTranscoder` |
| `h264_extractor.rs`  | RTSP H.264 extraction              | `H264Extractor`, `H264Packet`               |
| `webrtc.rs`          | WebRTC infrastructure              | `WebRtcInfra`, `WebRtcSession`              |
| `g711.rs`            | G.711 μ-law codec                  | `encode_ulaw()`, `decode_ulaw()`            |

## Error Handling Strategy

1. **Connection Failures**: Auto-reconnect with exponential backoff
2. **Stream Interruptions**: Graceful degradation, attempt recovery
3. **Transcoding Errors**: Log and skip frame, continue stream
4. **WebRTC Failures**: Close session, client can reconnect
5. **DoorBird Busy**: Inform user, retry after delay

## Testing Strategy

- Unit tests for codecs (G.711, audio transcoding)
- Integration tests for DoorBird client (doctests)
- Manual testing for WebRTC (requires browser)
- Load testing for multiple concurrent clients

## Performance Characteristics

- **Memory**: ~50MB base + ~5MB per connected client
- **CPU**: ~10-20% single core (mostly audio resampling)
- **Network**: ~500-800 Kbps per client (video dominant)
- **Latency**: Audio ~400ms, Video ~500ms-1s (configurable)

## Future Considerations

- **Recording**: Store streams to disk
- **Cloud Integration**: TURN server for remote access
- **Mobile Apps**: Native iOS/Android clients
- **Multi-Device**: Support multiple DoorBird devices
- **Analytics**: Connection quality metrics, uptime tracking

