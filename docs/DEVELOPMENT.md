# Development Guide

This document provides information for developers who want to build, modify, or contribute to Birdbox.

## Prerequisites

### System Requirements

- **Rust**: 1.90 or newer
- **ffmpeg**: 4.x or newer (with H.264 support)
- **libopus**: 1.3 or newer
- **pkg-config**: For finding system libraries
- **clang/LLVM**: For building ffmpeg-sys

### Platform-Specific Setup

#### macOS

```bash
# Install dependencies via Homebrew
brew install ffmpeg opus pkg-config

# Install Rust (if not already installed)
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.up | sh
```

#### Ubuntu/Debian Linux

```bash
# Install dependencies
sudo apt-get update
sudo apt-get install -y \
    libavcodec-dev \
    libavformat-dev \
    libavutil-dev \
    libswscale-dev \
    libopus-dev \
    pkg-config \
    clang \
    llvm-dev \
    libclang-dev

# Install Rust
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

#### Windows

Not officially supported, but may work with:
- Install Visual Studio Build Tools
- Install ffmpeg via vcpkg or pre-built binaries
- Install Opus via vcpkg

## Building from Source

### Development Build

```bash
# Clone the repository
git clone https://github.com/yourusername/birdbox.git
cd birdbox

# Copy environment template
cp env.example .env

# Edit .env with your DoorBird credentials
vim .env

# Build (debug mode)
cargo build

# Run with logging
RUST_LOG=debug cargo run
```

Access the interface at http://localhost:3000/intercom

### Release Build

```bash
cargo build --release

# Binary will be in target/release/birdbox-rs
./target/release/birdbox-rs
```

**Note**: Per project conventions, we typically use debug builds for development. Only build in release mode when needed for performance testing or deployment.

### Docker Build

```bash
# Build Docker image
docker compose build

# Run with Docker Compose
docker compose up -d

# View logs
docker compose logs -f birdbox
```

## Project Structure

```
birdbox/
├── src/
│   ├── main.rs              # Application entry point
│   ├── audio_fanout.rs      # Audio connection management
│   ├── video_fanout.rs      # Video connection management
│   ├── audio_transcode.rs   # Audio format conversion
│   ├── h264_extractor.rs    # RTSP H.264 extraction
│   ├── webrtc.rs            # WebRTC infrastructure
│   └── g711.rs              # G.711 μ-law codec
├── doorbird/
│   ├── Cargo.toml           # Library manifest
│   └── src/
│       └── lib.rs           # DoorBird API client
├── templates/
│   ├── base.html            # Base HTML template
│   └── intercom.html        # Intercom interface
├── static/
│   ├── manifest.json        # PWA manifest
│   └── icon.svg             # PWA icon
├── docs/                    # Technical documentation
├── Cargo.toml               # Workspace manifest
├── docker-compose.yml       # Docker Compose config
├── Dockerfile               # Container build
└── env.example              # Configuration template
```

## Code Organization

### Module Responsibilities

| Module               | Purpose                        | Key Types                      |
| -------------------- | ------------------------------ | ------------------------------ |
| `main.rs`            | App orchestration, HTTP server | `AppState`, `PttState`         |
| `doorbird/`          | DoorBird API client library    | `Client`, `DeviceInfo`         |
| `audio_fanout.rs`    | Audio streaming lifecycle      | `AudioFanout`, `OpusSample`    |
| `video_fanout.rs`    | Video streaming lifecycle      | `VideoFanout`, `H264Packet`    |
| `audio_transcode.rs` | Audio codec conversion         | `AudioTranscoder`              |
| `h264_extractor.rs`  | RTSP video extraction          | `H264Extractor`                |
| `webrtc.rs`          | WebRTC peer connections        | `WebRtcInfra`, `WebRtcSession` |
| `g711.rs`            | G.711 μ-law codec              | Functions                      |

### Dependency Graph

```
main.rs
  ├─→ doorbird::Client
  ├─→ audio_fanout::AudioFanout
  │     ├─→ audio_transcode::AudioTranscoder
  │     │     └─→ g711 (encode/decode)
  │     └─→ doorbird::Client
  ├─→ video_fanout::VideoFanout
  │     └─→ h264_extractor::H264Extractor
  └─→ webrtc::WebRtcInfra
        └─→ webrtc::WebRtcSession
              ├─→ audio_fanout::AudioFanout
              └─→ video_fanout::VideoFanout
