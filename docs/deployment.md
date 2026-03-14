# Deployment Guide

This guide covers deploying Reflet using Docker. Two deployment models are supported:

1. **Single machine** — both backend and frontend on the same host using Docker Compose
2. **Split deployment** — backend and frontend on separate machines

## Prerequisites

- Docker Engine 20.10+ and Docker Compose v2
- A `config.toml` file (copy from `config.toml.example` and adjust)
- Port access: **80** (frontend), **8080** (backend HTTP API), **179** (BGP)

## Configuration

Before deploying, create your configuration file:

```bash
cp config.toml.example config.toml
```

Edit `config.toml` to set your BGP local ASN, router ID, and peer list. See `config.toml.example` for the full schema.

### Optional Features

**Graceful Restart (RFC 4724)**: Keeps routes across Reflet restarts. Requires a persistent data directory:

```toml
[bgp.graceful_restart]
enabled = true
restart_time = 120
data_dir = "/var/lib/reflet/rib"
```

When using Docker, mount this directory as a volume to persist across container restarts.

**Event log**: Records route announcements, withdrawals, and session changes. Can optionally persist to disk as JSONL:

```toml
[event_log]
enabled = true
buffer_size = 10000
file = "/var/log/reflet/events.jsonl"
```

**Community annotations**: Provide human-readable names for BGP communities:

```toml
communities_dir = "/path/to/communities"
```

**ASN information**: Enrich AS path display with network names and countries:

```toml
ipinfo_dataset_file = "/path/to/ipinfo_lite.csv.gz"
```

**Privacy mode**: Hide peer IP addresses and router IDs from API responses:

```toml
[server]
hide_peer_addresses = true
```

## Single-Machine Deployment (Docker Compose)

This is the simplest setup — both containers run on the same host.

```bash
# Build and start both services
docker compose up -d

# View logs
docker compose logs -f

# Stop
docker compose down
```

The frontend is available on **port 80** and automatically proxies API requests to the backend container.

The backend API is also directly accessible on **port 8080** (useful for debugging or direct API access).

### Volumes for Persistent Data

If using Graceful Restart or event log file output, add volumes to your `docker-compose.yml`:

```yaml
services:
  backend:
    volumes:
      - ./config.toml:/etc/reflet/config.toml:ro
      - rib-data:/var/lib/reflet/rib          # Graceful Restart
      - event-logs:/var/log/reflet            # Event log file

volumes:
  rib-data:
  event-logs:
```

## Split Deployment

In a split deployment, the backend runs on one machine (Machine A) and the frontend on another (Machine B). The frontend's nginx reverse-proxies API requests to the backend.

### Machine A — Backend

Build and run the backend image:

```bash
# Build the image
docker build -t reflet-backend .

# Run (mount your config file)
docker run -d \
  --name reflet-backend \
  --restart unless-stopped \
  -v /path/to/config.toml:/etc/reflet/config.toml:ro \
  -p 8080:8080 \
  -p 179:179 \
  reflet-backend
```

### Machine B — Frontend

Build and run the frontend image, pointing `BACKEND_URL` at Machine A:

```bash
# Build the image
docker build -t reflet-frontend frontend/

# Run (replace MACHINE_A_IP with the backend's address)
docker run -d \
  --name reflet-frontend \
  --restart unless-stopped \
  -e BACKEND_URL=http://MACHINE_A_IP:8080 \
  -p 80:80 \
  reflet-frontend
```

The `BACKEND_URL` environment variable tells nginx where to proxy `/api/`, `/.well-known/`, `/metrics`, and `/docs` requests. It defaults to `http://backend:8080` (the Docker Compose service name).

## Network and Firewall Notes

| Port | Protocol | Service          | Direction                                     |
|------|----------|------------------|-----------------------------------------------|
| 80   | TCP      | Frontend (nginx) | Inbound from users                            |
| 8080 | TCP      | Backend HTTP API | Inbound from frontend (and optionally users)  |
| 179  | TCP      | BGP              | Inbound from BGP peers                        |

For a split deployment:
- Machine A (backend) must accept connections on **8080** from Machine B and on **179** from BGP peers
- Machine B (frontend) must accept connections on **80** from users
- If the backend API should not be publicly accessible, restrict port 8080 to only Machine B's IP

### Client IP Behind a Reverse Proxy

If the frontend container sits behind an external reverse proxy (Traefik, HAProxy, Caddy, etc.), the "Your IP" field and the `/api/v1/whoami` endpoint will show the proxy's internal Docker address instead of the real client IP. This happens because nginx sees the proxy container as `$remote_addr`.

The frontend image already includes a `realip` configuration in nginx that trusts `X-Forwarded-For` headers from private networks (`10.0.0.0/8`, `172.16.0.0/12`, `192.168.0.0/16`), so no change is needed on the Reflet side.

You must however ensure the reverse proxy forwards the header. For Traefik, add to your static configuration:

```
--entrypoints.websecure.forwardedHeaders.insecure=true
```

Or restrict to trusted proxy IPs with `forwardedHeaders.trustedIPs` instead.

### SSE Proxy Considerations

The backend serves a Server-Sent Events (SSE) endpoint at `/api/v1/events/stream` for real-time updates. If you place a reverse proxy (nginx, Cloudflare, etc.) in front of the backend:

- Disable response buffering for the SSE endpoint (nginx: `proxy_buffering off;`)
- Set `X-Accel-Buffering: no` header
- Ensure timeouts are long enough for the persistent connection (nginx: `proxy_read_timeout 86400;`)
- The Docker frontend image's nginx config handles this automatically

## Monitoring

### Prometheus Metrics

The backend exposes Prometheus metrics at `GET /metrics`. Available metrics:

| Metric                                     | Description                      |
|--------------------------------------------|----------------------------------|
| `reflet_info`                              | Instance info (ASN, router ID)   |
| `reflet_peers_total`                       | Total configured peers           |
| `reflet_peers_established`                 | Peers in Established state       |
| `reflet_prefixes_total{af="ipv4\|ipv6"}`   | Total prefixes by address family |
| `reflet_peer_up{peer, asn, name}`          | Per-peer established status      |
| `reflet_peer_prefixes{peer, af}`           | Per-peer prefix count            |
| `reflet_peer_uptime_seconds{peer}`         | Per-peer session uptime          |

Example Prometheus scrape config:

```yaml
scrape_configs:
  - job_name: reflet
    static_configs:
      - targets: ['localhost:8080']
```

### Health Check

`GET /api/v1/health` returns `{"status": "ok"}` — suitable for Docker health checks or load balancer probes.

## Rebuilding After Updates

```bash
# Single-machine
docker compose build
docker compose up -d

# Split deployment — rebuild each image on its respective machine
docker build -t reflet-backend .
docker build -t reflet-frontend frontend/
```

Then restart the containers to pick up the new images.

## Graceful Shutdown

Reflet handles `SIGINT` (Ctrl+C) / `SIGTERM` (Docker stop) gracefully:

1. SSE streams are signaled to terminate
2. Axum drains in-flight HTTP connections
3. Event log is flushed to disk
4. If Graceful Restart is enabled, RIBs are persisted to the configured `data_dir`

Docker's default stop timeout (10 seconds) is typically sufficient. Increase it if you have very large RIBs that take longer to persist:

```yaml
services:
  backend:
    stop_grace_period: 30s
```
