# Configuration Reference

Reflet is configured through a single TOML file, passed via the `--config` flag (defaults to `config.toml`). Copy [config.toml.example](../config.toml.example) as a starting point and adjust the values below to match your network.

## Top-level options

These options sit at the root of the TOML file, outside any section header. Both are optional and enable enrichment features.

| Option | Type | Default | Description |
|--------|------|---------|-------------|
| `communities_dir` | string | *(none)* | Path to a directory of community definition files in [NLNOG format](https://github.com/NLNOG/lg.ring.nlnog.net/tree/main/communities). When set, communities on routes are displayed with human-readable names. |
| `ipinfo_dataset_file` | string | *(none)* | Path to an [IPInfo Lite](https://ipinfo.io/lite) ASN dataset (`.csv` or `.csv.gz`). When set, AS numbers are enriched with names and countries. |

```toml
communities_dir = "/etc/reflet/communities"
ipinfo_dataset_file = "/etc/reflet/asn-lite.csv.gz"
```

## `[server]`

Controls the HTTP API and BGP listening sockets, the web UI title, and privacy settings.

| Option | Type | Default | Description |
|--------|------|---------|-------------|
| `listen` | socket address | `0.0.0.0:8080` | Address and port for the REST API and web UI. |
| `bgp_listen` | socket address | `0.0.0.0:179` | Address and port for incoming BGP connections. Port 179 requires root or `CAP_NET_BIND_SERVICE`. |
| `title` | string | `"Reflet"` | Display title shown in the web UI header. |
| `hide_peer_addresses` | boolean | `false` | When `true`, peer IP addresses, router IDs, and next-hop addresses are hidden from API responses. |
| `disable_route_refresh` | boolean | `false` | When `true`, the route refresh API returns 403 and the UI button is hidden. Useful for public instances to prevent abuse. |

```toml
[server]
listen = "0.0.0.0:8080"
bgp_listen = "0.0.0.0:179"
title = "Reflet"
hide_peer_addresses = false
disable_route_refresh = false
```

## `[bgp]`

BGP speaker parameters. These apply to all peer sessions.

| Option | Type | Default | Description |
|--------|------|---------|-------------|
| `local_asn` | integer (u32) | `65000` | The local autonomous system number used in BGP OPEN messages. |
| `router_id` | IPv4 address | `10.0.0.1` | BGP router identifier, sent in OPEN messages. |
| `hold_time` | integer (u16) | `90` | Hold time in seconds proposed during session negotiation. Must be `0` (disabled) or `>= 3`. |

```toml
[bgp]
local_asn = 65000
router_id = "10.0.0.1"
hold_time = 90
```

## `[bgp.graceful_restart]`

Graceful Restart support (RFC 4724). When enabled, RIBs are persisted to disk on shutdown and reloaded on startup so the looking glass retains routes across restarts.

| Option | Type | Default | Description |
|--------|------|---------|-------------|
| `enabled` | boolean | `false` | Enable Graceful Restart capability advertisement and RIB persistence. |
| `restart_time` | integer (u16) | `120` | Restart time in seconds advertised to peers. Must be 0–4095 (12-bit field). |
| `data_dir` | string | *(none)* | Directory for RIB persistence files. **Required** when `enabled = true`. |

```toml
[bgp.graceful_restart]
enabled = true
restart_time = 120
data_dir = "/var/lib/reflet/rib"
```

## `[[peers]]`

An array of BGP peer configurations. Each `[[peers]]` entry defines one BGP session. At least one peer is typically required for the looking glass to be useful.

| Option | Type | Default | Description |
|--------|------|---------|-------------|
| `address` | IP address | *(required)* | Peer IP address (IPv4 or IPv6). |
| `remote_asn` | integer (u32) | *(required)* | Peer autonomous system number. Must be greater than 0. |
| `name` | string | *(required)* | Short label shown in the UI, metrics, and lookup results. Must not be empty and must be unique across all peers. |
| `description` | string | `""` | Longer description shown in the peer list. |
| `location` | string | *(none)* | Free-text location (city, datacenter, etc.) displayed alongside the peer. |
| `families` | list of strings | `["ipv4-unicast", "ipv6-unicast"]` | Address families to negotiate. Valid values: `ipv4-unicast`, `ipv6-unicast`. |
| `snapshot_interval` | integer (u64) | *(none)* | Seconds between RIB snapshots for this peer. Set to `0` or omit to disable. Minimum `60`. Requires a `[snapshots]` section. |

```toml
[[peers]]
address = "10.0.0.2"
remote_asn = 65000
name = "Router A"
description = "Primary border router in Amsterdam DC1"
location = "DC1, Amsterdam"
families = ["ipv4-unicast", "ipv6-unicast"]

[[peers]]
address = "10.0.0.3"
remote_asn = 65000
name = "Router B"
description = "Secondary border router in Frankfurt DC2"
location = "DC2, Frankfurt"
families = ["ipv4-unicast"]  # IPv4 only
```

## `[rpki]`

RPKI validation (RFC 6811). Fetches Validated ROA Payloads from a [Routinator](https://routinator.docs.nlnetlabs.nl/)-compatible RPKI validator and annotates routes with their validation status (valid, invalid, or not-found). Validation is applied at API serve-time — statuses are never stored in the RIB.

| Option | Type | Default | Description |
|--------|------|---------|-------------|
| `enabled` | boolean | `false` | Enable RPKI validation. |
| `url` | string | *(none)* | Base URL of a Routinator-compatible validator (the `/json` endpoint is appended automatically). **Required** when `enabled = true`. |
| `refresh_interval` | integer (u64) | `300` | Seconds between VRP refreshes from the validator. |

```toml
[rpki]
enabled = true
url = "https://rpki.example.com"
refresh_interval = 300
```

## `[event_log]`

Route event logging. Records announcements, withdrawals, and session state changes. Events are kept in an in-memory ring buffer and optionally persisted to disk.

| Option | Type | Default | Description |
|--------|------|---------|-------------|
| `enabled` | boolean | `false` | Enable event logging. |
| `buffer_size` | integer | `10000` | Number of recent events kept in the in-memory buffer. Must be greater than 0 when enabled. |
| `file` | string | *(none)* | Path to a JSONL file for persistent event storage. When set, events are appended to this file in addition to the in-memory buffer. |

```toml
[event_log]
enabled = true
buffer_size = 10000
file = "/var/log/reflet/events.jsonl"
```

## `[snapshots]`

Periodic RIB snapshots. Takes a point-in-time copy of each peer's RIB at a configurable interval, stored on disk as gzipped JSON. Snapshots are browsable through the API and the web UI. This section is required when any peer has `snapshot_interval` set.

| Option | Type | Default | Description |
|--------|------|---------|-------------|
| `data_dir` | string | *(required)* | Directory to store snapshot files. Each peer gets a subdirectory. |
| `max_snapshots` | integer | *(none)* | Maximum number of snapshots to keep per peer. Oldest are deleted when exceeded. |
| `max_age_hours` | integer | *(none)* | Delete snapshots older than this many hours. |

```toml
[snapshots]
data_dir = "/var/lib/reflet/snapshots"
max_snapshots = 168
max_age_hours = 720
```

When using Docker, mount the `data_dir` as a volume to persist snapshots across container restarts.

## `[logging]`

Controls the application log output.

| Option | Type | Default | Description |
|--------|------|---------|-------------|
| `level` | string | `"info"` | Log level filter. Valid values: `error`, `warn`, `info`, `debug`, `trace`. |
| `format` | string | `"pretty"` | Log output format. Valid values: `pretty` (human-readable) or `json` (structured). |

```toml
[logging]
level = "info"
format = "pretty"
```

## Validation summary

The configuration is validated at startup. The following rules are enforced:

- `bgp.hold_time` must be `0` or `>= 3` seconds
- `bgp.graceful_restart.restart_time` must be 0–4095
- `bgp.graceful_restart.data_dir` is required when `bgp.graceful_restart.enabled = true`
- `rpki.url` is required when `rpki.enabled = true`
- `event_log.buffer_size` must be greater than 0 when `event_log.enabled = true`
- Each peer's `remote_asn` must be greater than 0
- Each peer's `name` must not be empty
- Peer names must be unique across all `[[peers]]` entries
- `[snapshots]` section with `data_dir` is required when any peer has `snapshot_interval > 0`
- `snapshot_interval` must be `0` (disabled) or `>= 60` seconds

Use `reflet --check --config config.toml` to validate the configuration without starting the server.
