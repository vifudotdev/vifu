use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use serde_json::Value;
use tokio::sync::{mpsc, oneshot, Mutex};
use uuid::Uuid;
use vifu_gateway::protocol::AgentGatewayCommand;

use crate::models::EndpointRoute;

#[derive(Clone)]
pub struct RelayHub {
    inner: Arc<Mutex<RelayState>>,
    queue_capacity: usize,
}

struct RelayState {
    connections: HashMap<String, AgentGatewayConnection>,
    pending: HashMap<Uuid, PendingCall>,
    next_channel_id: u64,
}

#[derive(Clone)]
struct AgentGatewayConnection {
    connection_id: Uuid,
    session_id: Uuid,
    sender: mpsc::Sender<AgentGatewayCommand>,
}

struct PendingCall {
    connection_id: Uuid,
    channel_id: u64,
    sender: oneshot::Sender<Result<Value, RelayCallError>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RelayCallError {
    AgentGatewayUnavailable,
    Backpressure,
    Timeout,
    AgentGateway(String),
}

impl RelayHub {
    pub fn new(queue_capacity: usize) -> Self {
        Self {
            inner: Arc::new(Mutex::new(RelayState {
                connections: HashMap::new(),
                pending: HashMap::new(),
                next_channel_id: 1,
            })),
            queue_capacity,
        }
    }

    pub fn channel(
        &self,
    ) -> (
        mpsc::Sender<AgentGatewayCommand>,
        mpsc::Receiver<AgentGatewayCommand>,
    ) {
        mpsc::channel(self.queue_capacity)
    }

    pub async fn register(
        &self,
        gateway_id: String,
        connection_id: Uuid,
        session_id: Uuid,
        sender: mpsc::Sender<AgentGatewayCommand>,
    ) {
        let replaced = {
            let mut state = self.inner.lock().await;
            state.connections.insert(
                gateway_id,
                AgentGatewayConnection {
                    connection_id,
                    session_id,
                    sender,
                },
            )
        };
        if let Some(replaced) = replaced {
            let _ = replaced.sender.try_send(AgentGatewayCommand::Error {
                request_id: None,
                channel_id: None,
                code: "SESSION_REPLACED".to_string(),
                message: "A newer connection replaced this agent gateway session.".to_string(),
            });
        }
    }

    pub async fn unregister(&self, gateway_id: &str, connection_id: Uuid) -> bool {
        let (removed_current, pending) = {
            let mut state = self.inner.lock().await;
            let removed_current = if state
                .connections
                .get(gateway_id)
                .is_some_and(|connection| connection.connection_id == connection_id)
            {
                state.connections.remove(gateway_id);
                true
            } else {
                false
            };

            let request_ids = state
                .pending
                .iter()
                .filter_map(|(request_id, pending)| {
                    (pending.connection_id == connection_id).then_some(*request_id)
                })
                .collect::<Vec<_>>();
            let pending = request_ids
                .into_iter()
                .filter_map(|request_id| state.pending.remove(&request_id))
                .collect::<Vec<_>>();
            (removed_current, pending)
        };
        for pending in pending {
            let _ = pending
                .sender
                .send(Err(RelayCallError::AgentGatewayUnavailable));
        }
        removed_current
    }

    pub async fn session_for(&self, gateway_id: &str) -> Option<Uuid> {
        self.inner
            .lock()
            .await
            .connections
            .get(gateway_id)
            .map(|connection| connection.session_id)
    }

    pub async fn connection_count(&self) -> usize {
        self.inner.lock().await.connections.len()
    }

    pub async fn disconnect(&self, gateway_id: &str, code: &str) -> bool {
        let connection = self.inner.lock().await.connections.get(gateway_id).cloned();
        let Some(connection) = connection else {
            return false;
        };
        connection
            .sender
            .try_send(AgentGatewayCommand::Error {
                request_id: None,
                channel_id: None,
                code: code.to_string(),
                message: "The agent gateway credential is no longer active.".to_string(),
            })
            .is_ok()
    }

    pub async fn invoke(
        &self,
        route: &EndpointRoute,
        request_id: Uuid,
        input: Value,
        timeout: Duration,
    ) -> Result<Value, RelayCallError> {
        let (response_sender, response_receiver) = oneshot::channel();
        let (connection, channel_id) = {
            let mut state = self.inner.lock().await;
            let connection = state
                .connections
                .get(&route.gateway_id)
                .cloned()
                .ok_or(RelayCallError::AgentGatewayUnavailable)?;
            let channel_id = next_channel_id(&mut state);
            state.pending.insert(
                request_id,
                PendingCall {
                    connection_id: connection.connection_id,
                    channel_id,
                    sender: response_sender,
                },
            );
            (connection, channel_id)
        };

        let message = AgentGatewayCommand::Invoke {
            request_id,
            channel_id,
            endpoint_id: route.endpoint_id,
            profile_id: route.profile_id,
            binding_id: route.binding_id,
            agent_id: route.agent_id.clone(),
            binding: route.binding_config.clone(),
            input,
            timeout_ms: timeout.as_millis().try_into().unwrap_or(u64::MAX),
        };

        if let Err(error) = connection.sender.try_send(message) {
            self.remove_pending(request_id).await;
            return match error {
                mpsc::error::TrySendError::Full(_) => Err(RelayCallError::Backpressure),
                mpsc::error::TrySendError::Closed(_) => {
                    Err(RelayCallError::AgentGatewayUnavailable)
                }
            };
        }

        match tokio::time::timeout(timeout, response_receiver).await {
            Ok(Ok(result)) => result,
            Ok(Err(_)) => Err(RelayCallError::AgentGatewayUnavailable),
            Err(_) => {
                self.remove_pending(request_id).await;
                let _ = connection.sender.try_send(AgentGatewayCommand::Cancel {
                    request_id,
                    channel_id,
                });
                Err(RelayCallError::Timeout)
            }
        }
    }

