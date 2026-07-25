//! Small, stateful runtime primitives for embedding Vifu in Rust applications.
//!
//! Add application behavior as Bevy plugins, dispatch [`RuntimeCommand`] values,
//! and let the host execute the resulting effects.

mod runtime;

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
        EffectRequest, EffectRequestQueue, EffectResult, EffectResultQueue, HeadlessRuntime,
        RuntimeAdvance, RuntimeCommand, RuntimeCommandQueue, RuntimeEvent, RuntimeEventQueue,
        RuntimeSchedule, RuntimeSnapshot, RuntimeState, VifuRuntimePlugin,
    };
}
