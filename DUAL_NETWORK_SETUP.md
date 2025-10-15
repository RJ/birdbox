# Dual-Network WebRTC Configuration

## Overview

This system now supports **split-brain DNS / dual-network topology** for WebRTC without requiring hairpin NAT on your router. This allows:

- **Internal LAN clients** → Connect directly via LAN IP (fast, no router traversal)
- **External clients** → Connect via public IP (NAT forwarding)
- **No STUN servers needed** (client-server architecture)

## How It Works

### Architecture

```
┌─────────────────────────────────────────────────────────────┐
│  Split-Brain DNS                                             │
│  • Inside LAN:  domain → 10.x.x.x                           │
│  • Outside LAN: domain → public IP                          │
└─────────────────────────────────────────────────────────────┘

┌─────────────────────────────────────────────────────────────┐
│  WebRTC Server (birdbox)                                     │
│  • Binds to: 0.0.0.0:50000 (all interfaces)                 │
│  • Advertises TWO ICE candidates:                            │
│    1. 10.x.x.x:50000     (for LAN clients)                  │
│    2. public-ip:50000     (for external clients)            │
└─────────────────────────────────────────────────────────────┘

┌───────────────────────┐           ┌────────────────────────┐
│  LAN Client           │           │  External Client       │
│  • Receives both IPs  │           │  • Receives both IPs   │
│  • Tries 10.x.x.x ✅  │           │  • Tries 10.x.x.x ❌   │
│  • Direct connection  │           │  • Tries public-ip ✅  │
└───────────────────────┘           │  • NAT forwards to     │
                                    │    10.x.x.x            │
                                    └────────────────────────┘
```

### Key Changes

1. **No STUN on client** - Client uses only server's advertised candidates
2. **Dual-IP advertisement** - Server advertises both LAN and public IPs
3. **ICE automatically selects** - WebRTC's ICE protocol tries all candidates and uses the one that works

## Configuration

### Step 1: Update your `.env` file

```bash
# Your public IP (or domain that resolves to public IP externally)
BIRDBOX_HOST_IP=203.0.113.50

# Your server's LAN IP (the machine running Docker/birdbox)
BIRDBOX_HOST_IP_LAN=192.168.1.50

# UDP port (must match your router's port forwarding)
BIRDBOX_UDP_PORT=50000
```

### Step 2: Ensure router port forwarding

Your router must forward:
- **UDP port 50000** → `192.168.1.50:50000` (for WebRTC media)
- **TCP port 443** (or 8443) → `192.168.1.50:443` (for HTTPS/WSS)

### Step 3: Rebuild and restart

```bash
docker compose down
docker compose build
docker compose up -d
```

## Testing

### Expected Log Output

When the server starts, you should see:

```
🌐 Using BIRDBOX_HOST_IP from environment: 203.0.113.50
🌐 Bound WebRTC UDP socket to 0.0.0.0:50000 (shared across all sessions)
🌐 Dual-network mode: advertising 2 ICE candidate(s): ["192.168.1.50", "203.0.113.50"]
🌐 Disabled mDNS candidates (using specific IPs only)
🌐 Setting NAT 1:1 mapping to advertise 2 IP(s) as ICE candidates
```

### Testing from LAN

1. Connect from a device on your LAN
2. Open browser console (F12)
3. Look for WebRTC logs showing:
   - `remote ICE candidate: { candidate: "... 192.168.1.50:50000 ..." }`
   - `remote ICE candidate: { candidate: "... 203.0.113.50:50000 ..." }`
   - `ICE connection state changed: connected`
4. Connection should establish quickly using the LAN IP

### Testing from External Network

1. Connect from a device outside your LAN (mobile data, different network, etc.)
2. Open browser console (F12)
3. Look for WebRTC logs showing:
   - Both candidates received
   - LAN IP candidate will fail (can't route to private IP)
   - Public IP candidate will succeed
   - `ICE connection state changed: connected`

### Debugging ICE Candidates

In the browser console, you'll see detailed ICE candidate information:

```javascript
[webrtc] remote ICE candidate: {
  candidate: "candidate:... 192.168.1.50 50000 typ host",
  sdpMid: "0",
  sdpMLineIndex: 0,
  type: "host",
  protocol: "udp",
  address: "192.168.1.50",
  port: 50000
}
```

**LAN clients** should successfully connect using `192.168.1.50`  
**External clients** should successfully connect using your public IP

## Single-IP Mode (Backward Compatible)

If you only set `BIRDBOX_HOST_IP` (without `BIRDBOX_HOST_IP_LAN`), the system works as before:

```bash
BIRDBOX_HOST_IP=192.168.1.50
# BIRDBOX_HOST_IP_LAN not set
```

Server will advertise only one IP:
```
🌐 Single-network mode: advertising ICE candidate: 192.168.1.50
```

## Troubleshooting

### "ICE connection state: failed"

**LAN clients:**
- Check that `BIRDBOX_HOST_IP_LAN` matches your server's actual LAN IP
- Verify no firewall blocking UDP 50000 on LAN

**External clients:**
- Check that `BIRDBOX_HOST_IP` is your correct public IP
- Verify router port forwarding for UDP 50000
- Check router logs for blocked packets

### "Only one IP advertised" (when you expected two)

- Ensure both `BIRDBOX_HOST_IP` and `BIRDBOX_HOST_IP_LAN` are set in `.env`
- Ensure they're different IPs
- Check server startup logs for configuration messages

### "Could not bind to IP"

The server defaults to binding to `0.0.0.0:50000` (all interfaces) which works in all scenarios. If you need to restrict binding to a specific interface, use the optional `BIRDBOX_BIND_IP` environment variable.

```bash
BIRDBOX_BIND_IP=192.168.1.50  # Optional: bind to specific interface
```

## Advanced: Docker Host Networking

If using Docker's host networking mode:

```yaml
birdbox:
  network_mode: "host"
```

You can optionally use `BIRDBOX_BIND_IP` to bind to a specific interface. By default, the server binds to `0.0.0.0` which works in all cases.

## Technical Details

### Why No STUN?

**STUN servers** solve the peer-to-peer problem where both peers are behind NAT and need to discover their public IPs.

In **client-server architecture**:
- Server has known address (via configuration)
- Server advertises its own candidates
- Client just needs to connect TO the server
- Client's IP doesn't matter (server responds to wherever client connects from)

### NAT 1:1 Mapping

The WebRTC library's `set_nat_1to1_ips()` tells the server:
"You're bound to one interface, but advertise these IP(s) as candidates instead"

This is perfect for Docker (bound to container IP, advertise host IP) and dual-network setups (advertise multiple IPs).

### ICE Candidate Selection

WebRTC's Interactive Connectivity Establishment (ICE) protocol:
1. Gathers all candidates (both sides)
2. Exchanges candidates via signaling (WebSocket)
3. Performs connectivity checks on all candidate pairs
4. Selects the working pair with the best priority

In dual-IP mode:
- LAN clients: Both IPs reachable, LAN IP has lower latency → uses LAN IP
- External clients: Only public IP reachable → uses public IP

