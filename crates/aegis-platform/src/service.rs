//! Platform service facade: identity, zero-Core status, and SAFE-bound effects.

use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use aegis_contracts::ipc::{
    IpcMethod, IPC_PROTOCOL_VERSION, METHOD_PLATFORM_EFFECT_PING, METHOD_PLATFORM_SHUTDOWN,
    METHOD_PLATFORM_STATUS, METHOD_SAFE_STOP_CLEAR, METHOD_SAFE_STOP_SET, METHOD_SAFE_STOP_STATUS,
};
use aegis_contracts::status::{
    EffectPingResult, PlatformStatus, StopMutationResult, PLATFORM_STATUS_SCHEMA,
    STOP_MUTATION_SCHEMA,
};
use aegis_modules::{ModuleError, ModuleRegistry};
use aegis_safe::{SafeError, SafeStore};
use aegis_storage::StoragePaths;
use serde_json::{json, Value};

use crate::store::{PlatformIdentity, PlatformStore};
use crate::PlatformError;

#[derive(Debug, thiserror::Error)]
pub enum ServiceError {
    #[error("Platform operation failed: {0}")]
    Platform(#[from] PlatformError),
    #[error("Safe operation failed: {0}")]
    Safe(#[from] SafeError),
    #[error("storage operation failed: {0}")]
    Storage(#[from] aegis_storage::StorageError),
    #[error("JSON encoding failed: {0}")]
    Json(#[from] serde_json::Error),
    #[error("owner authorization failed")]
    Unauthorized,
    #[error("operator STOP is active")]
    StopActive,
    #[error("invalid operation parameters: {0}")]
    InvalidParams(&'static str),
    #[error("operation is not supported")]
    UnsupportedMethod,
    #[error("owner proof has not been initialized")]
    OwnerNotInitialized,
    #[error("module registry initialization failed: {0}")]
    Modules(#[from] ModuleError),
}

impl ServiceError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::Unauthorized => "unauthorized",
            Self::StopActive => "stop_active",
            Self::InvalidParams(_) => "invalid_request",
            Self::UnsupportedMethod => "unsupported_method",
            Self::OwnerNotInitialized => "owner_not_initialized",
            Self::Platform(_)
            | Self::Safe(_)
            | Self::Storage(_)
            | Self::Json(_)
            | Self::Modules(_) => "internal",
        }
    }
}

pub struct InitializationResult {
    pub platform_id: String,
    pub callsign: String,
    pub owner_token: String,
}

#[derive(Clone)]
pub struct PlatformService {
    paths: StoragePaths,
    store: PlatformStore,
    safe: SafeStore,
    shutdown: Arc<AtomicBool>,
}

impl PlatformService {
    pub fn initialize(root: impl AsRef<Path>) -> Result<InitializationResult, ServiceError> {
        let paths = StoragePaths::new(root)?;
        let store = PlatformStore::new(paths.clone());
        let identity = store.initialize()?;
        ModuleRegistry::new(paths.clone()).initialize()?;
        let safe = SafeStore::new(paths);
        safe.initialize()?;
        let owner_token = match safe.owner_token() {
            Ok(token) => token,
            Err(SafeError::OwnerNotInitialized) => safe.initialize_owner()?,
            Err(error) => return Err(error.into()),
        };
        Ok(InitializationResult {
            platform_id: identity.platform_id,
            callsign: identity.callsign,
            owner_token,
        })
    }

    pub fn start(root: impl AsRef<Path>) -> Result<Self, ServiceError> {
        let paths = StoragePaths::new(root)?;
        let store = PlatformStore::new(paths.clone());
        store.initialize()?;
        ModuleRegistry::new(paths.clone()).initialize()?;
        let safe = SafeStore::new(paths.clone());
        safe.start()?;
        Ok(Self {
            paths,
            store,
            safe,
            shutdown: Arc::new(AtomicBool::new(false)),
        })
    }

    pub fn identity(&self) -> Result<PlatformIdentity, ServiceError> {
        Ok(self.store.identity()?)
    }

    pub fn status(&self) -> Result<PlatformStatus, ServiceError> {
        let identity = self.store.identity()?;
        let core_count = self.store.core_count()?;
        let stop = self.safe.stop_status()?;
        Ok(PlatformStatus {
            schema: PLATFORM_STATUS_SCHEMA.to_owned(),
            protocol: IPC_PROTOCOL_VERSION.to_owned(),
            platform_id: identity.platform_id,
            callsign: identity.callsign,
            zero_core: core_count == 0,
            core_count,
            stop_active: stop.active,
            authority: "local_single".to_owned(),
            generated_at: aegis_storage::now_rfc3339()?,
        })
    }

    pub fn owner_token(&self) -> Result<String, ServiceError> {
        self.safe.owner_token().map_err(ServiceError::from)
    }

    pub fn dispatch(
        &self,
        method: &str,
        params: &Value,
        owner_token: &str,
    ) -> Result<Value, ServiceError> {
        if !self.safe.verify_owner(owner_token)? {
            return Err(ServiceError::Unauthorized);
        }
        match method {
            METHOD_PLATFORM_STATUS => Ok(serde_json::to_value(self.status()?)?),
            METHOD_PLATFORM_SHUTDOWN => {
                self.shutdown.store(true, Ordering::SeqCst);
                Ok(json!({"ok": true, "shutdown": true}))
            }
            METHOD_PLATFORM_EFFECT_PING => {
                if self.safe.stop_status()?.active {
                    return Err(ServiceError::StopActive);
                }
                Ok(serde_json::to_value(EffectPingResult::ok())?)
            }
            METHOD_SAFE_STOP_STATUS => Ok(serde_json::to_value(self.safe.stop_status()?)?),
            METHOD_SAFE_STOP_SET => {
                let reason = params
                    .get("reason")
                    .and_then(Value::as_str)
                    .ok_or(ServiceError::InvalidParams("reason"))?;
                let updated_by = params
                    .get("updated_by")
                    .and_then(Value::as_str)
                    .unwrap_or("owner");
                let status = self.safe.set_stop(updated_by, reason, owner_token)?;
                Ok(serde_json::to_value(StopMutationResult {
                    schema: STOP_MUTATION_SCHEMA.to_owned(),
                    active: status.active,
                    revision: status.revision,
                    updated_at: status.updated_at,
                })?)
            }
            METHOD_SAFE_STOP_CLEAR => {
                let updated_by = params
                    .get("updated_by")
                    .and_then(Value::as_str)
                    .unwrap_or("owner");
                let status = self.safe.clear_stop(updated_by, owner_token)?;
                Ok(serde_json::to_value(StopMutationResult {
                    schema: STOP_MUTATION_SCHEMA.to_owned(),
                    active: status.active,
                    revision: status.revision,
                    updated_at: status.updated_at,
                })?)
            }
            _ => {
                if IpcMethod::parse(method).is_none() {
                    return Err(ServiceError::UnsupportedMethod);
                }
                Err(ServiceError::UnsupportedMethod)
            }
        }
    }

    pub fn set_stop(&self, reason: &str) -> Result<StopMutationResult, ServiceError> {
        let token = self.owner_token()?;
        let value = self.dispatch(METHOD_SAFE_STOP_SET, &json!({"reason": reason}), &token)?;
        serde_json::from_value(value).map_err(ServiceError::from)
    }

    pub fn clear_stop(&self) -> Result<StopMutationResult, ServiceError> {
        let token = self.owner_token()?;
        let value = self.dispatch(METHOD_SAFE_STOP_CLEAR, &json!({}), &token)?;
        serde_json::from_value(value).map_err(ServiceError::from)
    }

    pub fn stop_status(&self) -> Result<aegis_contracts::status::StopStatus, ServiceError> {
        let token = self.owner_token()?;
        let value = self.dispatch(METHOD_SAFE_STOP_STATUS, &json!({}), &token)?;
        serde_json::from_value(value).map_err(ServiceError::from)
    }

    pub fn effect_ping(&self) -> Result<EffectPingResult, ServiceError> {
        let token = self.owner_token()?;
        let value = self.dispatch(METHOD_PLATFORM_EFFECT_PING, &json!({}), &token)?;
        serde_json::from_value(value).map_err(ServiceError::from)
    }

    pub fn paths(&self) -> &StoragePaths {
        &self.paths
    }

    /// Request a graceful shutdown of a serving Platform process.
    pub fn request_shutdown(&self) {
        self.shutdown.store(true, Ordering::SeqCst);
    }

    /// Whether a graceful shutdown has been requested.
    pub fn shutdown_requested(&self) -> bool {
        self.shutdown.load(Ordering::SeqCst)
    }
}
