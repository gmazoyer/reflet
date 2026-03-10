# Changelog

All notable changes to Reflet will be documented in this file.

## 0.1.0 - 2026-03-10

First public release.

Reflet is a BGP Looking Glass built with Rust and React. It peers with your
routers using native BGP, stores every route in memory, and serves them through
a fast web interface and REST API.

### BGP speaker
- Full RFC 4271 session state machine with per-peer Tokio tasks
- Add-Path (RFC 7911) to receive and display multiple paths per prefix
- Route Refresh (RFC 2918 + RFC 7313) for basic and Enhanced Route Refresh
  with stale-route sweep
- Graceful Restart (RFC 4724) for RIB persistence to disk, configurable
  restart timer, stale route cleanup
- IPv4 and IPv6 unicast address families
- Configurable hold time, per-peer address family negotiation

### Route lookups
- Exact match, longest-prefix-match, and more-specifics across all peers
- Filters: ASN, AS path regex, community (with wildcards), origin, MED,
  local-pref (with comparison operators)
- Interactive AS path DAG visualization in the web UI

### RPKI validation (RFC 6811)
- Fetch Validated ROA Payloads from a Routinator-compatible validator
- Routes annotated with validation status (Valid / Invalid / Not Found) at
  query time
- Background refresh on a configurable interval

### Web frontend
- React, Vite, Tailwind CSS, TanStack Query / Table / Virtual
- Virtual scrolling — handles 100k+ route tables smoothly
- Real-time route updates via Server-Sent Events
- Dark mode support
- Community annotations with human-readable names (NLNOG-format definition
  files)
- ASN enrichment with names and countries (IPInfo Lite datasets)

### API & operations
- REST API under `/api/v1/` with pagination and filtering
- RFC 8522 Well-Known Looking Glass endpoints at
  `/.well-known/looking-glass/v1/`
- Interactive Swagger UI at `/docs`, OpenAPI schema at
  `/api-docs/openapi.json`
- Prometheus metrics at `/metrics`
- SSE event stream at `/api/v1/events/stream` with in-memory buffer and
  optional disk persistence
- TOML configuration with a single file — see `config.toml.example`

### Deployment
- Docker images and `docker-compose.yml`
- GitHub Actions CI for code quality and image builds
