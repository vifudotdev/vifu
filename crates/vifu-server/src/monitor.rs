use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use axum::extract::ws::{Message as AxumMessage, WebSocket};
use axum::extract::{State, WebSocketUpgrade};
use axum::http::header::AUTHORIZATION;
use axum::http::{HeaderMap, HeaderValue};
use axum::response::{IntoResponse, Response};
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::Message as TungsteniteMessage;
use vifu_gateway::protocol::{AgentDescriptor, RuntimeTelemetryBatch};

use crate::auth::{Identity, Operation};
use crate::error::ApiError;
use crate::AppState;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GatewayMonitorSnapshot {
    pub gateway_id: String,
    pub agents: Vec<AgentDescriptor>,
}

/// Payload-safe events accepted by Vifu Server and available to authenticated
/// monitor clients such as the Vifu TUI.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum ServerMonitorEvent {
    Snapshot {
        gateways: Vec<GatewayMonitorSnapshot>,
    },
    GatewayConnected {
        gateway_id: String,
        agents: Vec<AgentDescriptor>,
    },
    GatewayDisconnected {
        gateway_id: String,
    },
    RuntimeTelemetry {
        gateway_id: String,
        batch: RuntimeTelemetryBatch,
    },
}

#[derive(Clone)]
pub struct ServerMonitorHub {
    sender: broadcast::Sender<ServerMonitorEvent>,
    gateways: Arc<Mutex<HashMap<String, Vec<AgentDescriptor>>>>,
}

