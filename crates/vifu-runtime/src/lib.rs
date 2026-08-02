//! Embeddable, stateful Agent Runtime primitives for Rust applications.
//!
//! Add application behavior as Bevy plugins, dispatch [`RuntimeCommand`] values,
//! and let the host execute the resulting effects.

mod application;
pub mod bridge;
mod manifest;
pub mod protocol;
pub mod providers;
mod runtime;
#[cfg(feature = "sqlite")]
mod sqlite_store;

pub use application::{
    AgentDefinition, AgentProvider, CancellationToken, EffectExecution, EndpointDefinition,
    InvocationData, InvocationEvent, InvocationEventKind, InvocationHandle, InvocationInput,
    InvocationOutput, InvocationPoll, InvocationStatus, InvocationTraceEvent, MemoryRuntimeStore,
    ProviderEvent, ProviderEventSink, ProviderFuture, ProviderRequest, ProviderResponse,
    ProviderStage, RuntimeError, RuntimeSession, RuntimeStore, VifuRuntime,
};
pub use bridge::{
    RuntimeBridge, RuntimeBridgeCancelParams, RuntimeBridgeError, RuntimeBridgeHelloParams,
    RuntimeBridgeHelloPayload, RuntimeBridgeInvocationEvent, RuntimeBridgeInvokePayload,
    RUNTIME_BRIDGE_CANCELLED_EVENT, RUNTIME_BRIDGE_CANCEL_METHOD, RUNTIME_BRIDGE_COMPLETED_EVENT,
    RUNTIME_BRIDGE_FAILED_EVENT, RUNTIME_BRIDGE_HELLO_METHOD, RUNTIME_BRIDGE_INVOKE_METHOD,
    RUNTIME_BRIDGE_OUTPUT_DELTA_EVENT, RUNTIME_BRIDGE_STARTED_EVENT,
    VIFU_RUNTIME_BRIDGE_PROTOCOL_VERSION,
};
pub use manifest::{
    LocalProviderBinding, ProjectSettings, ProviderRequirement, RuntimeManifest, RuntimeRelease,
    RuntimeTraceRecord, RUNTIME_MANIFEST_SCHEMA_VERSION,
};
pub use protocol::{
    decode_protocol_frame, encode_protocol_frame, ErrorShape, EventFrame, EventFrameType,
    ProtocolFrame, RequestFrame, RequestFrameType, ResponseFrame, ResponseFrameType, StateVersion,
    MAX_PROTOCOL_FRAME_BYTES,
};
pub use providers::{HttpCapabilityProvider, HttpCapabilityRoute};
pub use runtime::{
    EffectRequest, EffectRequestQueue, EffectResult, EffectResultQueue, HeadlessRuntime,
    RuntimeAdvance, RuntimeCommand, RuntimeCommandQueue, RuntimeEvent, RuntimeEventQueue,
    RuntimeSchedule, RuntimeSnapshot, RuntimeState, VifuRuntimePlugin,
};
#[cfg(feature = "sqlite")]
pub use sqlite_store::SqliteRuntimeStore;

pub(crate) fn unix_time_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |duration| {
            u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
        })
}

pub mod prelude {
    //! Common imports for application runtime plugins.

    pub use bevy_app::{App, Plugin};
    pub use bevy_ecs::prelude::*;
    pub use serde_json::{json, Value};

    pub use crate::{
        AgentDefinition, AgentProvider, CancellationToken, EffectExecution, EffectRequest,
        EffectRequestQueue, EffectResult, EffectResultQueue, EndpointDefinition, HeadlessRuntime,
        HttpCapabilityProvider, HttpCapabilityRoute, InvocationData, InvocationEvent,
        InvocationEventKind, InvocationHandle, InvocationInput, InvocationOutput, InvocationPoll,
        InvocationStatus, InvocationTraceEvent, MemoryRuntimeStore, ProjectSettings, ProviderEvent,
        ProviderEventSink, ProviderFuture, ProviderRequest, ProviderRequirement, ProviderResponse,
        ProviderStage, RuntimeAdvance, RuntimeBridge, RuntimeBridgeError, RuntimeCommand,
        RuntimeCommandQueue, RuntimeError, RuntimeEvent, RuntimeEventQueue, RuntimeManifest,
        RuntimeRelease, RuntimeSchedule, RuntimeSession, RuntimeSnapshot, RuntimeState,
        RuntimeStore, RuntimeTraceRecord, VifuRuntime, VifuRuntimePlugin,
    };
}
