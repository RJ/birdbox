# Birdbox

An API proxy that presents a more modern doorbird API, including two-way browser-friendly WebRTC.

* **Fan-out** will open 1 connection (on-demand) to doorbird for audio and video, and fan-out to any number of clients.
* **Low-latency** best efforts made to reduce latency whereever possible
* **WebRTC

## Features

- **Real-time Audio & Video Streaming**: H.264 video and Opus audio via WebRTC
- **Push-to-Talk**: Two-way communication with automatic G.711 μ-law transcoding
- **Low Latency**: Tuned as best I can for low end-to-end latency
- **Smart Fan-out**: A single DoorBird audio/video connection shared across multiple viewers
- **Smart Connection Management**: Automatic connect/disconnect based on viewer presence
- **Progressive Web App**: Install on mobile devices for app-like experience
- **Door Control**: Open gates/doors remotely via web interface
- **Dockerized**

## Quick Start

### Prerequisites

- Docker Desktop (for macOS/Windows) or Docker + Docker Compose (for Linux)
- DoorBird smart doorbell on your local network
- DoorBird user credentials with "watch always" permission

### 1. Configuration

Create a `.env` file in the project root:

```bash
# DoorBird Configuration
DOORBIRD_URL=http://192.168.1.100
DOORBIRD_USER=your_username
DOORBIRD_PASSWORD=your_password

# WebRTC Configuration
HOST_IP=192.168.1.50          # Your Docker host's IP address
UDP_PORT=50000                # WebRTC media port

# Optional: Latency tuning (defaults shown)
AUDIO_FANOUT_BUFFER_SAMPLES=20    # ~400ms audio latency
VIDEO_FANOUT_BUFFER_FRAMES=4      # ~330ms video latency
RTSP_TRANSPORT_PROTOCOL=udp       # or "tcp" for VPN/complex networks

# Logging
RUST_LOG=info
```

See [`env.example`](env.example) for full configuration options with detailed explanations.

### 2. Start the Service

```bash
docker compose up -d
```

### 3. Access the Interface

Open in your browser:
- **From Docker host**: http://localhost:3000/intercom
- **From other devices**: http://YOUR_HOST_IP:3000/intercom

## Architecture Overview

```
┌─────────────────┐
│  DoorBird       │
│  Smart Doorbell │
└────────┬────────┘
         │ HTTP/RTSP
         │ G.711 μ-law audio @ 8kHz
         │ H.264 video @ 12fps
         │
    ┌────▼─────────────────────────────────┐
    │  Birdbox Server (Rust)               │
    │                                       │
    │  ┌─────────────────────────────────┐ │
    │  │ Audio Pipeline                  │ │
    │  │ G.711 → Resample → Opus         │ │
    │  └─────────────────────────────────┘ │
    │                                       │
    │  ┌─────────────────────────────────┐ │
    │  │ Video Pipeline                  │ │
    │  │ H.264 pass-through (no transcode)│ │
    │  └─────────────────────────────────┘ │
    │                                       │
    │  ┌─────────────────────────────────┐ │
    │  │ Fanout System                   │ │
    │  │ Single DoorBird → N clients     │ │
    │  └─────────────────────────────────┘ │
    └────────┬──────────────────────────────┘
             │ WebRTC (WebSocket signaling)
             │ Opus audio @ 48kHz
             │ H.264 video
             │
    ┌────────▼──────────────────┐
    │  Web Browsers             │
    │  Chrome, Firefox, Safari  │
    │  Desktop & Mobile         │
    └───────────────────────────┘
```

**Key Design:**
- **Fanout Architecture**: One DoorBird connection serves multiple viewers
- **Zero Video Transcoding**: H.264 packets forwarded directly to WebRTC
- **Smart Lifecycle**: Auto-connect when first viewer joins, auto-disconnect after grace period
- **Direct LAN Communication**: No STUN/TURN servers needed for local network

## Configuration

### Network Setup

For optimal performance:
- Place Docker host and DoorBird on the same LAN segment
- Configure `HOST_IP` to your Docker host's LAN IP
- Ensure UDP port 50000 is not blocked by firewall

### Latency Tuning

**Audio latency** (default: ~400ms):
```bash
AUDIO_FANOUT_BUFFER_SAMPLES=20  # Lower = less latency, more dropouts
```

**Video latency** (default: ~330ms):
```bash
VIDEO_FANOUT_BUFFER_FRAMES=4    # Lower = less latency, more frame drops
```

**Network reliability** (VPN/Docker):
```bash
RTSP_TRANSPORT_PROTOCOL=tcp     # Use TCP for VPN scenarios
```

See [docs/LATENCY.md](docs/LATENCY.md) for detailed tuning guide.

## Troubleshooting

### WebRTC Connection Fails
- Verify `HOST_IP` is set to your Docker host's actual LAN IP
- Check that UDP port 50000 is not blocked by firewall
- Ensure browser and Docker host are on the same network
- Check logs: `docker compose logs -f birdbox`

### Audio/Video Stuttering
- Increase buffer sizes in `.env`
- For VPN deployments, set `RTSP_TRANSPORT_PROTOCOL=tcp`
- Check network bandwidth and quality

### Video Stream Hangs
- If using VPN: switch to TCP transport (`RTSP_TRANSPORT_PROTOCOL=tcp`)
- Verify DoorBird is reachable from Docker container
- Check for UDP packet loss on network

See [docs/NETWORKING.md](docs/NETWORKING.md) for advanced troubleshooting.

## Development

### Building from Source

```bash
# Install dependencies (macOS example)
brew install ffmpeg opus

# Build and run
cargo build
RUST_LOG=info cargo run
```

Access at http://localhost:3000/intercom

### Running Tests

```bash
cargo test
```

### Documentation

For developers:
- [Architecture Overview](docs/ARCHITECTURE.md) - System design and components
- [Development Guide](docs/DEVELOPMENT.md) - Building and contributing
- [Latency Analysis](docs/LATENCY.md) - Performance tuning details
- [Networking Guide](docs/NETWORKING.md) - WebRTC and Docker networking
- [DoorBird API Reference](docs/DOORBIRD_API.md) - Official API documentation
- [PWA Setup](docs/PWA.md) - Progressive Web App installation

## Technology Stack

- **Rust** - High-performance systems language
- **Axum** - Web server and WebSocket handling
- **webrtc-rs** - WebRTC implementation
- **ffmpeg** - Video stream handling
- **Opus** - Audio encoding
- **Askama** - HTML templating
- **Docker** - Containerization

## License

TODO

## Contributing

Contributions welcome! Please check [docs/DEVELOPMENT.md](docs/DEVELOPMENT.md) for guidelines.