```

## Running Tests

### Unit Tests

```bash
# Run all tests
cargo test

# Run tests with output
cargo test -- --nocapture

# Run specific test
cargo test test_decode_silence

# Run tests in specific module
cargo test audio_transcode
```

### Integration Tests

Currently, most integration testing is manual due to WebRTC's requirement for real browsers.

**Manual Test Checklist**:
- [ ] Audio streaming works
- [ ] Video streaming works
- [ ] Push-to-talk works
- [ ] Door open button works
- [ ] Multiple simultaneous clients work
- [ ] Connection survives network interruption
- [ ] Graceful degradation when DoorBird busy

### Test Coverage

Current test coverage:
- ✅ G.711 μ-law codec (encode/decode)
- ✅ Audio transcoder (creation, processing)
- ✅ DoorBird client (doctests)
- ❌ WebRTC (requires browser)
- ❌ Fanout systems (requires integration test)
- ❌ End-to-end (manual only)

## Development Workflow

### Typical Development Cycle

1. **Make changes** to source code
2. **Run tests**: `cargo test`
3. **Check compilation**: `cargo check`
4. **Run locally**: `RUST_LOG=debug cargo run`
5. **Test in browser**: Open http://localhost:3000/intercom
6. **Check logs**: Watch terminal output for errors
7. **Iterate**: Repeat until working

### Hot Reload

For template changes:
- Templates are loaded at runtime (via Askama)
- Restart server to see changes
- No recompilation needed

For Rust code changes:
- Full recompilation required
- Use `cargo watch` for automatic rebuilds:
  ```bash
  cargo install cargo-watch
  cargo watch -x run
  ```

### Logging

Control log levels via `RUST_LOG` environment variable:

```bash
# Error only
RUST_LOG=error cargo run

# Warnings and errors
RUST_LOG=warn cargo run

# Info, warnings, errors (recommended)
RUST_LOG=info cargo run

# Debug (verbose)
RUST_LOG=debug cargo run

# Trace (very verbose)
RUST_LOG=trace cargo run

# Module-specific logging
RUST_LOG=birdbox_rs=debug,doorbird=trace cargo run
```

## Code Style Guidelines

### Rust Style

Follow standard Rust conventions:
- Use `rustfmt` for formatting: `cargo fmt`
- Use `clippy` for lints: `cargo clippy`
- Write documentation comments for public APIs
- Add inline comments for complex logic

### Documentation Standards

**Module-level documentation**:
```rust
//! Brief module description
//!
//! Detailed explanation of what this module does,
//! key concepts, and usage examples.
```

**Function documentation**:
```rust
/// Brief one-line description
///
/// Longer explanation if needed.
///
/// # Arguments
/// * `param` - Description
///
/// # Returns
/// Description of return value
///
/// # Errors
/// When this function might fail
///
/// # Examples
/// ```
/// let result = function(42);
/// ```
pub fn function(param: i32) -> Result<String> {
    // ...
}
```

**Inline comments for complex code**:
```rust
// Explain WHY, not WHAT
// Good: "Bind to specific IP to prevent multiple ICE candidates"
// Bad: "Create UDP socket"
```

### Naming Conventions

- **Constants**: `SCREAMING_SNAKE_CASE`
- **Functions/variables**: `snake_case`
- **Types/traits**: `PascalCase`
- **Modules**: `snake_case`

**Good names**:
- `audio_fanout` (clear purpose)
- `RECONNECT_DELAY_SECS` (units in name)
- `OpusSample` (describes what it contains)

**Avoid**:
- Single-letter variables (except loop counters)
- Abbreviations (unless very common: `id`, `url`)
- Hungarian notation

## Adding New Features

### Adding a New DoorBird API Endpoint

1. **Add method to `doorbird/src/lib.rs`**:
```rust
impl Client {
    /// Description of what this does
    pub async fn new_api_call(&self) -> Result<ResponseType> {
        let url = format!("{}/bha-api/endpoint.cgi", self.base_url);
        let response = self.client
            .get(&url)
            .basic_auth(&self.username, Some(&self.password))
            .send()
            .await?;
        // Parse and return
    }
}
```

2. **Add tests** (if possible)
3. **Update documentation**
4. **Add example usage** to docs

### Adding Configuration Options

1. **Add to `env.example`**:
```bash
# New Feature Configuration
# Description of what this controls
NEW_OPTION=default_value
```

2. **Read in `main.rs`**:
```rust
let new_option = std::env::var("NEW_OPTION")
    .unwrap_or_else(|_| "default_value".to_string());
