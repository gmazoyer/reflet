# Reflet

A BGP Looking Glass built with Rust and React. Reflet connects to your BGP routers, collects routing tables, and provides a fast web interface for browsing, searching, and analyzing routes.

*Reflet* is French for "reflection" — the tool reflects your network's routing state for observation.

## Features

- **Full BGP support**: RFC 4271 state machine, Add-Path (RFC 7911), Route Refresh (RFC 2918 + RFC 7313), Graceful Restart (RFC 4724)
- **RPKI validation**: Per-route ROA validity (valid/invalid/not-found) via Routinator or any compatible RPKI validator
- **Rich route filtering**: Search by prefix, ASN, AS path, community (standard/large/extended, with wildcards), origin, MED, and local-pref with comparison operators
- **Prefix lookup**: Exact match, longest-prefix-match, and subnet (more-specific) lookups across all peers
- **AS path graph**: Interactive DAG visualization of AS paths from lookup results
- **Real-time updates**: Server-Sent Events push route changes to the browser instantly
- **Prometheus metrics**: Monitor peer status, prefix counts, and session uptime
- **Community annotations**: Human-readable community names from definition files
- **ASN enrichment**: AS names and countries from [IPInfo Lite](https://ipinfo.io/lite) datasets
- **RFC 8522 API**: Standard looking glass REST interface
- **OpenAPI docs**: Swagger UI at `/docs`

## Quick Start

### Prerequisites

- [Rust](https://rustup.rs) (stable)
- [Node.js](https://nodejs.org) 18+ and npm

### Build

```bash
# Backend
cargo build

# Frontend
cd frontend && npm install && npm run build
```

### Configure

```bash
cp config.toml.example config.toml
# Edit config.toml with your BGP local ASN, router ID, and peers
```

### Run

```bash
# Start the backend
cargo run --bin reflet -- --config config.toml

# Start the frontend dev server (in another terminal)
cd frontend && npm run dev
```

Open http://localhost:5173.

### Docker

```bash
docker compose up -d
```

Frontend on port 80, backend API on port 8080. See [docs/DEPLOYMENT.md](docs/DEPLOYMENT.md) for production deployment options.

## CLI Usage

```
reflet [OPTIONS]
```

| Flag                  | Description                                          |
|-----------------------|------------------------------------------------------|
| `-c, --config <FILE>` | Path to configuration file (default: `config.toml`) |
| `--check`             | Validate the configuration file and exit             |
| `--version`           | Print version and exit                               |
| `-h, --help`          | Print help                                           |

```bash
# Validate your config without starting the server
reflet --check --config config.toml

# Print version
reflet --version
```

## Configuration

```toml
# Optional: human-readable community names from definition files.
# A good starting point is the NLNOG community definitions:
# https://github.com/NLNOG/lg.ring.nlnog.net/tree/main/communities
# communities_dir = "/path/to/communities"

# Optional: ASN names and countries from the IPInfo free Lite database.
# Download the ASN CSV from: https://ipinfo.io/products/free-ip-database
# Supports both plain CSV and gzip-compressed CSV (.csv.gz).
# ipinfo_dataset_file = "/path/to/ipinfo_lite.csv.gz"

[server]
listen = "0.0.0.0:8080"
bgp_listen = "0.0.0.0:179"
# title = "Reflet"
# hide_peer_addresses = true

[bgp]
local_asn = 65000
router_id = "10.0.0.1"
hold_time = 90

# [bgp.graceful_restart]
# enabled = true
# restart_time = 120
# data_dir = "/var/lib/reflet/rib"

[[peers]]
address = "10.0.0.2"
remote_asn = 65000
name = "Router A"
description = "Primary border router in Amsterdam DC1"
location = "DC1, Amsterdam"
families = ["ipv4-unicast", "ipv6-unicast"]

# [rpki]
# enabled = true
# url = "https://rpki.example.com"
# refresh_interval = 300

# [event_log]
# enabled = true
# buffer_size = 10000
# file = "/var/log/reflet/events.jsonl"

[logging]
level = "info"
format = "pretty"
```

See [config.toml.example](config.toml.example) for the full reference.

## API

| Method | Endpoint                             | Description                                     |
|--------|--------------------------------------|-------------------------------------------------|
| GET    | `/api/v1/summary`                    | Instance summary (ASN, router ID, peer/prefix counts) |
| GET    | `/api/v1/peers`                      | List all peers                                  |
| GET    | `/api/v1/peers/{id}`                 | Peer details                                    |
| POST   | `/api/v1/peers/{id}/refresh`         | Request route refresh                           |
| GET    | `/api/v1/peers/{id}/routes/ipv4`     | IPv4 routes (paginated, filterable)             |
| GET    | `/api/v1/peers/{id}/routes/ipv6`     | IPv6 routes (paginated, filterable)             |
| GET    | `/api/v1/lookup?prefix=...&type=...` | Prefix lookup (exact, longest-match, subnets)   |
| GET    | `/api/v1/events`                     | Recent route change events                      |
| GET    | `/api/v1/events/stream`              | SSE stream for real-time updates                |
| GET    | `/metrics`                           | Prometheus metrics                              |
| GET    | `/docs`                              | Swagger UI                                      |

RFC 8522 endpoints are available under `/.well-known/looking-glass/v1/`.

## Testing

```bash
# All backend tests
cargo test --workspace

# Clippy (zero warnings required)
cargo clippy --workspace

# Frontend
cd frontend && npm test && npm run lint
```

## Documentation

- [Deployment](docs/DEPLOYMENT.md) - Docker setup, monitoring, SSE proxy config, graceful shutdown

## License

Apache License 2.0 — see [LICENSE](LICENSE).
