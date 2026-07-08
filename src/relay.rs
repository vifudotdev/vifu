use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::thread;

use crate::openclaw::{self, Endpoint};
use crate::protocol::{self, Message, Role, OPENCLAW_GATEWAY_CAPABILITY};

const HEALTH_REQUEST_ID: u64 = 1;

pub fn run_server(listen_addr: &str) -> Result<(), String> {
    let listener = TcpListener::bind(listen_addr).map_err(|error| error.to_string())?;
    let local_addr = listener.local_addr().map_err(|error| error.to_string())?;

    println!("Vifu relay server");
    println!("Listen: {local_addr}");
    println!("Waiting for Vifu clients...");

    for stream in listener.incoming() {
        match stream {
            Ok(stream) => {
                thread::spawn(move || {
                    if let Err(error) = handle_client(stream) {
                        eprintln!("vifu server: client error: {error}");
                    }
                });
            }
            Err(error) => eprintln!("vifu server: accept error: {error}"),
        }
    }

    Ok(())
}

pub fn run_client(relay_addr: &str, device_id: &str, endpoint: &Endpoint) -> Result<(), String> {
    protocol::validate_device_id(device_id)?;

    let stream = TcpStream::connect(relay_addr).map_err(|error| error.to_string())?;
    let mut writer = stream.try_clone().map_err(|error| error.to_string())?;
    let mut reader = BufReader::new(stream);

    write_message(
        &mut writer,
        &Message::Hello {
            role: Role::Client,
            device_id: device_id.to_string(),
        },
    )?;
    write_message(
        &mut writer,
        &Message::Capability {
            name: OPENCLAW_GATEWAY_CAPABILITY.to_string(),
            target: format!("http://{}:{}", endpoint.host, endpoint.port),
        },
    )?;

    println!("Relay: connected to {relay_addr}");

    while let Some(message) = read_message(&mut reader)? {
        match message {
            Message::Request {
                id,
                capability,
                method,
                path,
                body,
            } => {
                if capability != OPENCLAW_GATEWAY_CAPABILITY {
                    write_message(
                        &mut writer,
                        &Message::Error {
                            id: Some(id),
                            message: "unsupported capability".to_string(),
                        },
                    )?;
                    continue;
                }

                match openclaw::request(endpoint, &method, &path, &body) {
                    Ok(response) => write_message(
                        &mut writer,
                        &Message::Response {
                            id,
                            status: response.status,
                            body: response.body,
                        },
                    )?,
                    Err(error) => write_message(
                        &mut writer,
                        &Message::Error {
                            id: Some(id),
                            message: sanitize_error(&error),
                        },
                    )?,
                }
            }
            Message::Ping => write_message(&mut writer, &Message::Pong)?,
            Message::Error { message, .. } => {
                return Err(format!(
                    "relay returned an error: {}",
                    sanitize_error(&message)
                ));
            }
            _ => return Err("relay sent an unexpected protocol message".to_string()),
        }
    }

    println!("Relay: disconnected");
    Ok(())
}

fn handle_client(stream: TcpStream) -> Result<(), String> {
    let peer = stream
        .peer_addr()
        .map(|addr| addr.to_string())
        .unwrap_or_else(|_| "unknown".to_string());
    let mut writer = stream.try_clone().map_err(|error| error.to_string())?;
    let mut reader = BufReader::new(stream);

    let hello = read_message(&mut reader)?.ok_or_else(|| "client disconnected".to_string())?;
    let device_id = match hello {
        Message::Hello {
            role: Role::Client,
            device_id,
        } => device_id,
        _ => return Err("client must send HELLO first".to_string()),
    };

    let capability = read_message(&mut reader)?
        .ok_or_else(|| "client disconnected before capability".to_string())?;
    match capability {
        Message::Capability { name, .. } if name == OPENCLAW_GATEWAY_CAPABILITY => {}
        Message::Capability { name, .. } => {
            return Err(format!("unsupported client capability: {name}"));
        }
        _ => return Err("client must register capability after HELLO".to_string()),
    }

    println!("Client: {device_id} from {peer}");
    write_message(
        &mut writer,
        &Message::Request {
            id: HEALTH_REQUEST_ID,
            capability: OPENCLAW_GATEWAY_CAPABILITY.to_string(),
            method: "GET".to_string(),
            path: "/health".to_string(),
            body: Vec::new(),
        },
    )?;

    while let Some(message) = read_message(&mut reader)? {
        match message {
            Message::Response { id, status, body } if id == HEALTH_REQUEST_ID => {
                println!(
                    "OpenClaw health through relay: status={status} body_bytes={}",
                    body.len()
                );
                return Ok(());
            }
            Message::Error { id, message } if id == Some(HEALTH_REQUEST_ID) => {
                return Err(format!(
                    "OpenClaw health failed: {}",
                    sanitize_error(&message)
                ));
            }
            Message::Pong => {}
            _ => return Err("client sent an unexpected protocol message".to_string()),
        }
    }

    Err("client disconnected before health response".to_string())
}

fn read_message<R: BufRead>(reader: &mut R) -> Result<Option<Message>, String> {
    let mut line = String::new();
    let count = reader
        .read_line(&mut line)
        .map_err(|error| error.to_string())?;
    if count == 0 {
        return Ok(None);
    }
    protocol::decode(&line).map(Some)
}

fn write_message<W: Write>(writer: &mut W, message: &Message) -> Result<(), String> {
    let encoded = protocol::encode(message)?;
    writer
        .write_all(encoded.as_bytes())
        .map_err(|error| error.to_string())?;
    writer.flush().map_err(|error| error.to_string())
}

fn sanitize_error(value: &str) -> String {
    let mut output = String::with_capacity(value.len().min(256));
    for ch in value.chars().take(256) {
        if ch.is_control() && ch != '\n' {
            output.push(' ');
        } else {
            output.push(ch);
        }
    }
    if output.is_empty() {
        "unknown error".to_string()
    } else {
        output
    }
}

#[cfg(test)]
mod tests {
    use super::sanitize_error;

    #[test]
    fn sanitize_error_removes_control_characters() {
        assert_eq!(sanitize_error("bad\0token"), "bad token");
    }
}
