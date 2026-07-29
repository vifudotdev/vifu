//! Small, stateful runtime primitives for embedding Vifu in Rust applications.
//!
//! Add application behavior as Bevy plugins, dispatch [`RuntimeCommand`] values,
//! and let the host execute the resulting effects.

mod application;
pub mod providers;
mod runtime;

pub use application::{
    AgentDefinition, AgentProvider, CancellationToken, EffectExecution, EndpointDefinition,
    InvocationData, InvocationHandle, InvocationInput, InvocationOutput, InvocationPoll,
    InvocationStatus, InvocationTraceEvent, MemoryRuntimeStore, ProviderFuture, ProviderRequest,
    ProviderResponse, RuntimeError, RuntimeSession, RuntimeStore, VifuRuntime,
};
pub use providers::{HttpCapabilityProvider, HttpCapabilityRoute};
pub use runtime::{
    EffectRequest, EffectRequestQueue, EffectResult, EffectResultQueue, HeadlessRuntime,
    RuntimeAdvance, RuntimeCommand, RuntimeCommandQueue, RuntimeEvent, RuntimeEventQueue,
    RuntimeSchedule, RuntimeSnapshot, RuntimeState, VifuRuntimePlugin,
};

pub mod prelude {
    //! Common imports for application runtime plugins.

    pub use bevy_app::{App, Plugin};
    pub use bevy_ecs::prelude::*;
    pub use serde_json::{json, Value};

    pub use crate::{
        AgentDefinition, AgentProvider, CancellationToken, EffectExecution, EffectRequest,
        EffectRequestQueue, EffectResult, EffectResultQueue, EndpointDefinition, HeadlessRuntime,
        HttpCapabilityProvider, HttpCapabilityRoute, InvocationData, InvocationHandle,
        InvocationInput, InvocationOutput, InvocationPoll, InvocationStatus, InvocationTraceEvent,
        MemoryRuntimeStore, ProviderFuture, ProviderRequest, ProviderResponse, RuntimeAdvance,
        RuntimeCommand, RuntimeCommandQueue, RuntimeError, RuntimeEvent, RuntimeEventQueue,
        RuntimeSchedule, RuntimeSession, RuntimeSnapshot, RuntimeState, RuntimeStore, VifuRuntime,
        VifuRuntimePlugin,
    };
}
