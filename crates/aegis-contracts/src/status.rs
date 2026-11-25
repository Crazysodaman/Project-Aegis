//! Read-only status contracts.

use serde::{Deserialize, Serialize};

pub const PLATFORM_STATUS_SCHEMA: &str = "aegis.platform.status.v1";
pub const STOP_STATUS_SCHEMA: &str = "aegis.safe.stop_status.v1";
pub const STOP_MUTATION_SCHEMA: &str = "aegis.safe.stop_mutation.v1";
pub const EFFECT_PING_SCHEMA: &str = "aegis.platform.effect_ping.v1";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlatformStatus {
    pub schema: String,
    pub protocol: String,
    pub platform_id: String,
    pub callsign: String,
    pub zero_core: bool,
    pub core_count: u32,
    pub stop_active: bool,
    pub authority: String,
    pub generated_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StopStatus {
    pub schema: String,
    pub active: bool,
    pub revision: u64,
    pub updated_at: String,
    pub updated_by: String,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StopMutationResult {
    pub schema: String,
    pub active: bool,
    pub revision: u64,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EffectPingResult {
    pub schema: String,
    pub ok: bool,
}

impl EffectPingResult {
    pub fn ok() -> Self {
        Self {
            schema: EFFECT_PING_SCHEMA.to_owned(),
            ok: true,
        }
    }
}