    pub async fn complete_result(
        &self,
        connection_id: Uuid,
        request_id: Uuid,
        channel_id: u64,
        output: Value,
    ) -> bool {
        self.complete(connection_id, request_id, channel_id, Ok(output))
            .await
    }

    pub async fn complete_error(
        &self,
        connection_id: Uuid,
        request_id: Uuid,
        channel_id: u64,
        message: String,
    ) -> bool {
        self.complete(
            connection_id,
            request_id,
            channel_id,
            Err(RelayCallError::AgentGateway(message)),
        )
        .await
    }

    async fn complete(
        &self,
        connection_id: Uuid,
        request_id: Uuid,
        channel_id: u64,
        result: Result<Value, RelayCallError>,
    ) -> bool {
        let pending = {
            let mut state = self.inner.lock().await;
            let matches = state.pending.get(&request_id).is_some_and(|pending| {
                pending.connection_id == connection_id && pending.channel_id == channel_id
            });
            matches.then(|| state.pending.remove(&request_id)).flatten()
        };
        pending.is_some_and(|pending| pending.sender.send(result).is_ok())
    }

    async fn remove_pending(&self, request_id: Uuid) {
        self.inner.lock().await.pending.remove(&request_id);
    }
}

fn next_channel_id(state: &mut RelayState) -> u64 {
    let channel_id = state.next_channel_id.max(1);
    state.next_channel_id = channel_id.wrapping_add(1).max(1);
    channel_id
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use serde_json::json;
    use uuid::Uuid;
    use vifu_gateway::protocol::AgentGatewayCommand;

    use super::{RelayCallError, RelayHub};
    use crate::models::EndpointRoute;

    #[tokio::test]
    async fn multiplexes_ten_concurrent_calls_on_one_connection() {
        let hub = RelayHub::new(16);
        let (sender, mut receiver) = hub.channel();
        let connection_id = Uuid::new_v4();
        hub.register(
            "openclaw-local".to_string(),
            connection_id,
            Uuid::new_v4(),
            sender,
        )
        .await;
        let route = route();

        let calls = (0..10)
            .map(|index| {
                let hub = hub.clone();
                let route = route.clone();
                tokio::spawn(async move {
                    hub.invoke(
                        &route,
                        Uuid::new_v4(),
                        json!({ "index": index }),
                        Duration::from_secs(1),
                    )
                    .await
                })
            })
            .collect::<Vec<_>>();

        for _ in 0..10 {
            let message = receiver.recv().await.unwrap();
            let AgentGatewayCommand::Invoke {
                request_id,
                channel_id,
                input,
                ..
            } = message
            else {
                panic!("expected invoke");
            };
            assert!(channel_id > 0);
            hub.complete_result(connection_id, request_id, channel_id, input)
                .await;
        }

        for call in calls {
            assert!(call.await.unwrap().is_ok());
        }
        assert_eq!(hub.connection_count().await, 1);
    }

    #[tokio::test]
    async fn applies_backpressure_to_a_full_agent_gateway_queue() {
        let hub = RelayHub::new(1);
        let (sender, _receiver) = hub.channel();
        hub.register(
            "openclaw-local".to_string(),
            Uuid::new_v4(),
            Uuid::new_v4(),
            sender,
        )
        .await;

        let first = {
            let hub = hub.clone();
            tokio::spawn(async move {
                hub.invoke(
                    &route(),
                    Uuid::new_v4(),
                    json!({}),
                    Duration::from_millis(500),
                )
                .await
            })
        };
        tokio::task::yield_now().await;
        let second = hub
            .invoke(
                &route(),
                Uuid::new_v4(),
                json!({}),
                Duration::from_millis(500),
            )
            .await;
        assert_eq!(second, Err(RelayCallError::Backpressure));
        first.abort();
    }

    fn route() -> EndpointRoute {
        EndpointRoute {
            endpoint_id: Uuid::new_v4(),
            endpoint_slug: "guide".to_string(),
            endpoint_name: "Guide".to_string(),
            request_timeout_ms: 30_000,
            profile_id: Uuid::new_v4(),
            binding_id: Uuid::new_v4(),
            gateway_id: "openclaw-local".to_string(),
            agent_id: "guide-agent".to_string(),
            binding_config: json!({}),
        }
    }
}
