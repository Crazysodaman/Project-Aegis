//! Wave 1b Core initialization orchestration.
//!
//! This crate composes identity, per-Core storage, integrity, and the Platform
//! registry. It does not own any of those domains.

use std::path::Path;

use aegis_contracts::core::{CoreIdentity, CoreStorageMetadata};
use aegis_contracts::initialization::{
    CoreInitializationRequest, InitializationCheckpoint, InitializationFailureCode,
    InitializationStage, CORE_INIT_CHECKPOINT_SCHEMA,
};
use aegis_contracts::integrity::{IntegrityReport, IntegrityStatus};
use aegis_contracts::roster::{CoreRegistrationRequest, CoreRegistryEntry};
use aegis_core_identity::{
    read_checkpoint, recover_decision, validate_initialization_request, write_checkpoint,
    InitializationStateMachine, RecoveryDecision,
};
use aegis_core_integrity::{seal_manifest_with_metadata, verify_core, IntegrityError};
use aegis_core_registry::{CoreRegistry, RegistryError};
use aegis_core_storage::{CoreDatabase, CoreStorageError, CoreStorageLayout};
use aegis_safe::{SafeError, SafeStore};
use aegis_storage::{now_rfc3339, StorageError, StoragePaths};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum InstallError {
    #[error("Core identity or initialization state failed: {0}")]
    Identity(#[from] aegis_core_identity::IdentityError),
    #[error("Core storage failed: {0}")]
    Storage(#[from] CoreStorageError),
    #[error("Core integrity failed: {0}")]
    Integrity(#[from] IntegrityError),
    #[error("Core registry failed: {0}")]
    Registry(#[from] RegistryError),
    #[error("storage paths failed: {0}")]
    Paths(#[from] StorageError),
    #[error("Safe authorization failed: {0}")]
    Safe(#[from] SafeError),
    #[error("owner authorization failed")]
    Unauthorized,
    #[error("Core is already initialized")]
    AlreadyInitialized,
    #[error("Core initialization was interrupted; explicit cleanup is required")]
    Interrupted,
    #[error("an unregistered Core database already exists; refusing silent adoption")]
    ExistingUnregisteredDatabase,
    #[error("Core integrity verification failed: {0:?}")]
    VerificationFailed(IntegrityStatus),
}

impl InstallError {
    pub fn failure_code(&self) -> InitializationFailureCode {
        match self {
            Self::Identity(error) => error.failure_code(),
            Self::Storage(CoreStorageError::ForeignOwnership) => {
                InitializationFailureCode::ForeignOwnership
            }
            Self::Storage(CoreStorageError::PartialState) | Self::ExistingUnregisteredDatabase => {
                InitializationFailureCode::AlreadyExists
            }
            Self::Storage(_) | Self::Paths(_) => InitializationFailureCode::StorageRejected,
            Self::Safe(_) | Self::Unauthorized => InitializationFailureCode::Internal,
            Self::Integrity(_) | Self::VerificationFailed(_) => {
                InitializationFailureCode::IntegrityFailure
            }
            Self::Registry(_) => InitializationFailureCode::RegistrationFailure,
            Self::AlreadyInitialized => InitializationFailureCode::AlreadyExists,
            Self::Interrupted => InitializationFailureCode::Interrupted,
        }
    }
}

#[derive(Debug, Clone)]
pub struct CoreInstallation {
    pub identity: CoreIdentity,
    pub metadata: CoreStorageMetadata,
    pub registry_entry: CoreRegistryEntry,
    pub integrity: IntegrityReport,
}

#[derive(Debug, Clone)]
pub struct CoreInspection {
    pub identity: CoreIdentity,
    pub metadata: CoreStorageMetadata,
    pub registry_entry: Option<CoreRegistryEntry>,
    pub integrity: IntegrityReport,
}

#[derive(Debug, Clone)]
pub struct CoreInstaller {
    paths: StoragePaths,
    layout: CoreStorageLayout,
    registry: CoreRegistry,
    safe: SafeStore,
}

impl CoreInstaller {
    pub fn new(root: impl AsRef<Path>) -> Result<Self, InstallError> {
        let paths = StoragePaths::new(root)?;
        let layout = CoreStorageLayout::new(paths.root())?;
        let registry = CoreRegistry::new(paths.clone());
        let safe = SafeStore::new(paths.clone());
        Ok(Self {
            paths,
            layout,
            registry,
            safe,
        })
    }

    pub fn paths(&self) -> &StoragePaths {
        &self.paths
    }

    pub fn layout(&self) -> &CoreStorageLayout {
        &self.layout
    }

    pub fn initialize(
        &self,
        request: &CoreInitializationRequest,
        owner_token: &str,
    ) -> Result<CoreInstallation, InstallError> {
        self.authorize_owner(owner_token)?;
        let identity = validate_initialization_request(request)?;
        self.registry.initialize()?;
        let core_uuid = identity.core_uuid.clone();
        let marker = self.layout.initialization_marker_path(&core_uuid)?;
        let checkpoint = read_checkpoint(&marker)?;

        if let Some(checkpoint) = checkpoint.as_ref() {
            match recover_decision(checkpoint) {
                RecoveryDecision::Complete { .. } => return Err(InstallError::AlreadyInitialized),
                RecoveryDecision::Interrupted { .. } => return Err(InstallError::Interrupted),
                RecoveryDecision::Failed { .. } => {}
            }
        }

        if self.registry.get(&core_uuid)?.is_some() {
            return Err(InstallError::AlreadyInitialized);
        }

        let database_path = self.layout.database_path(&core_uuid)?;
        if database_path.exists() && checkpoint.is_none() {
            return Err(InstallError::ExistingUnregisteredDatabase);
        }

        let mut state = InitializationStateMachine::start(checkpoint.as_ref())?;
        self.checkpoint(&marker, &state, &core_uuid)?;

        let result = self.run_initialization(&identity, &mut state, &marker, &database_path);
        match result {
            Ok(installation) => Ok(installation),
            Err(error) => {
                let _ = self.mark_failed(&marker, &core_uuid, error.failure_code());
                Err(error)
            }
        }
    }

    fn run_initialization(
        &self,
        identity: &CoreIdentity,
        state: &mut InitializationStateMachine,
        marker: &Path,
        database_path: &Path,
    ) -> Result<CoreInstallation, InstallError> {
        let database = CoreDatabase::create_fresh(&self.layout, identity)?;
        state.advance_to(InitializationStage::StoragePrepared)?;
        self.checkpoint(marker, state, &identity.core_uuid)?;
        state.advance_to(InitializationStage::SchemaApplied)?;
        self.checkpoint(marker, state, &identity.core_uuid)?;

        let stored_identity = database.identity()?;
        if stored_identity.core_uuid != identity.core_uuid
            || stored_identity.core_number != identity.core_number
            || stored_identity.display_name != identity.display_name
        {
            return Err(InstallError::AlreadyInitialized);
        }
        let identity = &stored_identity;
        state.advance_to(InitializationStage::IdentityWritten)?;
        self.checkpoint(marker, state, &identity.core_uuid)?;

        let metadata = database.metadata()?;
        database.verify_sqlite_integrity()?;
        database.close()?;

        let manifest_path = self.layout.integrity_path(&identity.core_uuid)?;
        seal_manifest_with_metadata(
            &identity.core_uuid,
            database_path,
            &manifest_path,
            metadata.schema_revision,
            identity,
            Some(&metadata.database_instance_uuid),
            Some(&metadata.initialization_mode),
            Some(metadata.core_number),
        )?;
        state.advance_to(InitializationStage::IntegritySealed)?;
        self.checkpoint(marker, state, &identity.core_uuid)?;

        let report = verify_core(
            &identity.core_uuid,
            database_path,
            &manifest_path,
            metadata.schema_revision,
            Some(identity),
        );
        if report.status != IntegrityStatus::Verified {
            return Err(InstallError::VerificationFailed(report.status));
        }

        let storage_relative_path = format!("cores/{}/core.db", identity.core_uuid);
        let registry_entry = self.registry.register(CoreRegistrationRequest {
            schema: aegis_contracts::roster::CORE_REGISTRY_SCHEMA.to_owned(),
            core_uuid: identity.core_uuid.clone(),
            core_number: identity.core_number,
            display_name: identity.display_name.clone(),
            storage_relative_path,
            initialized_at: identity.created_at.clone(),
        })?;
        state.advance_to(InitializationStage::Registered)?;
        self.checkpoint(marker, state, &identity.core_uuid)?;

        Ok(CoreInstallation {
            identity: stored_identity,
            metadata,
            registry_entry,
            integrity: report,
        })
    }

    pub fn inspect(&self, core_uuid: &str) -> Result<CoreInspection, InstallError> {
        let snapshot = CoreDatabase::read_snapshot(&self.layout, core_uuid)?;
        let registry_entry = self.registry.get(core_uuid)?;
        let report = verify_core(
            core_uuid,
            &snapshot.database_path,
            &self.layout.integrity_path(core_uuid)?,
            snapshot.metadata.schema_revision,
            Some(&snapshot.identity),
        );
        Ok(CoreInspection {
            identity: snapshot.identity,
            metadata: snapshot.metadata,
            registry_entry,
            integrity: report,
        })
    }

    pub fn verify(&self, core_uuid: &str) -> Result<IntegrityReport, InstallError> {
        let inspection = self.inspect(core_uuid)?;
        if inspection.integrity.status != IntegrityStatus::Verified {
            return Err(InstallError::VerificationFailed(
                inspection.integrity.status,
            ));
        }
        Ok(inspection.integrity)
    }

    pub fn rename_display_name(
        &self,
        core_uuid: &str,
        new_name: &str,
        owner_token: &str,
    ) -> Result<CoreInstallation, InstallError> {
        self.authorize_owner(owner_token)?;
        let database = CoreDatabase::open(&self.layout, core_uuid)?;
        database.rename_display_name(new_name)?;
        let identity = database.identity()?;
        let metadata = database.metadata()?;
        database.close()?;

        let manifest_path = self.layout.integrity_path(core_uuid)?;
        seal_manifest_with_metadata(
            core_uuid,
            &self.layout.database_path(core_uuid)?,
            &manifest_path,
            metadata.schema_revision,
            &identity,
            Some(&metadata.database_instance_uuid),
            Some(&metadata.initialization_mode),
            Some(metadata.core_number),
        )?;
        let report = verify_core(
            core_uuid,
            &self.layout.database_path(core_uuid)?,
            &manifest_path,
            metadata.schema_revision,
            Some(&identity),
        );
        if report.status != IntegrityStatus::Verified {
            return Err(InstallError::VerificationFailed(report.status));
        }
        let registry_entry = self.registry.update_display_name(core_uuid, new_name)?;
        Ok(CoreInstallation {
            identity,
            metadata,
            registry_entry,
            integrity: report,
        })
    }

    pub fn deactivate(
        &self,
        core_uuid: &str,
        reason: &str,
        owner_token: &str,
    ) -> Result<CoreRegistryEntry, InstallError> {
        self.authorize_owner(owner_token)?;
        Ok(self.registry.deactivate(core_uuid, reason)?)
    }

    pub fn reactivate(
        &self,
        core_uuid: &str,
        reason: &str,
        owner_token: &str,
    ) -> Result<CoreRegistryEntry, InstallError> {
        self.authorize_owner(owner_token)?;
        Ok(self.registry.reactivate(core_uuid, reason)?)
    }

    pub fn uninstall_preserve(
        &self,
        core_uuid: &str,
        reason: &str,
        owner_token: &str,
    ) -> Result<CoreRegistryEntry, InstallError> {
        self.authorize_owner(owner_token)?;
        Ok(self.registry.uninstall_preserve(core_uuid, reason)?)
    }

    fn authorize_owner(&self, owner_token: &str) -> Result<(), InstallError> {
        if !self.safe.verify_owner(owner_token)? {
            return Err(InstallError::Unauthorized);
        }
        Ok(())
    }

    pub fn list(&self) -> Result<Vec<CoreRegistryEntry>, InstallError> {
        Ok(self.registry.list()?)
    }

    pub fn get(&self, core_uuid: &str) -> Result<Option<CoreRegistryEntry>, InstallError> {
        Ok(self.registry.get(core_uuid)?)
    }

    fn checkpoint(
        &self,
        marker: &Path,
        state: &InitializationStateMachine,
        core_uuid: &str,
    ) -> Result<(), InstallError> {
        write_checkpoint(marker, &state.checkpoint(core_uuid, now_rfc3339()?))?;
        Ok(())
    }

    fn mark_failed(
        &self,
        marker: &Path,
        core_uuid: &str,
        code: InitializationFailureCode,
    ) -> Result<(), InstallError> {
        let checkpoint = InitializationCheckpoint {
            schema: CORE_INIT_CHECKPOINT_SCHEMA.to_owned(),
            core_uuid: core_uuid.to_owned(),
            stage: InitializationStage::Failed,
            updated_at: now_rfc3339()?,
            failure_code: Some(code),
        };
        write_checkpoint(marker, &checkpoint)?;
        Ok(())
    }
}
