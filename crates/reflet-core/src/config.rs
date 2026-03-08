use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::Path;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::prefix::AddressFamily;

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("failed to read config file: {0}")]
    Io(#[from] std::io::Error),
    #[error("failed to parse config: {0}")]
    Parse(#[from] toml::de::Error),
    #[error("validation error: {0}")]
    Validation(String),
}

/// Top-level configuration.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub server: ServerConfig,
    #[serde(default)]
    pub bgp: BgpConfig,
    #[serde(default)]
    pub peers: Vec<PeerConfig>,
    #[serde(default)]
    pub logging: LoggingConfig,
    #[serde(default)]
    pub communities_dir: Option<String>,
    #[serde(default)]
    pub ipinfo_dataset_file: Option<String>,
    #[serde(default)]
    pub event_log: EventLogConfig,
    #[serde(default)]
    pub rpki: RpkiConfig,
}

/// Event log configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventLogConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_event_buffer_size")]
    pub buffer_size: usize,
    #[serde(default)]
    pub file: Option<String>,
}

fn default_event_buffer_size() -> usize {
    10_000
}

impl Default for EventLogConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            buffer_size: default_event_buffer_size(),
            file: None,
        }
    }
}

/// RPKI validation configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RpkiConfig {
    #[serde(default)]
    pub enabled: bool,
    /// Base URL of a Routinator (or compatible) RPKI validator.
    #[serde(default)]
    pub url: Option<String>,
    /// How often to refresh VRPs, in seconds.
    #[serde(default = "default_rpki_refresh")]
    pub refresh_interval: u64,
}

fn default_rpki_refresh() -> u64 {
    300
}

impl Default for RpkiConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            url: None,
            refresh_interval: default_rpki_refresh(),
        }
    }
}

/// HTTP server configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerConfig {
    #[serde(default = "default_http_listen")]
    pub listen: SocketAddr,
    #[serde(default = "default_bgp_listen")]
    pub bgp_listen: SocketAddr,
    /// Display title shown in the web UI header.
    #[serde(default = "default_title")]
    pub title: String,
    /// When true, peer IP addresses, router IDs, and next-hops are hidden
    /// from API responses. Useful when exposing the looking glass publicly.
    #[serde(default)]
    pub hide_peer_addresses: bool,
    /// When true, the route refresh API is disabled and the UI button is hidden.
    /// Useful for public instances to prevent abuse.
    #[serde(default)]
    pub disable_route_refresh: bool,
}

/// BGP speaker configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BgpConfig {
    #[serde(default = "default_local_asn")]
    pub local_asn: u32,
    #[serde(default = "default_router_id")]
    pub router_id: Ipv4Addr,
    #[serde(default = "default_hold_time")]
    pub hold_time: u16,
    #[serde(default)]
    pub graceful_restart: GracefulRestartConfig,
}

/// Graceful Restart (RFC 4724) configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GracefulRestartConfig {
    #[serde(default)]
    pub enabled: bool,
    /// Restart Time in seconds (0–4095). Advertised to peers.
    #[serde(default = "default_restart_time")]
    pub restart_time: u16,
    /// Directory to persist RIB data for restart recovery.
    #[serde(default)]
    pub data_dir: Option<String>,
}

fn default_restart_time() -> u16 {
    120
}

impl Default for GracefulRestartConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            restart_time: default_restart_time(),
            data_dir: None,
        }
    }
}

/// Per-peer configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PeerConfig {
    pub address: IpAddr,
    pub remote_asn: u32,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub location: Option<String>,
    #[serde(default = "default_families")]
    pub families: Vec<AddressFamily>,
}

/// Logging configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoggingConfig {
    #[serde(default = "default_log_level")]
    pub level: String,
    #[serde(default = "default_log_format")]
    pub format: LogFormat,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LogFormat {
    Json,
    Pretty,
}

fn default_title() -> String {
    "Reflet".to_string()
}

fn default_http_listen() -> SocketAddr {
    "0.0.0.0:8080".parse().unwrap()
}

fn default_bgp_listen() -> SocketAddr {
    "0.0.0.0:179".parse().unwrap()
}

fn default_local_asn() -> u32 {
    65000
}

fn default_router_id() -> Ipv4Addr {
    Ipv4Addr::new(10, 0, 0, 1)
}

fn default_hold_time() -> u16 {
    90
}