```

3. **Pass to relevant component**
4. **Document in README.md** and relevant docs

### Adding New WebRTC Track

1. **Create track in `webrtc.rs`**
2. **Add to peer connection**
3. **Update SDP handling**
4. **Update JavaScript client** in `templates/intercom.html`
5. **Test thoroughly** in browser

## Debugging

### Common Issues

**Build fails with ffmpeg errors**:
```bash
# Check ffmpeg installation
ffmpeg -version
pkg-config --modversion libavcodec

# Set PKG_CONFIG_PATH if needed
export PKG_CONFIG_PATH=/usr/local/lib/pkgconfig
```

**WebRTC connection fails**:
```bash
# Check logs for ICE candidates
RUST_LOG=debug cargo run

# Verify BIRDBOX_HOST_IP is correct
echo $BIRDBOX_HOST_IP
ifconfig  # or: ip addr show

# Test UDP port
nc -ul 50000  # Listen
nc -u localhost 50000  # Connect from another terminal
```

**Audio/Video stuttering**:
- Check CPU usage: `top` or `htop`
- Increase buffer sizes in `.env`
- Check network: `ping DOORBIRD_IP`

### Debugging Tools

**Rust debugging**:
```bash
# With rust-gdb (Linux)
rust-gdb target/debug/birdbox-rs

# With rust-lldb (macOS)
rust-lldb target/debug/birdbox-rs
```

**WebRTC debugging**:
- Chrome: `chrome://webrtc-internals`
- Firefox: `about:webrtc`
- Safari: Develop → WebRTC

**Network debugging**:
```bash
# Monitor traffic
tcpdump -i any port 50000

# Check connections
netstat -an | grep 50000
lsof -i UDP:50000
```

## Performance Profiling

### CPU Profiling

```bash
# Install flamegraph
cargo install flamegraph

# Generate profile
cargo flamegraph

# Open flamegraph.svg in browser
```

### Memory Profiling

```bash
# Use valgrind (Linux)
valgrind --leak-check=full target/debug/birdbox-rs

# Use instruments (macOS)
instruments -t Leaks target/debug/birdbox-rs
```

### Benchmarking

```bash
# Add benchmarks to benches/
# Run with:
cargo bench
```

## Contributing

### Before Submitting PR

1. **Run tests**: `cargo test`
2. **Format code**: `cargo fmt`
3. **Check lints**: `cargo clippy`
4. **Update documentation**: If adding features
5. **Test manually**: Verify in browser
6. **Check commit message**: Clear description of changes

### PR Guidelines

- Clear title describing the change
- Description of what and why
- Link to related issues
- Screenshots/video for UI changes
- Breaking changes clearly marked

### Code Review Checklist

- [ ] Tests pass
- [ ] Code formatted (`cargo fmt`)
- [ ] No new clippy warnings
- [ ] Documentation updated
- [ ] No breaking changes (or clearly documented)
- [ ] Manually tested
- [ ] Error handling appropriate
- [ ] Logging appropriate

