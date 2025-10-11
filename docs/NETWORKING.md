# Networking Guide

This document explains WebRTC networking, Docker deployment, ICE candidate management, and network troubleshooting.

## WebRTC Network Topology

### Architecture: Client-Server (Not Peer-to-Peer)

Birdbox uses a **client-server WebRTC architecture**:

```
Browser Client ←→ WebRTC Server (Rust) ←→ DoorBird Device
    (LAN)         (same host as web UI)        (LAN)
```

**Key characteristics**:
- **Server-authoritative**: The Rust application acts as a WebRTC server endpoint
- **Co-located with web server**: WebRTC server runs on the same machine serving the web UI
- **Fixed UDP port**: Uses predictable port (default: 50000) for all WebRTC traffic
- **Direct LAN connectivity**: No NAT traversal needed between client and server on same network

### Why STUN/TURN Servers Are NOT Needed

**STUN servers** help peers discover their public IP addresses when behind NAT. We don't need this because:

1. **Server has known address**: The server IP is explicitly configured via `HOST_IP` environment variable or auto-detected
2. **Same host as web server**: If the browser can reach `http://HOST_IP/intercom`, it can reach `udp://HOST_IP:50000`
3. **Client-server model**: We're not doing P2P between two browsers behind different NATs
4. **Explicit ICE candidates**: Server advertises its specific IP address via NAT 1:1 mapping
5. **Fixed port mapping**: UDP port is predictable and accessible (mapped through Docker if needed)

**TURN servers** provide media relay when direct connection fails. We don't need this because:
- LAN clients connect directly to server on same network
- No firewall/NAT between browser and server (both on LAN)
- If client can't reach the UDP port, a TURN relay wouldn't help (same network issue)
- TURN adds latency and bandwidth costs for no benefit in our topology

**Bottom line**: The WebRTC connection uses the same IP and network path as the web page itself, just over UDP instead of TCP. No discovery or relay needed.

### Connection Flow

1. **Initial Contact**: Browser loads `/intercom` page from web server over HTTP
2. **WebSocket Handshake**: Client establishes WebSocket connection for signaling
3. **SDP Exchange**:
   - Client sends WebRTC offer with its capabilities
   - Server responds with answer containing audio/video tracks and ICE candidates
4. **ICE Candidate Exchange**:
   - Server advertises single host candidate (its configured IP + UDP port)
   - Client sends its candidates (server uses them to send media back)
5. **Media Flow**: Direct UDP connection established for audio/video streaming

## ICE Candidate Management

### Problem: Multiple Network Interfaces

When running natively (not in Docker), WebRTC can gather ICE candidates from all network interfaces:
- Localhost (127.0.0.1)
- LAN IP (e.g., 10.0.0.154)
- VPN interfaces
- mDNS `.local` hostnames

This causes connection issues when clients try to connect via the wrong interface.

### Solution: Three-Layer ICE Control

**Location**: `src/webrtc.rs`, lines 84-195

#### 1. Specific IP Binding (Primary Fix)

Binds UDP socket to specific IP: `10.0.0.154:50000` instead of `0.0.0.0:50000`

```rust
let udp_socket = UdpSocket::bind(&format!("{}:{}", host_ip, udp_port)).await?;
```

**Effect**:
- Forces OS to only use that network interface for UDP traffic
- Automatically prevents localhost, VPN, and other interface candidates
- Ensures clients connect to the correct IP

#### 2. mDNS Disable

Prevents `.local` hostname candidates:

```rust
setting_engine.set_ice_multicast_dns_mode(MulticastDnsMode::Disabled);
```

**When active**: Only when specific IP is set

#### 3. NAT 1:1 Mapping

Advertises external IP for Docker deployments:

```rust
setting_engine.set_nat_1to1_ips(
    vec![host_ip],
    RTCIceCandidateType::Host,
);
```

**Use case**: Container binds to 0.0.0.0 but advertises host's external IP

### Auto-Detection & Fallback

**Native Deployment** (no HOST_IP set):
- Auto-detects LAN IP using UDP socket trick (connects to 8.8.8.8 to determine routing interface)
- Binds directly to detected IP
- Logs: `"Auto-detected local IP: 10.0.0.X"`