fn default_families() -> Vec<AddressFamily> {
    vec![AddressFamily::Ipv4Unicast, AddressFamily::Ipv6Unicast]
}

fn default_log_level() -> String {
    "info".to_string()
}

fn default_log_format() -> LogFormat {
    LogFormat::Pretty
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            listen: default_http_listen(),
            bgp_listen: default_bgp_listen(),
            title: default_title(),
            hide_peer_addresses: false,
            disable_route_refresh: false,
        }
    }
}

impl Default for BgpConfig {
    fn default() -> Self {
        Self {
            local_asn: default_local_asn(),
            router_id: default_router_id(),
            hold_time: default_hold_time(),
            graceful_restart: GracefulRestartConfig::default(),
        }
    }
}

impl Default for LoggingConfig {
    fn default() -> Self {
        Self {
            level: default_log_level(),
            format: default_log_format(),
        }
    }
}

impl Config {
    /// Load configuration from a TOML file.
    pub fn from_file(path: &Path) -> Result<Self, ConfigError> {
        let content = std::fs::read_to_string(path)?;
        let config: Config = toml::from_str(&content)?;
        config.validate()?;
        Ok(config)
    }

    /// Parse configuration from a TOML string.
    pub fn from_toml(s: &str) -> Result<Self, ConfigError> {
        let config: Config = toml::from_str(s)?;
        config.validate()?;
        Ok(config)
    }

    /// Validate the configuration.
    pub fn validate(&self) -> Result<(), ConfigError> {
        if self.bgp.hold_time != 0 && self.bgp.hold_time < 3 {
            return Err(ConfigError::Validation(
                "hold_time must be 0 or >= 3 seconds".to_string(),
            ));
        }
        if self.bgp.graceful_restart.restart_time > 4095 {
            return Err(ConfigError::Validation(
                "graceful_restart.restart_time must be 0–4095 (12-bit field)".to_string(),
            ));
        }
        if self.bgp.graceful_restart.enabled && self.bgp.graceful_restart.data_dir.is_none() {
            return Err(ConfigError::Validation(
                "graceful_restart.data_dir is required when graceful_restart is enabled"
                    .to_string(),
            ));
        }
        if self.event_log.enabled && self.event_log.buffer_size == 0 {
            return Err(ConfigError::Validation(
                "event_log.buffer_size must be > 0 when event_log is enabled".to_string(),
            ));
        }
        if self.rpki.enabled && self.rpki.url.is_none() {
            return Err(ConfigError::Validation(
                "rpki.url is required when rpki is enabled".to_string(),
            ));
        }
        let mut seen_names = std::collections::HashSet::new();
        for peer in &self.peers {
            if peer.remote_asn == 0 {
                return Err(ConfigError::Validation(format!(
                    "peer {} has invalid remote_asn 0",
                    peer.address
                )));
            }
            if peer.name.is_empty() {
                return Err(ConfigError::Validation(format!(
                    "peer {} has an empty name",
                    peer.address
                )));
            }
            if !seen_names.insert(&peer.name) {
                return Err(ConfigError::Validation(format!(
                    "duplicate peer name '{}'",
                    peer.name
                )));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_full_config() {
        let toml = r#"
[server]
listen = "0.0.0.0:8080"
bgp_listen = "0.0.0.0:1179"

[bgp]
local_asn = 65000
router_id = "10.0.0.1"
hold_time = 90

[[peers]]
address = "10.0.0.2"
remote_asn = 65001
name = "Router A"
description = "Primary router in Amsterdam"
location = "DC1, Amsterdam"
families = ["ipv4-unicast", "ipv6-unicast"]

[[peers]]
address = "10.0.0.3"
remote_asn = 65002
name = "Router B"
description = "Secondary router in Frankfurt"
families = ["ipv4-unicast"]

[logging]
level = "debug"
format = "json"
"#;
        let config = Config::from_toml(toml).unwrap();
        assert_eq!(config.bgp.local_asn, 65000);
        assert_eq!(config.peers.len(), 2);
        assert_eq!(config.peers[0].remote_asn, 65001);
        assert_eq!(config.peers[0].location.as_deref(), Some("DC1, Amsterdam"));
        assert!(config.peers[1].location.is_none());
        assert_eq!(config.peers[1].families.len(), 1);
        assert_eq!(
            config.server.bgp_listen,
            "0.0.0.0:1179".parse::<SocketAddr>().unwrap()
        );
    }

    #[test]
    fn defaults_applied() {
        let config = Config::from_toml("").unwrap();
        assert_eq!(config.bgp.local_asn, 65000);
        assert_eq!(config.bgp.hold_time, 90);
        assert!(config.peers.is_empty());
    }

    #[test]
    fn invalid_hold_time() {
        let toml = r#"
[bgp]
hold_time = 2
"#;
        let result = Config::from_toml(toml);
        assert!(result.is_err());
    }

    #[test]
    fn invalid_peer_asn() {
        let toml = r#"
[[peers]]
address = "10.0.0.2"
remote_asn = 0
name = "Router A"
"#;
        let result = Config::from_toml(toml);
        assert!(result.is_err());
    }

    #[test]
    fn peer_default_families() {
        let toml = r#"
[[peers]]
address = "10.0.0.2"
remote_asn = 65001
name = "Router A"
"#;
        let config = Config::from_toml(toml).unwrap();
        assert_eq!(config.peers[0].families.len(), 2);
    }

    #[test]
    fn validate_peer_empty_name() {
        let toml = r#"
[[peers]]
address = "10.0.0.2"
remote_asn = 65001
"#;
        let result = Config::from_toml(toml);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("empty name"));
    }

    #[test]
    fn validate_peer_duplicate_names() {
        let toml = r#"
[[peers]]
address = "10.0.0.2"
remote_asn = 65001
name = "Router A"

[[peers]]
address = "10.0.0.3"
remote_asn = 65002
name = "Router A"
"#;
        let result = Config::from_toml(toml);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("duplicate peer name"));
    }

    #[test]
    fn parse_gr_config() {
        let toml = r#"
[bgp]
local_asn = 65000

[bgp.graceful_restart]
enabled = true
restart_time = 90
data_dir = "/tmp/rib"
"#;
        let config = Config::from_toml(toml).unwrap();
        assert!(config.bgp.graceful_restart.enabled);
        assert_eq!(config.bgp.graceful_restart.restart_time, 90);
        assert_eq!(
            config.bgp.graceful_restart.data_dir.as_deref(),
            Some("/tmp/rib")
        );
    }

    #[test]
    fn validate_restart_time_too_large() {
        let toml = r#"
[bgp.graceful_restart]
restart_time = 5000
"#;
        let result = Config::from_toml(toml);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("4095"));
    }