## Release Process

### Version Numbering

Follow Semantic Versioning:
- **Major**: Breaking changes
- **Minor**: New features, backwards compatible
- **Patch**: Bug fixes

### Creating a Release

1. **Update version** in `Cargo.toml`
2. **Update CHANGELOG** (if exists)
3. **Test thoroughly**
4. **Tag release**: `git tag v0.2.0`
5. **Push tag**: `git push origin v0.2.0`
6. **Build release**: `cargo build --release`
7. **Create GitHub release** with binary artifacts

## Useful Resources

### Rust & WebRTC

- [Rust Book](https://doc.rust-lang.org/book/)
- [webrtc-rs Documentation](https://docs.rs/webrtc/)
- [WebRTC Spec](https://www.w3.org/TR/webrtc/)
- [Tokio Tutorial](https://tokio.rs/tokio/tutorial)

### Audio/Video

- [FFmpeg Documentation](https://ffmpeg.org/documentation.html)
- [Opus Codec](https://opus-codec.org/)
- [H.264 Spec](https://www.itu.int/rec/T-REC-H.264)
- [G.711](https://en.wikipedia.org/wiki/G.711)

### DoorBird

- [DoorBird LAN API](docs/DOORBIRD_API.md) (included in this repo)
- [DoorBird Official Site](https://www.doorbird.com/)

## Getting Help

### Troubleshooting Steps

1. **Check existing documentation**:
   - [README.md](../README.md)
   - [ARCHITECTURE.md](ARCHITECTURE.md)
   - [NETWORKING.md](NETWORKING.md)
   - [LATENCY.md](LATENCY.md)

2. **Search issues** on GitHub

3. **Check logs** with `RUST_LOG=debug`

4. **Verify configuration** in `.env`

5. **Test basic connectivity**:
   - Can you access DoorBird directly?
   - Can browser reach the web server?
   - Is UDP port open?

### Reporting Issues

Include:
- Rust version: `rustc --version`
- OS and version
- Docker version (if using)
- Relevant logs (with `RUST_LOG=debug`)
- Configuration (redact credentials)
- Steps to reproduce
- Expected vs actual behavior

## Development Tips

### Faster Iteration

```bash
# Use cargo watch for auto-rebuild
cargo watch -x 'run'

# Check without building
cargo check

# Run clippy on save
cargo watch -x clippy
```

### Testing Without DoorBird

For testing non-DoorBird-specific features:
- Mock the `doorbird::Client` trait
- Use test fixtures for audio/video data
- Test WebRTC with synthetic streams

### Understanding the Codebase

**Start here**:
1. Read `main.rs` to understand app structure
2. Look at `doorbird/src/lib.rs` for API
3. Trace audio flow: `audio_fanout.rs` → `audio_transcode.rs` → `g711.rs`
4. Trace video flow: `video_fanout.rs` → `h264_extractor.rs`
5. Study WebRTC: `webrtc.rs`
6. Review templates: `templates/intercom.html`

**Key concepts**:
- Fanout pattern for 1:N streaming
- Broadcast channels for distribution
- Grace periods for connection management
- UDP mux for single-port WebRTC
- ICE candidate control for Docker

## FAQ

**Q: Why Rust instead of Node.js/Python?**
A: Performance, type safety, and excellent WebRTC libraries. Low memory footprint for embedded deployment.

**Q: Can I run multiple instances?**
A: No, DoorBird supports only one API client at a time. Use the fanout to serve multiple viewers from one instance.

**Q: Why not transcode video?**
A: DoorBird already outputs H.264, which WebRTC supports. Transcoding adds latency and CPU load for no benefit.

**Q: Can I add feature X?**
A: Probably! Check the architecture docs, open an issue to discuss, then submit a PR.

**Q: Where do I start contributing?**
A: Look for "good first issue" tags, or tackle documentation improvements, add tests, or improve error messages.

---

Happy coding! If you have questions, open an issue or discussion on GitHub.

