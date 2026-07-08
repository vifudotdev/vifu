use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::time::Duration;

use crate::protocol;

const MAX_HTTP_RESPONSE_BYTES: usize = protocol::MAX_BODY_BYTES + 16 * 1024;

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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GatewayResponse {
    pub status: u16,
    pub body: Vec<u8>,
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
    match request(endpoint, "GET", "/health", &[]) {
        Ok(response) if (200..=299).contains(&response.status) => ProbeStatus::Online,
        Ok(_) => ProbeStatus::Offline("Gateway responded without healthy status".to_string()),
        Err(error) => ProbeStatus::Offline(error),
    }
}

pub fn request(
    endpoint: &Endpoint,
    method: &str,
    path: &str,
    body: &[u8],
) -> Result<GatewayResponse, String> {
    protocol::validate_method(method)?;
    protocol::validate_path(path)?;
    protocol::validate_body(body)?;

    let socket_addr = match (endpoint.host.as_str(), endpoint.port)
        .to_socket_addrs()
        .ok()
        .and_then(|mut list| list.next())
    {
        Some(value) => value,
        None => return Err("could not resolve loopback address".to_string()),
    };

    let timeout = Duration::from_secs(2);
    let mut stream = match TcpStream::connect_timeout(&socket_addr, timeout) {
        Ok(stream) => stream,
        Err(error) => return Err(error.to_string()),
    };

    let _ = stream.set_read_timeout(Some(timeout));
    let _ = stream.set_write_timeout(Some(timeout));

    let request = format!(
        "{method} {path} HTTP/1.1\r\nHost: {}\r\nConnection: close\r\nContent-Length: {}\r\n{}\r\n",
        endpoint.host,
        body.len(),
        if body.is_empty() {
            ""
        } else {
            "Content-Type: application/json\r\n"
        }
    );

    if let Err(error) = stream.write_all(request.as_bytes()) {
        return Err(error.to_string());
    }
    if let Err(error) = stream.write_all(body) {
        return Err(error.to_string());
    }

    let response = read_limited(&mut stream, MAX_HTTP_RESPONSE_BYTES)?;
    parse_http_response(&response)
}

fn read_limited(stream: &mut TcpStream, limit: usize) -> Result<Vec<u8>, String> {
    let mut output = Vec::new();
    let mut buffer = [0; 8192];
    loop {
        let count = stream
            .read(&mut buffer)
            .map_err(|error| error.to_string())?;
        if count == 0 {
            break;
        }
        output.extend_from_slice(&buffer[..count]);
        if output.len() > limit {
            return Err("OpenClaw response is too large".to_string());
        }
    }
    Ok(output)
}

fn parse_http_response(response: &[u8]) -> Result<GatewayResponse, String> {
    let header_end = response
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .ok_or_else(|| "OpenClaw response is missing HTTP headers".to_string())?;
    let headers = std::str::from_utf8(&response[..header_end])
        .map_err(|_| "OpenClaw response headers are not utf-8".to_string())?;
    let status_line = headers
        .lines()
        .next()
        .ok_or_else(|| "OpenClaw response is missing a status line".to_string())?;
    let status = status_line
        .split_whitespace()
        .nth(1)
        .ok_or_else(|| "OpenClaw response status is missing".to_string())?
        .parse::<u16>()
        .map_err(|_| "OpenClaw response status is invalid".to_string())?;
    let body = response[header_end + 4..].to_vec();
    protocol::validate_body(&body)?;

    Ok(GatewayResponse { status, body })
}

#[cfg(test)]
mod tests {
    use super::{parse_endpoint, parse_http_response, ProbeStatus};

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

    #[test]
    fn parses_http_response_body() {
        let response =
            parse_http_response(b"HTTP/1.1 200 OK\r\nContent-Length: 11\r\n\r\nhello world")
                .unwrap();
        assert_eq!(response.status, 200);
        assert_eq!(response.body, b"hello world");
    }
}
