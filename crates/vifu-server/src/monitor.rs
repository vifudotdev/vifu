use std::collections::{HashMap, HashSet};
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

use crate::auth::{bearer_token, hash_api_key, Identity, Operation};
use crate::db;
use crate::error::ApiError;
use crate::models::ResourcePermission;
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
    GatewayEnrolled {
        gateway_id: String,
        enrollment_id: uuid::Uuid,
    },
    RuntimeTelemetry {
        gateway_id: String,
        batch: Box<RuntimeTelemetryBatch>,
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
            ServerMonitorEvent::Snapshot { .. }
            | ServerMonitorEvent::GatewayEnrolled { .. }
            | ServerMonitorEvent::RuntimeTelemetry { .. } => {}
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
    let scope = monitor_scope(&state, &headers).await?;
    Ok(upgrade
        .on_upgrade(move |socket| serve_monitor_socket(state, scope, socket))
        .into_response())
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum MonitorScope {
    Deployment,
    Projects {
        ids: HashSet<uuid::Uuid>,
        slugs: HashSet<String>,
    },
}

impl MonitorScope {
    fn allows_project_slug(&self, slug: &str) -> bool {
        match self {
            Self::Deployment => true,
            Self::Projects { slugs, .. } => slugs.contains(slug),
        }
    }

    async fn allows_gateway(&self, state: &AppState, gateway_id: &str) -> bool {
        match self {
            Self::Deployment => true,
            Self::Projects { ids, .. } => db::list_projects_for_gateway(&state.pool, gateway_id)
                .await
                .is_ok_and(|projects| projects.iter().any(|(id, _)| ids.contains(id))),
        }
    }

    async fn gateway_agents(
        &self,
        state: &AppState,
        gateway_id: &str,
        agents: Vec<AgentDescriptor>,
    ) -> Option<Vec<AgentDescriptor>> {
        let Self::Projects { ids, .. } = self else {
            return Some(agents);
        };
        let assigned = db::list_projects_for_gateway(&state.pool, gateway_id)
            .await
            .ok()?;
        let visible_projects = assigned
            .into_iter()
            .map(|(id, _)| id)
            .filter(|id| ids.contains(id))
            .collect::<Vec<_>>();
        if visible_projects.is_empty() {
            return None;
        }
        let mut allowed_resources = HashSet::new();
        let mut legacy_agent_ids = HashSet::new();
        for project_id in visible_projects {
            let resources = db::list_project_profile_provider_resources(&state.pool, project_id)
                .await
                .ok()?;
            if resources.is_empty() {
                let profiles = db::list_project_profiles(&state.pool, project_id)
                    .await
                    .ok()?;
                legacy_agent_ids.extend(profiles.into_iter().map(|profile| profile.slug));
            } else {
                allowed_resources.extend(resources);
            }
        }
        Some(
            agents
                .into_iter()
                .filter(|agent| {
                    legacy_agent_ids.contains(&agent.id)
                        || monitor_resource_matches_agent(&allowed_resources, agent)
                })
                .collect(),
        )
    }
}

fn monitor_resource_matches_agent(
    allowed_resources: &HashSet<(String, String)>,
    agent: &AgentDescriptor,
) -> bool {
    let provider_key = agent
        .metadata
        .get("providerKey")
        .and_then(serde_json::Value::as_str);
    allowed_resources
        .iter()
        .any(|(allowed_provider, resource_id)| {
            resource_id == &agent.id
                && provider_key.is_none_or(|provider_key| provider_key == allowed_provider)
        })
}

async fn monitor_scope(state: &AppState, headers: &HeaderMap) -> Result<MonitorScope, ApiError> {
    let deployment_error = match state.auth.authorize(headers, Operation::ProjectRead).await {
        Ok(Identity::DeploymentAdmin) => return Ok(MonitorScope::Deployment),
        Ok(Identity::ActingUser { subject, .. }) => {
            let projects = db::list_projects_for_owner_user_id(&state.pool, &subject).await?;
            return Ok(project_scope(
                projects
                    .into_iter()
                    .map(|project| (project.project.id, project.project.slug)),
            ));
        }
        Err(error) => error,
    };

    let Some(token) = bearer_token(headers).filter(|token| token.starts_with("vifu_pk_")) else {
        return Err(deployment_error);
    };
    let key_hash = hash_api_key(token, &state.config.api_key_pepper);
    let key = db::active_api_key_by_hash(&state.pool, &key_hash).await?;
    if !matches!(
        key.permissions.project,
        ResourcePermission::Read | ResourcePermission::Write
    ) {
        return Err(ApiError::EndpointAccessDenied);
    }
    let project = db::get_project(&state.pool, key.project_id).await?;
    Ok(project_scope([(project.project.id, project.project.slug)]))
}

fn project_scope(projects: impl IntoIterator<Item = (uuid::Uuid, String)>) -> MonitorScope {
    let (ids, slugs) = projects.into_iter().unzip();
    MonitorScope::Projects { ids, slugs }
}

async fn serve_monitor_socket(state: AppState, scope: MonitorScope, mut socket: WebSocket) {
    let mut events = state.monitor.subscribe();
    let snapshot = scoped_event(&state, &scope, state.monitor.snapshot()).await;
    if send_event(&mut socket, &snapshot).await.is_err() {
        return;
    }
    loop {
        tokio::select! {
            event = events.recv() => {
                match event {
                    Ok(event) => {
                        if let Some(event) = scoped_event_optional(&state, &scope, event).await {
                            if send_event(&mut socket, &event).await.is_err() {
                                break;
                            }
                        }
                    }
                    Err(broadcast::error::RecvError::Closed) => break,
                    Err(broadcast::error::RecvError::Lagged(_)) => {
                        let snapshot = scoped_event(&state, &scope, state.monitor.snapshot()).await;
                        if send_event(&mut socket, &snapshot).await.is_err() {
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

async fn scoped_event(
    state: &AppState,
    scope: &MonitorScope,
    event: ServerMonitorEvent,
) -> ServerMonitorEvent {
    scoped_event_optional(state, scope, event)
        .await
        .unwrap_or(ServerMonitorEvent::Snapshot {
            gateways: Vec::new(),
        })
}

async fn scoped_event_optional(
    state: &AppState,
    scope: &MonitorScope,
    event: ServerMonitorEvent,
) -> Option<ServerMonitorEvent> {
    match event {
        ServerMonitorEvent::Snapshot { gateways } => {
            let mut visible = Vec::with_capacity(gateways.len());
            for mut gateway in gateways {
                if let Some(agents) = scope
                    .gateway_agents(state, &gateway.gateway_id, gateway.agents)
                    .await
                {
                    gateway.agents = agents;
                    visible.push(gateway);
                }
            }
            Some(ServerMonitorEvent::Snapshot { gateways: visible })
        }
        ServerMonitorEvent::GatewayConnected { gateway_id, agents } => scope
            .gateway_agents(state, &gateway_id, agents)
            .await
            .map(|agents| ServerMonitorEvent::GatewayConnected { gateway_id, agents }),
        ServerMonitorEvent::GatewayDisconnected { gateway_id } => scope
            .allows_gateway(state, &gateway_id)
            .await
            .then_some(ServerMonitorEvent::GatewayDisconnected { gateway_id }),
        ServerMonitorEvent::GatewayEnrolled {
            gateway_id,
            enrollment_id,
        } => scope.allows_gateway(state, &gateway_id).await.then_some(
            ServerMonitorEvent::GatewayEnrolled {
                gateway_id,
                enrollment_id,
            },
        ),
        ServerMonitorEvent::RuntimeTelemetry { gateway_id, batch } => scope
            .allows_project_slug(&batch.project_id)
            .then_some(ServerMonitorEvent::RuntimeTelemetry { gateway_id, batch }),
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
    use std::path::{Path, PathBuf};
    use std::sync::Arc;
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    use tokio::net::TcpListener;
    use tokio::sync::Notify;
    use uuid::Uuid;
    use vifu_gateway::identity::MachineIdentity;
    use vifu_gateway::protocol::AgentDescriptor;
    use vifu_gateway::relay::{
        run_agent_gateway, AgentGatewayProvider, AgentGatewayRuntime, GatewayConnectionState,
        GatewayOutputPolicy, GatewayRuntimeEvent,
    };
    use vifu_gateway::session::{read_session, SessionStatus, SessionSummary};

    use super::*;

    struct LiveTestFiles {
        database: PathBuf,
        runtime_database: PathBuf,
        session: PathBuf,
    }

    impl LiveTestFiles {
        fn new(name: &str) -> Self {
            let root = std::env::temp_dir().join(format!(
                "vifu-topology-live-{name}-{}",
                Uuid::new_v4().simple()
            ));
            std::fs::create_dir_all(&root).unwrap();
            Self {
                database: root.join("server.sqlite"),
                runtime_database: root.join("runtime.sqlite"),
                session: root.join("gateway-session.json"),
            }
        }

        fn root(&self) -> &Path {
            self.database.parent().unwrap()
        }
    }

    impl Drop for LiveTestFiles {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(self.root());
        }
    }

    fn live_test_config(
        addr: std::net::SocketAddr,
        files: &LiveTestFiles,
    ) -> crate::config::Config {
        crate::config::Config {
            addr,
            deployment_mode: crate::config::DeploymentMode::Local,
            service_version: env!("CARGO_PKG_VERSION").to_string(),
            database_url: format!("sqlite://{}", files.database.display()),
            database_max_connections: 5,
            admin_key: "synthetic-topology-admin".to_string(),
            agent_gateway_bootstrap_token: "synthetic-topology-bootstrap".to_string(),
            api_key_pepper: "synthetic-topology-pepper".to_string(),
            provider_secret_key: "synthetic-topology-provider-secret".to_string(),
            request_timeout: Duration::from_secs(3),
            heartbeat_interval: Duration::from_secs(1),
            queue_capacity: 64,
            provider_home_dir: files.root().to_path_buf(),
            provider_registry_file: None,
            runtime_extensions: Vec::new(),
            access_token_authority: None,
            guest_bootstrap_enabled: false,
            guest_project_ttl: Duration::from_secs(24 * 60 * 60),
            guest_project_limit: 8,
            server_url: Some(format!("http://{addr}")),
            public_dashboard_url: None,
            dashboard_addr: None,
            tls: None,
        }
    }

    async fn live_test_storage(files: &LiveTestFiles) -> crate::db::Storage {
        let storage = crate::db::connect(&format!("sqlite://{}", files.database.display()), 5)
            .await
            .unwrap();
        crate::db::migrate(&storage).await.unwrap();
        storage
    }

    async fn close_storage(storage: crate::db::Storage) {
        match storage {
            crate::db::Storage::Sqlite(pool) => pool.close().await,
            crate::db::Storage::Postgres(_) => unreachable!(),
        }
    }

    fn spawn_live_gateway(
        server_url: String,
        bootstrap_token: String,
        session_path: PathBuf,
        runtime_database_path: PathBuf,
        connected: Arc<Notify>,
    ) -> tokio::task::JoinHandle<Result<(), String>> {
        tokio::spawn(async move {
            let providers: Vec<Arc<dyn AgentGatewayProvider>> = Vec::new();
            let agents = vec![AgentDescriptor {
                id: "topology-agent".to_string(),
                name: "Topology agent".to_string(),
                metadata: serde_json::json!({
                    "providerKey": "synthetic-provider",
                    "providerType": "synthetic"
                }),
            }];
            let mut session = match read_session(&session_path) {
                SessionStatus::Ready(session) => *session,
                SessionStatus::Missing => SessionSummary::new(
                    MachineIdentity::generate()?,
                    SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .map_err(|error| error.to_string())?
                        .as_secs()
                        .max(1),
                )?,
                SessionStatus::Invalid(error) => return Err(error),
            };
            let observer = Arc::new(move |event| {
                if matches!(
                    event,
                    GatewayRuntimeEvent::ConnectionStatus {
                        state: GatewayConnectionState::Connected | GatewayConnectionState::Degraded,
                        ..
                    }
                ) {
                    connected.notify_one();
                }
            });
            run_agent_gateway(
                AgentGatewayRuntime {
                    server_url: &server_url,
                    server_certificate_der: None,
                    agent_gateway_bootstrap_token: Some(&bootstrap_token),
                    enrollment_token: None,
                    allow_guest_bootstrap: false,
                    providers: &providers,
                    agents: &agents,
                    route_overrides: None,
                    runtime_observer: Some(observer),
                    capture_sender: None,
                    config_epoch: 1,
                    provider_models: None,
                    session_path: Some(&session_path),
                    runtime_database_path: &runtime_database_path,
                    embedded_runtime: None,
                    embedded_monitor: None,
                    output_policy: GatewayOutputPolicy::Observer,
                },
                &mut session,
            )
            .await
        })
    }

    async fn next_gateway_event(
        monitor: &mut RemoteMonitorClient,
        connected: bool,
    ) -> (String, Vec<AgentDescriptor>) {
        tokio::time::timeout(Duration::from_secs(8), async {
            loop {
                match monitor.next_event().await.unwrap() {
                    Some(ServerMonitorEvent::GatewayConnected { gateway_id, agents })
                        if connected =>
                    {
                        return (gateway_id, agents);
                    }
                    Some(ServerMonitorEvent::GatewayDisconnected { gateway_id }) if !connected => {
                        return (gateway_id, Vec::new());
                    }
                    Some(_) => {}
                    None => panic!("runtime monitor closed before the expected Gateway event"),
                }
            }
        })
        .await
        .expect("runtime monitor should receive the expected Gateway event")
    }

    async fn wait_for_session(path: &Path) {
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if matches!(read_session(path), SessionStatus::Ready(_)) {
                    return;
                }
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
        })
        .await
        .expect("Gateway session should be persisted");
    }

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

    #[test]
    fn project_monitor_scope_only_allows_its_project_slug() {
        let project_id = uuid::Uuid::new_v4();
        let scope = project_scope([(project_id, "owned-project".to_string())]);

        assert!(scope.allows_project_slug("owned-project"));
        assert!(!scope.allows_project_slug("another-project"));
    }

    #[tokio::test]
    async fn project_monitor_filters_agents_shared_by_the_same_gateway() {
        let path = std::env::temp_dir().join(format!(
            "vifu-monitor-scope-{}.sqlite",
            uuid::Uuid::new_v4().simple()
        ));
        let storage = crate::db::connect(&format!("sqlite://{}", path.display()), 5)
            .await
            .unwrap();
        crate::db::migrate(&storage).await.unwrap();
        let visible_project_id = uuid::Uuid::new_v4();
        let hidden_project_id = uuid::Uuid::new_v4();
        for (id, slug) in [
            (visible_project_id, "visible-project"),
            (hidden_project_id, "hidden-project"),
        ] {
            crate::db::create_project(
                &storage,
                crate::db::NewProject {
                    id,
                    owner_user_id: Some("owner"),
                    slug,
                    name: slug,
                    description: None,
                    gateway_id: "shared-gateway",
                    binding_ids: &[],
                },
            )
            .await
            .unwrap();
        }
        crate::db::create_profile(
            &storage,
            uuid::Uuid::new_v4(),
            visible_project_id,
            "visible-agent",
            "Visible agent",
            None,
        )
        .await
        .unwrap();
        crate::db::create_profile(
            &storage,
            uuid::Uuid::new_v4(),
            hidden_project_id,
            "hidden-agent",
            "Hidden agent",
            None,
        )
        .await
        .unwrap();
        let state =
            crate::state_with_storage(crate::config::Config::from_env().unwrap(), storage.clone());
        let scope = project_scope([(visible_project_id, "visible-project".to_string())]);
        let agents = scope
            .gateway_agents(
                &state,
                "shared-gateway",
                vec![
                    AgentDescriptor {
                        id: "visible-agent".to_string(),
                        name: "Visible agent".to_string(),
                        metadata: serde_json::json!({}),
                    },
                    AgentDescriptor {
                        id: "hidden-agent".to_string(),
                        name: "Hidden agent".to_string(),
                        metadata: serde_json::json!({}),
                    },
                ],
            )
            .await
            .unwrap();

        assert_eq!(
            agents
                .iter()
                .map(|agent| agent.id.as_str())
                .collect::<Vec<_>>(),
            vec!["visible-agent"]
        );
        match storage {
            crate::db::Storage::Sqlite(pool) => pool.close().await,
            crate::db::Storage::Postgres(_) => unreachable!(),
        }
        std::fs::remove_file(path).unwrap();
    }

    #[tokio::test]
    async fn project_monitor_allows_every_gateway_assigned_to_the_deployment() {
        let path = std::env::temp_dir().join(format!(
            "vifu-monitor-multi-gateway-{}.sqlite",
            uuid::Uuid::new_v4().simple()
        ));
        let storage = crate::db::connect(&format!("sqlite://{}", path.display()), 5)
            .await
            .unwrap();
        crate::db::migrate(&storage).await.unwrap();
        let project_id = uuid::Uuid::new_v4();
        crate::db::create_project(
            &storage,
            crate::db::NewProject {
                id: project_id,
                owner_user_id: Some("owner"),
                slug: "multi-device-project",
                name: "Multi-device project",
                description: None,
                gateway_id: "phone-a",
                binding_ids: &[],
            },
        )
        .await
        .unwrap();
        let deployment = crate::db::get_runtime_deployment(&storage, project_id, "development")
            .await
            .unwrap();
        crate::db::assign_runtime_deployment_gateway(
            &storage,
            project_id,
            deployment.id,
            "phone-b",
        )
        .await
        .unwrap();
        let state =
            crate::state_with_storage(crate::config::Config::from_env().unwrap(), storage.clone());
        let scope = project_scope([(project_id, "multi-device-project".to_string())]);

        assert!(scope.allows_gateway(&state, "phone-a").await);
        assert!(scope.allows_gateway(&state, "phone-b").await);

        match storage {
            crate::db::Storage::Sqlite(pool) => pool.close().await,
            crate::db::Storage::Postgres(_) => unreachable!(),
        }
        std::fs::remove_file(path).unwrap();
    }

    #[tokio::test]
    async fn project_monitor_filters_by_physical_resource_instead_of_profile_slug() {
        let path = std::env::temp_dir().join(format!(
            "vifu-monitor-resource-{}.sqlite",
            uuid::Uuid::new_v4().simple()
        ));
        let storage = crate::db::connect(&format!("sqlite://{}", path.display()), 5)
            .await
            .unwrap();
        crate::db::migrate(&storage).await.unwrap();
        let project_id = uuid::Uuid::new_v4();
        crate::db::create_project(
            &storage,
            crate::db::NewProject {
                id: project_id,
                owner_user_id: Some("owner"),
                slug: "logical-profile-project",
                name: "Logical profile project",
                description: None,
                gateway_id: "android-phone",
                binding_ids: &[],
            },
        )
        .await
        .unwrap();
        let profile_id = uuid::Uuid::new_v4();
        crate::db::create_profile(
            &storage,
            profile_id,
            project_id,
            "friendly-companion",
            "Friendly companion",
            None,
        )
        .await
        .unwrap();
        let empty = serde_json::json!({});
        let source = serde_json::json!({
            "providerKey": "android-local",
            "resourceId": "physical-chat-agent"
        });
        let capabilities = vec![crate::models::ProfileCapabilityDraft {
            kind: "chat".to_string(),
            provider_type: "embedded".to_string(),
            provider_key: "android-local".to_string(),
            resource_id: Some("physical-chat-agent".to_string()),
            config: serde_json::json!({}),
            input_schema: serde_json::json!({}),
            output_schema: serde_json::json!({}),
        }];
        crate::db::create_profile_version(
            &storage,
            profile_id,
            crate::db::NewProfileVersion {
                persona: &empty,
                runtime: &empty,
                presentation: &empty,
                source: &source,
                capabilities: &capabilities,
                change_summary: None,
            },
        )
        .await
        .unwrap();
        let state =
            crate::state_with_storage(crate::config::Config::from_env().unwrap(), storage.clone());
        let scope = project_scope([(project_id, "logical-profile-project".to_string())]);
        let agents = scope
            .gateway_agents(
                &state,
                "android-phone",
                vec![
                    AgentDescriptor {
                        id: "physical-chat-agent".to_string(),
                        name: "Android Local Companion".to_string(),
                        metadata: serde_json::json!({"providerKey": "android-local"}),
                    },
                    AgentDescriptor {
                        id: "unrelated-agent".to_string(),
                        name: "Unrelated".to_string(),
                        metadata: serde_json::json!({"providerKey": "android-local"}),
                    },
                ],
            )
            .await
            .unwrap();

        assert_eq!(agents.len(), 1);
        assert_eq!(agents[0].id, "physical-chat-agent");
        assert_eq!(agents[0].name, "Android Local Companion");

        match storage {
            crate::db::Storage::Sqlite(pool) => pool.close().await,
            crate::db::Storage::Postgres(_) => unreachable!(),
        }
        std::fs::remove_file(path).unwrap();
    }

    #[tokio::test]
    async fn live_gateway_server_monitor_reconnects_with_persisted_session() {
        let files = LiveTestFiles::new("gateway-monitor");
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let config = live_test_config(addr, &files);
        let bootstrap_token = config.agent_gateway_bootstrap_token.clone();
        let storage = live_test_storage(&files).await;
        let state = crate::state_with_storage(config, storage.clone());
        let server_state = state.clone();
        let server = tokio::spawn(async move {
            axum::serve(listener, crate::app(server_state))
                .await
                .unwrap();
        });
        let server_url = format!("http://{addr}");
        let mut monitor = RemoteMonitorClient::connect(&server_url, "synthetic-topology-admin")
            .await
            .unwrap();
        assert_eq!(
            monitor.next_event().await.unwrap(),
            Some(ServerMonitorEvent::Snapshot {
                gateways: Vec::new()
            })
        );

        let first_connected = Arc::new(Notify::new());
        let mut first_gateway = spawn_live_gateway(
            server_url.clone(),
            bootstrap_token.clone(),
            files.session.clone(),
            files.runtime_database.clone(),
            Arc::clone(&first_connected),
        );
        tokio::select! {
            _ = tokio::time::sleep(Duration::from_secs(8)) => {
                panic!("Agent Gateway should connect to the live Server");
            }
            _ = first_connected.notified() => {}
            result = &mut first_gateway => {
                panic!("Agent Gateway exited before connecting: {result:?}");
            }
        }
        let (gateway_id, agents) = next_gateway_event(&mut monitor, true).await;
        assert_eq!(agents.len(), 1);
        assert_eq!(agents[0].id, "topology-agent");
        wait_for_session(&files.session).await;

        first_gateway.abort();
        let (disconnected_gateway_id, _) = next_gateway_event(&mut monitor, false).await;
        assert_eq!(disconnected_gateway_id, gateway_id);

        let second_connected = Arc::new(Notify::new());
        let mut second_gateway = spawn_live_gateway(
            server_url,
            bootstrap_token,
            files.session.clone(),
            files.runtime_database.clone(),
            Arc::clone(&second_connected),
        );
        tokio::select! {
            _ = tokio::time::sleep(Duration::from_secs(8)) => {
                panic!("Agent Gateway should reconnect from its persisted session");
            }
            _ = second_connected.notified() => {}
            result = &mut second_gateway => {
                panic!("Agent Gateway exited before reconnecting: {result:?}");
            }
        }
        let (reconnected_gateway_id, agents) = next_gateway_event(&mut monitor, true).await;
        assert_eq!(reconnected_gateway_id, gateway_id);
        assert_eq!(agents[0].id, "topology-agent");

        second_gateway.abort();
        server.abort();
        drop(monitor);
        drop(state);
        close_storage(storage).await;
    }

    #[tokio::test]
    async fn live_project_monitor_scope_filters_shared_gateway() {
        let files = LiveTestFiles::new("project-scope");
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let config = live_test_config(addr, &files);
        let storage = live_test_storage(&files).await;
        let visible_project_id = Uuid::new_v4();
        let hidden_project_id = Uuid::new_v4();
        for (id, slug) in [
            (visible_project_id, "live-visible-project"),
            (hidden_project_id, "live-hidden-project"),
        ] {
            crate::db::create_project(
                &storage,
                crate::db::NewProject {
                    id,
                    owner_user_id: Some("synthetic-owner"),
                    slug,
                    name: slug,
                    description: None,
                    gateway_id: "live-shared-gateway",
                    binding_ids: &[],
                },
            )
            .await
            .unwrap();
        }
        crate::db::create_profile(
            &storage,
            Uuid::new_v4(),
            visible_project_id,
            "live-visible-agent",
            "Visible agent",
            None,
        )
        .await
        .unwrap();
        crate::db::create_profile(
            &storage,
            Uuid::new_v4(),
            hidden_project_id,
            "live-hidden-agent",
            "Hidden agent",
            None,
        )
        .await
        .unwrap();
        let raw_key = format!("vifu_pk_{}", "7".repeat(64));
        let key_hash = crate::auth::hash_api_key(&raw_key, &config.api_key_pepper);
        let permissions = crate::models::ApiKeyPermissions {
            project: crate::models::ResourcePermission::Read,
            ..crate::models::ApiKeyPermissions::default()
        };
        crate::db::create_api_key(
            &storage,
            crate::db::NewApiKey {
                id: Uuid::new_v4(),
                project_id: visible_project_id,
                name: "Synthetic topology monitor",
                agent_scope: &crate::models::ApiKeyAgentScope::All,
                permissions: &permissions,
                key_prefix: &raw_key.chars().take(20).collect::<String>(),
                key_hash: &key_hash,
            },
        )
        .await
        .unwrap();
        let state = crate::state_with_storage(config, storage.clone());
        state.monitor.publish(ServerMonitorEvent::GatewayConnected {
            gateway_id: "live-shared-gateway".to_string(),
            agents: vec![
                AgentDescriptor {
                    id: "live-visible-agent".to_string(),
                    name: "Visible agent".to_string(),
                    metadata: serde_json::json!({}),
                },
                AgentDescriptor {
                    id: "live-hidden-agent".to_string(),
                    name: "Hidden agent".to_string(),
                    metadata: serde_json::json!({}),
                },
            ],
        });
        let server_state = state.clone();
        let server = tokio::spawn(async move {
            axum::serve(listener, crate::app(server_state))
                .await
                .unwrap();
        });

        let server_url = format!("http://{addr}");
        let mut monitor = RemoteMonitorClient::connect(&server_url, &raw_key)
            .await
            .unwrap();
        let Some(ServerMonitorEvent::Snapshot { gateways }) = monitor.next_event().await.unwrap()
        else {
            panic!("project monitor should receive a scoped snapshot");
        };
        assert_eq!(gateways.len(), 1);
        assert_eq!(gateways[0].gateway_id, "live-shared-gateway");
        assert_eq!(gateways[0].agents.len(), 1);
        assert_eq!(gateways[0].agents[0].id, "live-visible-agent");
        assert!(
            RemoteMonitorClient::connect(&server_url, "synthetic-invalid-key")
                .await
                .is_err()
        );

        server.abort();
        drop(monitor);
        drop(state);
        close_storage(storage).await;
    }
}
