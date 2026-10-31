//! `daedra serve`: run the MCP server over stdio or SSE.

use daedra::{
    DaedraResult,
    cache::CacheConfig,
    server::{DaedraServer, ServerConfig, TransportType},
    types::DaedraError,
};
use std::net::ToSocketAddrs;

use crate::cli::args::TransportOption;

pub(crate) fn parse_host_octets(host: &str) -> DaedraResult<[u8; 4]> {
    if host.eq_ignore_ascii_case("localhost") {
        return Ok([127, 0, 0, 1]);
    }
    let parts: Vec<u8> = host.split('.').filter_map(|s| s.parse().ok()).collect();
    if parts.len() == 4 {
        return Ok([parts[0], parts[1], parts[2], parts[3]]);
    }
    // Hostname: resolve it. SSE binds an IPv4 socket, so take the first
    // resolved A record.
    let resolved = (host, 0u16)
        .to_socket_addrs()
        .map_err(|e| DaedraError::InvalidArguments(format!("cannot resolve host {host:?}: {e}")))?
        .find(|a| a.is_ipv4());
    match resolved.and_then(|a| match a.ip() {
        std::net::IpAddr::V4(v4) => Some(v4.octets()),
        std::net::IpAddr::V6(_) => None,
    }) {
        Some(octets) => Ok(octets),
        None => Err(DaedraError::InvalidArguments(format!(
            "cannot resolve host {host:?} to an IPv4 address"
        ))),
    }
}

pub(super) async fn run_serve(
    transport: TransportOption,
    port: u16,
    host: String,
    cache: CacheConfig,
) -> DaedraResult<()> {
    let config = ServerConfig {
        cache,
        verbose: false,
        ..Default::default()
    };

    let server = DaedraServer::new(config)?;

    let transport_type = match transport {
        TransportOption::Stdio => TransportType::Stdio,
        TransportOption::Sse => TransportType::Sse {
            port,
            host: parse_host_octets(&host)?,
        },
    };

    server.run(transport_type).await
}
