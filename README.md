# Reflet

A BGP Looking Glass built with Rust and React. Reflet connects to your BGP routers, collects routing tables, and provides a fast web interface for browsing, searching, and analyzing routes.

*Reflet* is French for "reflection" — the tool reflects your network's routing state for observation.

## What it does

Reflet speaks BGP natively. It peers with your routers using a full RFC 4271 state machine with support for Add-Path (RFC 7911), Route Refresh (RFC 2918 + RFC 7313), and Graceful Restart (RFC 4724), then stores every route it receives in memory for fast lookups.

Routes can be searched by prefix (exact, longest-prefix-match, or more-specifics across all peers), filtered by ASN, AS path regex, community values with wildcards, origin, MED, or local-pref with comparison operators. Lookup results include an interactive AS path DAG visualization. If you run an RPKI validator like Routinator, Reflet annotates every route with its ROA validation status (valid, invalid, or not-found).

The web frontend streams route changes in real time over Server-Sent Events, so the view stays current without polling. Communities are shown with human-readable names if you provide definition files, and AS numbers are enriched with names and countries from [IPInfo Lite](https://ipinfo.io/lite) datasets.

Optionally, Reflet can take periodic snapshots of each peer's RIB and let you browse historical routing state through the same interface. Note that snapshot browsing is resource-intensive and should not be enabled on public instances (see [configuration docs](docs/configuration.md) for details).

For automation, the backend exposes a REST API following the RFC 8522 Looking Glass standard, with interactive Swagger UI at `/docs` and Prometheus metrics at `/metrics`.

## Quick Start

You'll need [Rust](https://rustup.rs) (stable) and [Node.js](https://nodejs.org) 18+ with npm.

```bash
# Build
cargo build
cd frontend && npm install && npm run build && cd ..

# Configure
cp config.toml.example config.toml
# Edit config.toml with your BGP local ASN, router ID, and peers

# Run
cargo run --bin reflet -- --config config.toml
```

For development, start the frontend dev server in a second terminal with `cd frontend && npm run dev`, then open http://localhost:5173.

### Docker

```bash
docker compose up -d
```

Frontend on port 80, backend API on port 8080. See [docs/deployment.md](docs/deployment.md) for production deployment options.

## Configuration

Reflet is configured through a single TOML file. Copy [config.toml.example](config.toml.example) as a starting point. Here's a minimal configuration:

```toml
[server]
listen = "0.0.0.0:8080"
bgp_listen = "0.0.0.0:179"

[bgp]
local_asn = 65000
router_id = "10.0.0.1"
hold_time = 90

[[peers]]
address = "10.0.0.2"
remote_asn = 65000
name = "Router A"
description = "Primary border router in Amsterdam DC1"
location = "DC1, Amsterdam"
families = ["ipv4-unicast", "ipv6-unicast"]

[logging]
level = "info"
format = "pretty"
```

Optional sections include `[bgp.graceful_restart]` for RIB persistence across restarts, `[rpki]` for RPKI validation via a Routinator endpoint, and `[event_log]` for persisting route change events to disk. Community definition files and IPInfo ASN datasets can be loaded from paths set at the top level. See the example config for the full reference. For a complete reference of every option, see [docs/configuration.md](docs/configuration.md).

## CLI

```
reflet [OPTIONS]
```

| Flag                  | Description                                          |
|-----------------------|------------------------------------------------------|
| `-c, --config <FILE>` | Path to configuration file (default: `config.toml`) |
| `--check`             | Validate the configuration file and exit             |
| `--version`           | Print version and exit                               |
| `-h, --help`          | Print help                                           |

## API

The REST API lives under `/api/v1/`. Peers and their routes are accessed through `/api/v1/peers/{id}/routes/ipv4` and `.../ipv6`, with pagination and filtering support. Cross-peer prefix lookups (exact, longest-match, or more-specifics) are available at `/api/v1/lookup`. A summary of the instance, including ASN, router ID, and prefix counts, is served from `/api/v1/summary`.

Real-time route changes are streamed over SSE at `/api/v1/events/stream`, with a recent event buffer at `/api/v1/events`. RFC 8522 endpoints are available under `/.well-known/looking-glass/v1/`.

Interactive API documentation is served at `/docs` and Prometheus metrics at `/metrics`.

## Testing

```bash
cargo test --workspace          # All backend tests
cargo clippy --workspace        # Lint (zero warnings required)
cd frontend && npm test         # Frontend tests
```

## Documentation

- [Configuration](docs/configuration.md) — full reference for every config option
- [Deployment](docs/deployment.md) — Docker setup, monitoring, SSE proxy configuration, and graceful shutdown

## License

Apache License 2.0 — see [LICENSE](LICENSE).