**Docker Deployment** (HOST_IP set to external IP):
- Attempts to bind to HOST_IP
- If fails (container doesn't have that IP), falls back to 0.0.0.0 with NAT 1:1 mapping
- Logs: `"Could not bind to X.X.X.X, binding to 0.0.0.0 instead (Docker mode)"`

### Result

**Before ICE control**: Multiple candidates from all interfaces, clients may connect to wrong one
```
candidate: 127.0.0.1 ... typ host
candidate: 10.0.0.154 ... typ host
candidate: d8e0adbe...local ... typ host
candidate: (VPN) ... typ host
```

**After ICE control**: Single candidate with correct IP only
```
candidate: 10.0.0.154 50000 typ host
```

This ensures reliable WebRTC connections on both native and Docker deployments.

## Split-Brain DNS / Dual Network Topology

### Use Case

Support both internal LAN clients and external internet clients simultaneously:
- **Internal LAN clients**: Connect via private IP (fast, direct)
- **External clients**: Connect via public IP (NAT forwarding)
- **No hairpin NAT dependency**: Router doesn't need to route external IP back to internal network

### Configuration

Set both environment variables:

```bash
# Public IP for external clients
HOST_IP=203.0.113.50

# Private LAN IP for internal clients
HOST_IP_LAN=192.168.1.154
```

**Behavior**:
- Both IPs are advertised as ICE candidates
- WebRTC automatically selects the best route
- LAN clients use LAN IP (lower latency, no router traversal)
- External clients use public IP (NAT forwarding)

**Logging**:
```
🌐 Dual-network mode: advertising 2 ICE candidate(s): ["192.168.1.154", "203.0.113.50"]
```

### Port Forwarding Required

For external access, configure your router:
- Forward UDP port 50000 → internal server IP
- Forward TCP port 3000 → internal server IP (for HTTP/WebSocket)

## Docker Compose on macOS

### Platform Specifics

**Platform**: macOS with Docker Desktop + Docker Compose

Docker on macOS runs Linux containers in a VM (HyperKit/QEMU), which adds networking layers:

#### Networking Layers

```
Browser (LAN) → macOS Host → Docker VM → Linux Container (Rust app)
10.0.0.X        10.0.0.154    172.x.x.x    172.y.y.y
```

#### Challenges

**1. Port Forwarding Complexity**:
- TCP ports (HTTP/WebSocket) forward automatically through Docker's proxy
- UDP port 50000 requires explicit mapping in `docker-compose.yml`
- Traffic path: LAN → macOS → VM → Container

**2. IP Address Confusion**:
- Container sees its internal Docker IP (172.x.x.x)
- macOS host has LAN IP (e.g., 10.0.0.154)
- Browser needs to connect to LAN IP, not container IP
- Must use `HOST_IP` environment variable to advertise correct external IP

**3. Bind Address Limitation**:
- Container cannot bind directly to `HOST_IP` (doesn't own that interface)
- Must bind to `0.0.0.0` inside container
- Use NAT 1:1 mapping to advertise external IP in ICE candidates

### Docker Solution

**Location**: `src/webrtc.rs`, lines 115-145

1. Container binds to `0.0.0.0:50000` (listens on all container interfaces)
2. `HOST_IP` environment variable set to macOS LAN IP (e.g., 10.0.0.154)
3. WebRTC NAT 1:1 mapping advertises `HOST_IP` in ICE candidates
4. Docker's port mapping forwards `HOST_IP:50000` → container's `0.0.0.0:50000`
5. Browser connects to `10.0.0.154:50000`, Docker routes to container

**Why this works without STUN**:
- We explicitly tell WebRTC what IP to advertise (no discovery needed)
- Docker's built-in port mapping handles the NAT traversal
- From browser's perspective: connecting to LAN IP, same as web server
- From container's perspective: receives traffic on 0.0.0.0, no awareness of NAT

### Docker Compose UDP Mapping

**Required in `docker-compose.yml`**:
```yaml
ports:
  - "8080:8080"           # HTTP/WebSocket (TCP)
  - "50000:50000/udp"     # WebRTC media (UDP) - explicit /udp required!
```

**Critical**: The `/udp` suffix is required. Without it, Docker only maps TCP.

### macOS-Specific Gotchas

- **Docker Desktop networking**: Uses vpnkit or socket_vmnet, can have quirks
- **Firewall rules**: macOS firewall may block UDP; check System Preferences → Security
- **Wi-Fi vs Ethernet**: Different interfaces may have different firewall rules
- **VPN interference**: Active VPNs can complicate routing, prefer direct LAN

## Network Verification

**If the browser can successfully:**
- Load the web page at `http://HOST_IP:PORT/intercom`
- Establish WebSocket connection for signaling

**Then it will also be able to:**
- Reach WebRTC endpoint at `udp://HOST_IP:50000`
- Stream audio/video without STUN

The WebRTC connection is **no more complex** than the HTTP connection - it's just UDP instead of TCP to the same host.

## Troubleshooting

### WebRTC Connection Fails

**Symptom**: Browser shows "Connecting..." but never establishes media

**Diagnosis**:
1. Check browser console for errors
2. Look for ICE connection state messages
3. Check server logs for ICE candidates

**Common Causes**:

**Wrong HOST_IP configured**:
```bash
# Check what IP browser is trying to connect to
# Should match your server's LAN IP
echo $HOST_IP
ip addr show  # or: ifconfig
```

**UDP port blocked**:
```bash
# On server, check if port is listening
netstat -an | grep 50000
# or: ss -an | grep 50000

# Test UDP connectivity from client
nc -u HOST_IP 50000
```

**Firewall blocking UDP**:
```bash
# macOS: System Preferences → Security → Firewall → Firewall Options
# Allow incoming connections for birdbox-rs

# Linux:
sudo ufw allow 50000/udp
# or: sudo iptables -A INPUT -p udp --dport 50000 -j ACCEPT
```

**Docker UDP mapping missing**:
```yaml
# Verify docker-compose.yml has:
ports:
  - "50000:50000/udp"  # Must have /udp suffix!
```

### Audio/Video Stuttering

**Symptom**: Media plays but with frequent gaps or freezes

**Diagnosis**:
1. Check server CPU usage (`top` or `htop`)
2. Check network quality (packet loss, latency)
3. Review logs for transcoding errors

**Solutions**:

**Increase buffer sizes**:
```bash
# .env
AUDIO_FANOUT_BUFFER_SAMPLES=30
VIDEO_FANOUT_BUFFER_FRAMES=5
```

**Switch to TCP for reliability**:
```bash
# .env
RTSP_TRANSPORT_PROTOCOL=tcp
```

**Check network quality**:
```bash
# Ping DoorBird from server
ping -c 100 DOORBIRD_IP

# Check for packet loss
mtr DOORBIRD_IP
```

### Video Stream Hangs on Connection

**Symptom**: Video never starts, or starts then hangs within seconds

**Common Cause**: Using UDP transport over VPN or complex network

**Solution**: Switch to TCP transport
```bash
# .env
RTSP_TRANSPORT_PROTOCOL=tcp
```

**Why**: UDP-over-UDP (RTSP UDP through VPN UDP tunnel) causes packet loss amplification and timeout conflicts.

### ICE Gathering Timeout

**Symptom**: Browser console shows "ICE gathering timeout" or "Failed to gather candidates"

**Cause**: WebRTC can't bind to UDP port or determine local IP

**Solutions**:

**Check if port is already in use**:
```bash
lsof -i UDP:50000
# or: netstat -an | grep 50000
```

**Verify HOST_IP is correct**:
```bash
# Should be your server's LAN IP, not 127.0.0.1
echo $HOST_IP
```

**For Docker, ensure proper mapping**:
```bash
docker compose ps  # Verify container is running
docker compose logs birdbox  # Check startup logs
```

### Multiple ICE Candidates Advertised

**Symptom**: Browser receives multiple ICE candidates, connection unreliable

**Cause**: ICE control not working properly, multiple interfaces being advertised

**Solution**: Verify specific IP binding in logs
```
# Should see:
🌐 Bound WebRTC UDP socket to 10.0.0.154:50000

# Should NOT see multiple candidates like:
candidate: 127.0.0.1 ...
candidate: 192.168.1.1 ...
candidate: xxxx.local ...
```

**Fix**: Set explicit HOST_IP in `.env`

### NAT Hairpinning Issues

**Symptom**: External clients can connect but internal LAN clients cannot (or vice versa)

**Cause**: Router doesn't support hairpin NAT (routing external IP back to internal network)

**Solution**: Use split-brain DNS configuration
```bash
# .env
HOST_IP=YOUR_PUBLIC_IP        # For external clients
HOST_IP_LAN=YOUR_LAN_IP       # For internal clients
```

This advertises both IPs, allowing WebRTC to choose the right one.

## Security Considerations

### LAN-Only Deployment

**Recommended**: Keep Birdbox on LAN only

- No public IP exposure
- No attack surface
- DoorBird credentials stay on local network
- Simple firewall rules

### Public Internet Exposure

**If you must expose publicly**:

1. **Use HTTPS with reverse proxy** (Caddy, nginx):
   ```yaml
   # Caddy example included in project
   caddy:
     image: caddy:2
     ports:
       - "443:443"
       - "443:443/udp"
   ```

2. **Implement authentication**: Add auth middleware to Axum

3. **Use strong DoorBird passwords**: Prevent credential stuffing

4. **Monitor logs**: Watch for unusual connection patterns

5. **Consider VPN instead**: Tailscale, WireGuard provide secure access without public exposure

### Firewall Rules

**Minimal ruleset for LAN-only**:
```bash
# Allow LAN access only
iptables -A INPUT -p tcp --dport 3000 -s 192.168.0.0/16 -j ACCEPT
iptables -A INPUT -p udp --dport 50000 -s 192.168.0.0/16 -j ACCEPT
```

## Advanced Scenarios

### Running Through Tailscale/WireGuard VPN

**Configuration**:
```bash
# .env
RTSP_TRANSPORT_PROTOCOL=tcp     # Required for VPN
AUDIO_FANOUT_BUFFER_SAMPLES=30  # Increase for VPN latency
VIDEO_FANOUT_BUFFER_FRAMES=5    # Increase for VPN latency
HOST_IP=YOUR_TAILSCALE_IP       # Use Tailscale IP
```

**Why TCP**: Avoids UDP-over-UDP issues that cause packet loss and timeouts

### Multiple Network Interfaces

If server has multiple interfaces (WiFi + Ethernet, multiple VLANs):

```bash
# Bind to specific interface IP
HOST_IP=192.168.1.154  # Use IP of interface you want

# Or use split-brain for multiple interfaces
HOST_IP=10.0.0.154          # Primary interface
HOST_IP_LAN=192.168.1.154   # Secondary interface
```

### Kubernetes/Container Orchestration

**Not currently supported** due to:
- Fixed UDP port (not cloud-native friendly)
- Single-instance design
- No external STUN/TURN support

**Possible with modifications**:
- Dynamic UDP port allocation
- Service mesh integration
- External TURN server for NAT traversal

## Monitoring & Debugging

### Useful Log Messages

**Connection established**:
```
🌐 Using HOST_IP from environment: 192.168.1.154
🌐 Bound WebRTC UDP socket to 192.168.1.154:50000
New WebSocket connection: session <uuid>
```

**ICE negotiation**:
```
received client offer, creating answer...
sending answer to client
received client ICE candidate: <candidate>
```

**Media streaming**:
```
Audio subscriber added (total: 1)
Video subscriber added (total: 1)
Connecting to DoorBird audio stream...
Successfully connected to DoorBird audio stream
```

### Network Debugging Tools

**Test UDP connectivity**:
```bash
# From client to server
nc -u SERVER_IP 50000

# From server, listen on port
nc -ul 50000
```

**Check route to DoorBird**:
```bash
traceroute DOORBIRD_IP
mtr DOORBIRD_IP
```

**Monitor WebRTC in browser**:
1. Open browser console
2. Go to `chrome://webrtc-internals` (Chrome)
3. Or `about:webrtc` (Firefox)
4. Inspect ICE candidates, connection state, media statistics

## Summary

### Quick Reference

**LAN Deployment (Simple)**:
```bash
HOST_IP=192.168.1.154
UDP_PORT=50000
RTSP_TRANSPORT_PROTOCOL=udp
```

**Docker Deployment**:
```bash
HOST_IP=192.168.1.154       # Your host's LAN IP
UDP_PORT=50000
# docker-compose.yml must have: "50000:50000/udp"
```

**VPN Deployment**:
```bash
HOST_IP=100.64.x.x          # Your VPN IP
UDP_PORT=50000
RTSP_TRANSPORT_PROTOCOL=tcp # Required!
```

**Dual Network (LAN + Internet)**:
```bash
HOST_IP=203.0.113.50        # Public IP
HOST_IP_LAN=192.168.1.154   # LAN IP
UDP_PORT=50000
# Configure router port forwarding
```

### Key Takeaways

1. **WebRTC is client-server**, not peer-to-peer
2. **No STUN/TURN needed** for LAN deployments
3. **HOST_IP must match** server's actual IP (not 127.0.0.1)
4. **UDP port mapping is critical** in Docker
5. **Use TCP transport** for VPN/complex networks
6. **Split-brain DNS** solves NAT hairpinning issues
7. **If HTTP works, WebRTC should work** (same IP, different protocol)

The networking is simpler than it appears - once you understand it's client-server on the same LAN, most complexity disappears.