impl ServerMonitorHub {
    pub fn new(capacity: usize) -> Self {
        let (sender, _) = broadcast::channel(capacity.max(64));
        Self {
            sender,
            gateways: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub fn subscribe(&self) -> broadcast::Receiver<ServerMonitorEvent> {
        self.sender.subscribe()
    }

    pub fn publish(&self, event: ServerMonitorEvent) {
        match &event {
            ServerMonitorEvent::GatewayConnected { gateway_id, agents } => {
                lock(&self.gateways).insert(gateway_id.clone(), agents.clone());
            }
            ServerMonitorEvent::GatewayDisconnected { gateway_id } => {
                lock(&self.gateways).remove(gateway_id);
            }
            ServerMonitorEvent::Snapshot { .. } | ServerMonitorEvent::RuntimeTelemetry { .. } => {}
        }
        let _ = self.sender.send(event);
    }

    pub fn snapshot(&self) -> ServerMonitorEvent {
        let mut gateways = lock(&self.gateways)
            .iter()
            .map(|(gateway_id, agents)| GatewayMonitorSnapshot {
                gateway_id: gateway_id.clone(),
                agents: agents.clone(),
            })
            .collect::<Vec<_>>();
        gateways.sort_by(|left, right| left.gateway_id.cmp(&right.gateway_id));
        ServerMonitorEvent::Snapshot { gateways }
    }
}

pub async fn upgrade(
    State(state): State<AppState>,
    headers: HeaderMap,
    upgrade: WebSocketUpgrade,
) -> Result<Response, ApiError> {
    let identity = state
        .auth
        .authorize(&headers, Operation::DeploymentRead)
        .await?;
    if !matches!(identity, Identity::DeploymentAdmin) {
        return Err(ApiError::Forbidden);
    }
    Ok(upgrade
        .on_upgrade(move |socket| serve_monitor_socket(state.monitor, socket))
        .into_response())
}

async fn serve_monitor_socket(hub: ServerMonitorHub, mut socket: WebSocket) {
    let mut events = hub.subscribe();
    if send_event(&mut socket, &hub.snapshot()).await.is_err() {
        return;
    }
    loop {
        tokio::select! {
            event = events.recv() => {
                match event {
                    Ok(event) if send_event(&mut socket, &event).await.is_ok() => {}
                    Ok(_) | Err(broadcast::error::RecvError::Closed) => break,
                    Err(broadcast::error::RecvError::Lagged(_)) => {
                        if send_event(&mut socket, &hub.snapshot()).await.is_err() {
                            break;
                        }
                    }
                }
            }
            message = socket.recv() => {
                match message {
                    Some(Ok(AxumMessage::Close(_))) | None | Some(Err(_)) => break,
                    Some(Ok(_)) => {}
                }
            }
        }
    }
}

async fn send_event(socket: &mut WebSocket, event: &ServerMonitorEvent) -> Result<(), String> {
    let payload = serde_json::to_string(event)
        .map_err(|error| format!("runtime monitor event could not be encoded: {error}"))?;
    socket
        .send(AxumMessage::Text(payload.into()))
        .await
        .map_err(|error| format!("runtime monitor event could not be sent: {error}"))
}

pub struct RemoteMonitorClient {
    socket: tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
}

impl RemoteMonitorClient {
    pub async fn connect(server_address: &str, credential: &str) -> Result<Self, String> {
        let url = monitor_websocket_url(server_address)?;
        let mut request = url
            .as_str()
            .into_client_request()
            .map_err(|error| format!("runtime monitor request is invalid: {error}"))?;
        let authorization = HeaderValue::from_str(&format!("Bearer {}", credential.trim()))
            .map_err(|_| "runtime monitor credential is invalid".to_string())?;
        request.headers_mut().insert(AUTHORIZATION, authorization);
        let (socket, _) = tokio_tungstenite::connect_async(request)
            .await
            .map_err(|error| format!("runtime monitor connection failed: {error}"))?;
        Ok(Self { socket })
    }

    pub async fn next_event(&mut self) -> Result<Option<ServerMonitorEvent>, String> {
        while let Some(message) = self.socket.next().await {
            match message.map_err(|error| format!("runtime monitor connection failed: {error}"))? {
                TungsteniteMessage::Text(payload) => {
                    let event = serde_json::from_str(payload.as_ref()).map_err(|error| {
                        format!("runtime monitor event could not be decoded: {error}")
                    })?;
                    return Ok(Some(event));
                }
                TungsteniteMessage::Close(_) => return Ok(None),
                TungsteniteMessage::Ping(payload) => {
                    self.socket
                        .send(TungsteniteMessage::Pong(payload))
                        .await
                        .map_err(|error| format!("runtime monitor connection failed: {error}"))?;
                }
                TungsteniteMessage::Binary(_)
                | TungsteniteMessage::Pong(_)
                | TungsteniteMessage::Frame(_) => {}
            }
        }
        Ok(None)
    }
}

fn monitor_websocket_url(server_address: &str) -> Result<reqwest::Url, String> {
    let mut url = reqwest::Url::parse(server_address.trim())
        .map_err(|error| format!("server address is invalid: {error}"))?;
    let websocket_scheme = match url.scheme() {
        "http" => "ws",
        "https" => "wss",
        scheme => return Err(format!("server address scheme {scheme:?} is not supported")),
    };
    url.set_scheme(websocket_scheme)
        .map_err(|_| "server address scheme could not be converted".to_string())?;
    url.set_path("/v1/runtime-monitor/connect");
    Ok(url)
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn broadcasts_gateway_lifecycle_events_to_local_monitors() {
        let hub = ServerMonitorHub::new(1);
        let mut receiver = hub.subscribe();
        hub.publish(ServerMonitorEvent::GatewayConnected {
            gateway_id: "gateway-test".to_string(),
            agents: Vec::new(),
        });

        match receiver.recv().await.unwrap() {
            ServerMonitorEvent::GatewayConnected { gateway_id, agents } => {
                assert_eq!(gateway_id, "gateway-test");
                assert!(agents.is_empty());
            }
            _ => panic!("unexpected monitor event"),
        }
    }

    #[test]
    fn snapshot_tracks_connected_gateways() {
        let hub = ServerMonitorHub::new(1);
        hub.publish(ServerMonitorEvent::GatewayConnected {
            gateway_id: "gateway-test".to_string(),
            agents: Vec::new(),
        });

        assert_eq!(
            hub.snapshot(),
            ServerMonitorEvent::Snapshot {
                gateways: vec![GatewayMonitorSnapshot {
                    gateway_id: "gateway-test".to_string(),
                    agents: Vec::new(),
                }],
            }
        );
    }

    #[test]
    fn builds_runtime_monitor_websocket_url_from_server_origin() {
        assert_eq!(
            monitor_websocket_url("https://api.vifu.ai")
                .unwrap()
                .as_str(),
            "wss://api.vifu.ai/v1/runtime-monitor/connect"
        );
    }
}
