//! Owner-authorized module lifecycle facade.
//!
//! [`ModuleRegistry`] is the Platform-owned persistence layer. This facade adds
//! the Safe-boundary owner check that every mutating module operation requires
//! and materializes declared assets for process modules into a per-module
//! package directory. It still grants no implicit execution or data access.

use std::fs;
use std::path::{Path, PathBuf};

use aegis_safe::SafeStore;
use aegis_storage::StoragePaths;
use sha2::{Digest, Sha256};

use crate::manifest::{validate_module_id, ModuleManifest};
use crate::{
    ModuleError, ModuleEvent, ModuleInspection, ModuleRecord, ModuleRegistry, ModuleVerification,
};

/// Owner-authorized module lifecycle operations.
#[derive(Debug, Clone)]
pub struct ModuleManager {
    registry: ModuleRegistry,
    safe: SafeStore,
    paths: StoragePaths,
}

impl ModuleManager {
    pub fn new(paths: StoragePaths) -> Self {
        Self {
            registry: ModuleRegistry::new(paths.clone()),
            safe: SafeStore::new(paths.clone()),
            paths,
        }
    }

    pub fn initialize(&self) -> Result<(), ModuleError> {
        self.registry.initialize()
    }

    pub fn install(
        &self,
        manifest_json: &str,
        expected_sha256: Option<&str>,
        owner_token: &str,
    ) -> Result<ModuleRecord, ModuleError> {
        self.authorize(owner_token)?;
        self.registry.install(manifest_json, expected_sha256)
    }

    /// Install a package directory containing `manifest.json` and every
    /// declared asset.
    ///
    /// Assets are verified by size and SHA-256, staged under
    /// `<root>/modules/.staging`, registered, and then moved into
    /// `<root>/modules/<module_id>`. A failure before the final move leaves no
    /// registry row. This establishes content integrity only.
    pub fn install_package(
        &self,
        package_dir: impl AsRef<Path>,
        expected_sha256: Option<&str>,
        owner_token: &str,
    ) -> Result<ModuleRecord, ModuleError> {
        self.authorize(owner_token)?;
        let package_dir = package_dir.as_ref();
        require_package_dir(package_dir)?;

        let manifest_json = fs::read_to_string(package_dir.join("manifest.json"))?;
        let manifest: ModuleManifest = serde_json::from_str(&manifest_json)?;
        manifest.validate()?;
        validate_module_id(&manifest.module_id)?;
        if !manifest
            .assets
            .iter()
            .any(|asset| asset.path == manifest.entrypoint.path)
        {
            return Err(ModuleError::Package(
                "the entrypoint path must be declared as an asset".to_owned(),
            ));
        }

        let modules_root = self.paths.root().join("modules");
        fs::create_dir_all(&modules_root)?;
        let final_dir = modules_root.join(&manifest.module_id);
        if final_dir.exists() {
            return Err(ModuleError::PackageAlreadyMaterialized(
                manifest.module_id.clone(),
            ));
        }

        let staging_root = modules_root.join(".staging");
        fs::create_dir_all(&staging_root)?;
        let staging = staging_root.join(format!(
            "{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|duration| duration.as_nanos())
                .unwrap_or(0)
        ));

        if let Err(error) = stage_package(package_dir, &manifest, &staging) {
            let _ = fs::remove_dir_all(&staging);
            return Err(error);
        }

        let record = match self.registry.install(&manifest_json, expected_sha256) {
            Ok(record) => record,
            Err(error) => {
                let _ = fs::remove_dir_all(&staging);
                return Err(error);
            }
        };

        if let Err(error) = fs::rename(&staging, &final_dir) {
            let _ = fs::remove_dir_all(&staging);
            let _ = self.registry.uninstall(&manifest.module_id);
            return Err(ModuleError::Io(error));
        }

        Ok(record)
    }

    /// Directory containing one installed package, if it was materialized.
    pub fn package_dir(&self, module_id: &str) -> Result<PathBuf, ModuleError> {
        validate_module_id(module_id)?;
        Ok(self.paths.root().join("modules").join(module_id))
    }

    /// Read-only enable precondition check (no owner proof or mutation).
    pub fn check_enable(&self, module_id: &str) -> Result<(), ModuleError> {
        self.registry.check_enable(module_id)
    }