    #[test]
    fn parse_event_log_config() {
        let toml = r#"
[event_log]
enabled = true
buffer_size = 5000
file = "/tmp/events.jsonl"
"#;
        let config = Config::from_toml(toml).unwrap();
        assert!(config.event_log.enabled);
        assert_eq!(config.event_log.buffer_size, 5000);
        assert_eq!(config.event_log.file.as_deref(), Some("/tmp/events.jsonl"));
    }

    #[test]
    fn validate_enabled_zero_buffer() {
        let toml = r#"
[event_log]
enabled = true
buffer_size = 0
"#;
        let result = Config::from_toml(toml);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("buffer_size"));
    }

    #[test]
    fn parse_rpki_config() {
        let toml = r#"
[rpki]
enabled = true
url = "https://rpki.example.com"
refresh_interval = 600
"#;
        let config = Config::from_toml(toml).unwrap();
        assert!(config.rpki.enabled);
        assert_eq!(config.rpki.url.as_deref(), Some("https://rpki.example.com"));
        assert_eq!(config.rpki.refresh_interval, 600);
    }

    #[test]
    fn rpki_defaults() {
        let config = Config::from_toml("").unwrap();
        assert!(!config.rpki.enabled);
        assert!(config.rpki.url.is_none());
        assert_eq!(config.rpki.refresh_interval, 300);
    }

    #[test]
    fn validate_rpki_enabled_no_url() {
        let toml = r#"
[rpki]
enabled = true
"#;
        let result = Config::from_toml(toml);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("rpki.url"));
    }

    #[test]
    fn parse_disable_route_refresh() {
        let toml = r#"
[server]
disable_route_refresh = true
"#;
        let config = Config::from_toml(toml).unwrap();
        assert!(config.server.disable_route_refresh);
    }

    #[test]
    fn disable_route_refresh_defaults_to_false() {
        let config = Config::from_toml("").unwrap();
        assert!(!config.server.disable_route_refresh);
    }

    #[test]
    fn validate_gr_enabled_no_data_dir() {
        let toml = r#"
[bgp.graceful_restart]
enabled = true
"#;
        let result = Config::from_toml(toml);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("data_dir"));
    }
}
