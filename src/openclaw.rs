use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::time::Duration;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Endpoint {
    pub host: String,
    pub port: u16,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProbeStatus {
    Online,
    Offline(String),
    Unsupported(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProbeReport {
    pub endpoint: Endpoint,
    pub status: ProbeStatus,
}

pub fn probe(url: &str) -> ProbeReport {
    match parse_endpoint(url) {
        Ok(endpoint) => {
            let status = probe_endpoint(&endpoint);
            ProbeReport { endpoint, status }
        }
        Err(error) => ProbeReport {
            endpoint: Endpoint {
                host: "invalid".to_string(),
                port: 0,
            },
            status: ProbeStatus::Unsupported(error),
        },
    }
}

pub fn parse_endpoint(url: &str) -> Result<Endpoint, String> {
    let raw = url.trim();
    let rest = raw
        .strip_prefix("http://")
        .ok_or_else(|| "only local http:// OpenClaw Gateway URLs are supported".to_string())?;
    let authority = rest.split('/').next().unwrap_or(rest);
    let (host, port) = parse_authority(authority)?;

    if !is_loopback_host(&host) {
        return Err("only loopback OpenClaw Gateway hosts are supported".to_string());
    }

    Ok(Endpoint { host, port })
}

fn parse_authority(authority: &str) -> Result<(String, u16), String> {
    if authority.is_empty() {
        return Err("OpenClaw Gateway URL is missing a host".to_string());
    }

    if let Some(rest) = authority.strip_prefix('[') {
        let end = rest
            .find(']')
            .ok_or_else(|| "invalid IPv6 loopback URL".to_string())?;
        let host = rest[..end].to_string();
        let port = rest[end + 1..]
            .strip_prefix(':')
            .map(parse_port)
            .transpose()?
            .unwrap_or(18789);
        return Ok((host, port));
    }

    let mut parts = authority.split(':');
    let host = parts.next().unwrap_or_default().to_string();
    let port = match (parts.next(), parts.next()) {
        (Some(value), None) => parse_port(value)?,
        (None, None) => 18789,
        _ => return Err("invalid OpenClaw Gateway authority".to_string()),
    };

    Ok((host, port))
}

fn parse_port(value: &str) -> Result<u16, String> {
    value
        .parse::<u16>()
        .map_err(|_| format!("invalid OpenClaw Gateway port: {value}"))
}

fn is_loopback_host(host: &str) -> bool {
    matches!(host, "127.0.0.1" | "localhost" | "::1")
}

fn probe_endpoint(endpoint: &Endpoint) -> ProbeStatus {
    let socket_addr = match (endpoint.host.as_str(), endpoint.port)
        .to_socket_addrs()
        .ok()
        .and_then(|mut list| list.next())
    {
        Some(value) => value,
        None => return ProbeStatus::Offline("could not resolve loopback address".to_string()),
    };

    let timeout = Duration::from_millis(800);
    let mut stream = match TcpStream::connect_timeout(&socket_addr, timeout) {
        Ok(stream) => stream,
        Err(error) => return ProbeStatus::Offline(error.to_string()),
    };

    let _ = stream.set_read_timeout(Some(timeout));
    let _ = stream.set_write_timeout(Some(timeout));

    let request = format!(
        "GET /health HTTP/1.1\r\nHost: {}\r\nConnection: close\r\n\r\n",
        endpoint.host
    );

    if let Err(error) = stream.write_all(request.as_bytes()) {
        return ProbeStatus::Offline(error.to_string());
    }

    let mut response = String::new();
    if let Err(error) = stream.read_to_string(&mut response) {
        return ProbeStatus::Offline(error.to_string());
    }

    if response.starts_with("HTTP/1.1 200") || response.starts_with("HTTP/1.0 200") {
        ProbeStatus::Online
    } else {
        ProbeStatus::Offline("Gateway responded without healthy status".to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::{parse_endpoint, ProbeStatus};

    #[test]
    fn parses_default_loopback_endpoint() {
        let endpoint = parse_endpoint("http://127.0.0.1:18789").unwrap();
        assert_eq!(endpoint.host, "127.0.0.1");
        assert_eq!(endpoint.port, 18789);
    }

    #[test]
    fn rejects_remote_hosts() {
        let error = parse_endpoint("http://example.com:18789").unwrap_err();
        assert!(error.contains("loopback"));
    }

    #[test]
    fn unsupported_status_is_debuggable() {
        let status = ProbeStatus::Unsupported("bad url".to_string());
        assert_eq!(format!("{status:?}"), "Unsupported(\"bad url\")");
    }
}
