# Birdbox - a DoorBird WebRTC Proxy

An API proxy that exposes a DoorBird device via WebRTC to browsers.

* WebRTC video repackaged from doorbird's rtsp stream at highest available resolution.
* WebRTC 2-way audio, with the necessary transcoding.
* Smart fan-out, so even with multiple clients, birdbox only makes 1 connection to doorbird's A/V streams.
* UDP muxing and no STUN/TURN needed. WebRTC exposed on a single fixed UDP port.
* Single docker image deployment.
* Low latency / overhead.
* PWA web app (add to homescreen like an app).
* Relay control (Open gates button).

<img src="https://github.com/RJ/birdbox/blob/main/docs/birdbox-web-screenshot.png">

## Why does this exist

I wanted a better integration with home assistant. not written that yet, but i decided it needed
webrtc support first.

Also I wanted to add some features to the doorbird app relating to recognising visitors, so having
a web based client i could iterate on seemed like a good idea.

I used my 25 years of experience as a software developer to make an LLM write this entire project,
and it went surprisingly well. I didn't write a single line of code. So it was partly an experiment.


### 1. Configuration

Create a `.env` file in the project root or set these vars for your docker deployment somehow.
(also for my convenience, birdbox attempts to read a .env file from the current directory on boot)

```bash
# DoorBird Configuration
BIRDBOX_DOORBIRD_URL=http://192.168.1.100
BIRDBOX_DOORBIRD_USER=abcdef0001
BIRDBOX_DOORBIRD_PASSWORD=your_password

# WebRTC Configuration - public ip that exposes the webrtc service (and port 50000/udp).
BIRDBOX_HOST_IP=192.168.1.50
# change to tcp if there's any udp-over-udp tunneling happening, such as
# using a vpn or complex docker networking.
BIRDBOX_RTSP_TRANSPORT_PROTOCOL=udp

# more options and docs in env.example
```


## Troubleshooting

### WebRTC Connection Fails
- Verify `BIRDBOX_HOST_IP` is set to your Docker host's actual LAN IP
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

### Dev Docs

For developers / LLM agents:
- [Architecture Overview](docs/ARCHITECTURE.md) - System design and components
- [Development Guide](docs/DEVELOPMENT.md) - Building and contributing
- [Latency Analysis](docs/LATENCY.md) - Performance tuning details
- [Networking Guide](docs/NETWORKING.md) - WebRTC and Docker networking
- [DoorBird API Reference](docs/DOORBIRD_API.md) - Official API documentation
- [PWA Setup](docs/PWA.md) - Progressive Web App installation

## License

TODO