    /// Read-only disable precondition check (no owner proof or mutation).
    pub fn check_disable(&self, module_id: &str) -> Result<(), ModuleError> {
        self.registry.check_disable(module_id)
    }

    pub fn enable(&self, module_id: &str, owner_token: &str) -> Result<ModuleRecord, ModuleError> {
        self.authorize(owner_token)?;
        self.registry.enable(module_id)
    }

    pub fn disable(&self, module_id: &str, owner_token: &str) -> Result<ModuleRecord, ModuleError> {
        self.authorize(owner_token)?;
        self.registry.disable(module_id)
    }

    pub fn uninstall(
        &self,
        module_id: &str,
        owner_token: &str,
    ) -> Result<ModuleRecord, ModuleError> {
        self.authorize(owner_token)?;
        self.registry.uninstall(module_id)
    }

    pub fn list(&self) -> Result<Vec<ModuleRecord>, ModuleError> {
        self.registry.list()
    }

    pub fn inspect(&self, module_id: &str) -> Result<ModuleInspection, ModuleError> {
        self.registry.inspect(module_id)
    }

    pub fn verify(&self, module_id: &str) -> Result<ModuleVerification, ModuleError> {
        self.registry.verify(module_id)
    }

    pub fn lifecycle_events(&self, module_id: &str) -> Result<Vec<ModuleEvent>, ModuleError> {
        self.registry.lifecycle_events(module_id)
    }

    pub fn tool_bindings(&self) -> Result<Vec<crate::registry::ModuleToolBinding>, ModuleError> {
        self.registry.tool_bindings()
    }

    fn authorize(&self, owner_token: &str) -> Result<(), ModuleError> {
        if !self.safe.verify_owner(owner_token)? {
            return Err(ModuleError::Unauthorized);
        }
        Ok(())
    }
}

fn require_package_dir(package_dir: &Path) -> Result<(), ModuleError> {
    let metadata = fs::symlink_metadata(package_dir)?;
    if is_redirected(&metadata) || !metadata.is_dir() {
        return Err(ModuleError::Package(
            "package root must be a real directory".to_owned(),
        ));
    }
    Ok(())
}

fn stage_package(
    package_dir: &Path,
    manifest: &ModuleManifest,
    staging: &Path,
) -> Result<(), ModuleError> {
    fs::create_dir_all(staging)?;
    let mut total_bytes = 0u64;
    for asset in &manifest.assets {
        let source = package_dir.join(&asset.path);
        let metadata = fs::symlink_metadata(&source)?;
        if is_redirected(&metadata) || !metadata.is_file() {
            return Err(ModuleError::Package(format!(
                "asset is not a real file: {}",
                asset.path
            )));
        }
        let bytes = fs::read(&source)?;
        if bytes.len() as u64 != asset.bytes {
            return Err(ModuleError::Package(format!(
                "asset size mismatch: {}",
                asset.path
            )));
        }
        if sha256_hex(&bytes) != asset.sha256 {
            return Err(ModuleError::Package(format!(
                "asset digest mismatch: {}",
                asset.path
            )));
        }
        total_bytes = total_bytes.saturating_add(asset.bytes);
        let destination = staging.join(&asset.path);
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&destination, &bytes)?;
        let copied = fs::read(&destination)?;
        if sha256_hex(&copied) != asset.sha256 {
            return Err(ModuleError::Package(format!(
                "staged asset digest mismatch: {}",
                asset.path
            )));
        }
    }
    if total_bytes > manifest.resources.max_disk_bytes {
        return Err(ModuleError::Package(
            "package assets exceed the declared disk budget".to_owned(),
        ));
    }
    let canonical = serde_json::to_vec_pretty(manifest)?;
    fs::write(staging.join("manifest.json"), canonical)?;
    Ok(())
}

fn sha256_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let digest = Sha256::digest(bytes);
    let mut encoded = String::with_capacity(64);
    for byte in digest {
        encoded.push(HEX[(byte >> 4) as usize] as char);
        encoded.push(HEX[(byte & 0x0f) as usize] as char);
    }
    encoded
}

#[cfg(windows)]
fn is_redirected(metadata: &fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0000_0400;
    metadata.file_type().is_symlink()
        || (metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT) != 0
}

#[cfg(not(windows))]
fn is_redirected(metadata: &fs::Metadata) -> bool {
    metadata.file_type().is_symlink()
}
