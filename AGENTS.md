# Guidelines for AI Assistants

## Project Context

This is **Birdbox**, a WebRTC and API gateway for DoorBird smart doorbells. It streams audio/video from DoorBird devices to web browsers with low latency and supports push-to-talk communication.

## Build & Development Rules

- **Don't build in --release mode** unless explicitly asked - we stick to dev builds for development
- Use **"docker compose"** (NOT "docker-compose") - we use the modern Docker Compose V2
- The host machine is a Mac for developing, and linux (typically ubuntu) for deployment. Consider the implications. For example, docker on mac has no host networking mode.

## Documentation Structure

All technical documentation is in the **`docs/`** folder:

- **`docs/ARCHITECTURE.md`**: System design, components, data flow, key design decisions
- **`docs/DEVELOPMENT.md`**: Building, testing, contributing, code structure
- **`docs/LATENCY.md`**: Performance tuning, buffer configuration, latency analysis
- **`docs/NETWORKING.md`**: WebRTC, Docker networking, ICE candidates, troubleshooting
- **`docs/PWA.md`**: Progressive Web App installation and configuration
- **`docs/DOORBIRD_API.md`**: Official DoorBird LAN API reference

**Before making changes**: Review relevant docs to understand current architecture and design decisions.

## Code Organization

### Key Modules

- **`src/main.rs`**: Application entry point, HTTP server, orchestration
- **`doorbird/`**: DoorBird API client library (separate crate)
- **`src/audio_fanout.rs`**: Audio connection lifecycle and distribution
- **`src/video_fanout.rs`**: Video connection lifecycle and distribution  
- **`src/audio_transcode.rs`**: Audio format conversion (G.711 ↔ Opus)
- **`src/h264_extractor.rs`**: RTSP video stream extraction
- **`src/webrtc.rs`**: WebRTC infrastructure and session management
- **`src/g711.rs`**: G.711 μ-law codec implementation

### Important Patterns

- **Fanout architecture**: One DoorBird connection serves N WebRTC clients
- **Grace periods**: Prevent connection flapping (3s audio, 5s video)
- **Broadcast channels**: Distribute media samples to multiple consumers
- **Zero video transcoding**: H.264 packets forwarded directly
- **ICE control**: Specific IP binding to prevent multiple candidates

## Configuration

All configuration is via environment variables in `.env`:
- `BIRDBOX_DOORBIRD_*`: DoorBird device connection
- `BIRDBOX_HOST_IP` / `BIRDBOX_HOST_IP_LAN`: WebRTC IP advertising (split-brain DNS support)
- `BIRDBOX_*_BUFFER_*`: Latency vs reliability tuning
- `BIRDBOX_RTSP_TRANSPORT_PROTOCOL`: TCP (reliable) vs UDP (low latency)

See `env.example` for full documentation of all options.

## Code Style

- **Format**: Use `cargo fmt` before committing
- **Lints**: Use `cargo clippy` to catch common issues
- **Documentation**: Add doc comments (`///`) for public APIs and module-level docs (`//!`)
- **Inline comments**: Explain *why*, not *what* - especially for complex logic
- **Naming**: Use descriptive names, include units in constants (`_MS`, `_SECS`)
- **Errors**: Use `.context()` from `anyhow` to add descriptive error messages

## Testing

- **Unit tests**: `cargo test`
- **Manual testing**: Required for WebRTC functionality
- **Log levels**: Use `RUST_LOG=debug` for verbose output

## Common Gotchas

- **Docker UDP mapping**: Must include `/udp` suffix in `docker-compose.yml`
- **BIRDBOX_HOST_IP required**: For Docker deployments, must set to host's LAN IP
- **ffmpeg options**: `nobuffer` and `low_delay` flags are critical for latency
- **DoorBird limits**: Only one API client at a time; official app has precedence
- **Resampler latency**: `sinc_len: 256` trades quality for ~32ms delay

## When Making Changes

1. **Read relevant docs** in `docs/` folder first
2. **Check existing patterns** - follow established code style
3. **Consider latency impact** - many design decisions prioritize low latency
4. **Test manually** - WebRTC requires browser testing
5. **Update documentation** - Keep docs in sync with code changes

## Key Design Philosophy

- **Reliability over ultra-low latency**: Doorbell/intercom use case tolerates 400-500ms delay
- **Simplicity**: Avoid complexity where possible (e.g., no service worker in PWA)
- **Zero transcoding for video**: Pass-through H.264 to avoid latency and CPU
- **Smart resource management**: Connect only when needed, graceful cleanup
- **Explicit configuration**: No magic - clear env vars for all tunables
