# Birdbox-RS

WebRTC audio server built with Rust, sending a 440Hz tone to clients.

## Running with Docker Compose

### Prerequisites

- Docker Desktop for Mac
- Your Mac's LAN IP address

### Setup

1. **Find your Mac's LAN IP address:**
   ```bash
   ipconfig getifaddr en0
   # Example output: 192.168.1.100
   ```

2. **Set the HOST_IP environment variable:**
   
   Create a `.env` file in the project root:
   ```bash
   HOST_IP=192.168.1.100  # Replace with your actual LAN IP
   UDP_PORT=50000         # Single UDP port for WebRTC (using UDP mux)
   RUST_LOG=info
   ```

3. **Build and start the service:**
   ```bash
   docker compose up --build
   ```

   Or run in detached mode:
   ```bash
   docker compose up -d --build
   ```

4. **Access the web interface:**
   
   From your Mac:
   ```
   http://localhost:3000/intercom
   ```
   
   From other devices on your LAN:
   ```
   http://192.168.1.100:3000/intercom  # Use your Mac's IP
   ```

### Stopping the Service

```bash
docker compose down
```

### Viewing Logs

```bash
docker compose logs -f birdbox
```

### Troubleshooting

**WebRTC connection fails:**
- Verify HOST_IP is set to your Mac's current LAN IP
- Ensure UDP port 50000 is not blocked by your firewall
- Check the logs for ICE candidate details
- Ensure both devices are on the same LAN
- If still failing, you may need to add a TURN server

**UDP Mux:**
- The server uses UDP multiplexing to handle all WebRTC traffic over a single UDP port (50000)
- This simplifies Docker port mapping and firewall configuration
- Multiple simultaneous connections share the same port

**Audio not playing:**
- Check browser console for WebRTC errors
- Verify the ICE connection state reaches "connected"
- Try refreshing the page and clicking "Connect" again

## Development (without Docker)

```bash
# Run locally
RUST_LOG=info cargo run

# Access at http://localhost:3000/intercom
```

## Architecture

- **Axum**: Web server and WebSocket handling
- **webrtc-rs**: WebRTC peer connection and media streaming
- **audiopus**: Opus audio encoding for the 440Hz test tone
- **Askama**: HTML templating

The server generates a 440Hz sine wave tone, encodes it to Opus format, and streams it to connected clients via WebRTC.

