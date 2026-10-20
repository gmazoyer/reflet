# Changelog

All notable changes to Reflet will be documented in this file.

## 0.2.0 - 2026-10-20

Reflet can keep periodic snapshots of the RIB of a peer. BGP sessions no
longer reset when a message arrives in two TCP segments. Reflet stops
cleanly on SIGTERM and sends a Cease NOTIFICATION to its peers.

### Changes that can need an action after the upgrade
- The RPKI fetch uses the root certificates of the operating system, not a
  copy of the Mozilla root certificates in the binary. A host with no system
  root certificates cannot fetch VRPs over HTTPS. The Docker image has these
  certificates.
- The HTTP server answers 406 Not Acceptable to a client that refuses all
  encodings, for example with `Accept-Encoding: identity;q=0` and no other
  encoding. Before, it sent the response without compression.

### RIB snapshots
- Reflet can save a copy of the RIB of a peer at a fixed interval. You can
  then view the routes of the peer at the time of each snapshot, in the web
  UI and through the API.
- To enable snapshots, set `snapshot_interval` (60 seconds or more) on each
  peer and add a `[snapshots]` section with `data_dir`. Use `max_snapshots`
  and `max_age_hours` to limit the number of snapshots that Reflet keeps.
  See `docs/configuration.md`.
- The routes page of a peer shows a "Live / History" switch when snapshots
  exist.
- New API endpoints and fields:
  - `GET /api/v1/peers/{id}/snapshots` lists the snapshots of a peer.
  - `GET /api/v1/peers/{id}/snapshots/{timestamp}/routes/ipv4` and
    `.../routes/ipv6` return the routes of one snapshot, with the same
    search filters as the live routes.
  - `GET /api/v1/summary` has a new field, `snapshots_enabled`.
- Each request on a snapshot reads and decompresses a full RIB from disk.
  Do not enable snapshots on a public instance.

### BGP sessions
- Sessions no longer reset when a BGP message arrives in two TCP segments.
  Before, Reflet lost the first part of the message when the keepalive
  timer expired between the two segments. It then closed the session
  without a NOTIFICATION, and the peer saw a TCP reset. On a session with a
  full table, this happened several times a day.
- Reflet sends a NOTIFICATION with error code 1 (Message Header Error)
  before it closes a session because of a bad message header. RFC 4271
  section 6.1 requires this NOTIFICATION. Before, the peer saw only a TCP
  reset. The subcodes are:
  - 1 (Connection Not Synchronized) for a bad marker.
  - 2 (Bad Message Length) for a length outside 19 to 4096. The data field
    contains the bad length.
  - 3 (Bad Message Type) for an unknown message type. The data field
    contains the bad type, followed by one zero octet.

### Shutdown
- Reflet stops cleanly on SIGTERM, which `docker stop` and `systemctl stop`
  send. Before, Reflet stopped cleanly only on SIGINT (Ctrl+C). After a
  SIGTERM, it did not save the RIBs for graceful restart and did not write
  the buffered events to the event log file:
  - Outside a container, SIGTERM stopped Reflet at once.
  - In a container, `docker stop` waited for its stop timeout and then
    killed Reflet.
- On shutdown, each established session sends a Cease NOTIFICATION with
  subcode 2 (Administrative Shutdown) to its peer. Before, the TCP
  connections closed without a NOTIFICATION. Reflet waits at most 2 seconds
  for the sessions to close, so the event log file also contains the
  session down events.

### Web UI and API
- The web UI and the API list the peers in the order of the configuration
  file. Before, the order did not follow the configuration file.
- The frontend Docker image trusts the `X-Forwarded-For` header from
  private networks (10.0.0.0/8, 172.16.0.0/12 and 192.168.0.0/16). Behind a
  reverse proxy, the "Your IP" field and `/api/v1/whoami` show the IP
  address of the client, not the address of the proxy. The reverse proxy
  must send the header. See `docs/deployment.md`.
- The `/docs` page uses a newer Swagger UI. In the OpenAPI document, three
  `oneOf` lists of nullable fields put `"null"` after the schema reference.

### Build and dependencies
- The Docker image builds with Rust 1.99, the same version as CI.
- The configuration file can use TOML 1.1. All TOML 1.0 files stay valid.
- Upgraded crates:
  - reqwest 0.13
  - tower-http 0.7
  - toml 1
  - prefix-trie 0.10
  - utoipa 6, utoipa-axum 0.3 and utoipa-swagger-ui 10
- The other crates use their latest compatible versions.

All changes: https://github.com/gmazoyer/reflet/compare/v0.1.0...v0.2.0

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
