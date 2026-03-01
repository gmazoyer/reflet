use std::net::{IpAddr, SocketAddr};

use axum::Json;
use axum::extract::ConnectInfo;
use axum::http::HeaderMap;
use serde::Serialize;
use utoipa::ToSchema;

#[derive(Serialize, ToSchema)]
pub struct WhoamiResponse {
    /// The client's IP address as seen by the server
    pub ip: String,
}

/// GET /api/v1/whoami
#[utoipa::path(
    get,
    path = "/api/v1/whoami",
    responses(
        (status = 200, description = "Client IP address", body = WhoamiResponse)
    ),
    tag = "summary"
)]
pub async fn whoami(
    headers: HeaderMap,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
) -> Json<WhoamiResponse> {
    let ip = client_ip(&headers, addr.ip());
    Json(WhoamiResponse { ip: ip.to_string() })
}

fn client_ip(headers: &HeaderMap, connect_ip: IpAddr) -> IpAddr {
    // Check X-Real-IP first (typically set by nginx)
    if let Some(val) = headers.get("x-real-ip")
        && let Ok(s) = val.to_str()
        && let Ok(ip) = s.trim().parse::<IpAddr>()
    {
        return normalize_ip(ip);
    }

    // Check X-Forwarded-For (first entry is the original client)
    if let Some(val) = headers.get("x-forwarded-for")
        && let Ok(s) = val.to_str()
        && let Some(first) = s.split(',').next()
        && let Ok(ip) = first.trim().parse::<IpAddr>()
    {
        return normalize_ip(ip);
    }

    normalize_ip(connect_ip)
}

/// Convert IPv4-mapped IPv6 addresses (::ffff:1.2.3.4) to plain IPv4.
fn normalize_ip(ip: IpAddr) -> IpAddr {
    match ip {
        IpAddr::V6(v6) => {
            if let Some(v4) = v6.to_ipv4_mapped() {
                IpAddr::V4(v4)
            } else {
                ip
            }
        }
        _ => ip,
    }
}
