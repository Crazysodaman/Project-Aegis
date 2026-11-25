//! Versioned module manifest contract (`aegis.module.manifest.v1`).
//!
//! The manifest is a declarative description only. Declaring a capability or a
//! permission request does not grant the module access to Platform or Core
//! data. This stage does not execute, load, or sandbox modules.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;

pub const MODULE_MANIFEST_SCHEMA: &str = "aegis.module.manifest.v1";
pub const MODULE_SDK_NAME: &str = "aegis.module.sdk";
pub const MODULE_SDK_VERSION: &str = "1.0.0";
pub const DIGEST_ALGORITHM_SHA256: &str = "sha256";

pub const MAX_MODULE_ID_LEN: usize = 64;
pub const MAX_NAME_LEN: usize = 128;
pub const MAX_PUBLISHER_LEN: usize = 128;
pub const MAX_CAPABILITIES: usize = 64;
pub const MAX_PERMISSIONS: usize = 64;
pub const MAX_DEPENDENCIES: usize = 64;
pub const MAX_ASSETS: usize = 256;
pub const MAX_LIFECYCLE_HOOKS: usize = 16;
pub const MAX_TOOLS: usize = 64;
pub const MAX_TOOL_NAME_LEN: usize = 64;
pub const MAX_TOOL_DESCRIPTION_LEN: usize = 512;
pub const MAX_TOOL_SCHEMA_BYTES: usize = 32 * 1024;
pub const MAX_TOOL_TIMEOUT_MS: u64 = 10 * 60 * 1000;
pub const DEFAULT_TOOL_TIMEOUT_MS: u64 = 30 * 1000;
pub const MAX_CONFIG_SCHEMA_BYTES: usize = 64 * 1024;
pub const MAX_MEMORY_BYTES: u64 = 64 * 1024 * 1024 * 1024;
pub const MAX_CPU_MILLIS: u64 = 24 * 60 * 60 * 1000;
pub const MAX_DISK_BYTES: u64 = 1024 * 1024 * 1024 * 1024;
pub const MAX_OPEN_FILES: u32 = 65_535;

const RESERVED_MODULE_IDS: &[&str] = &["platform", "safe", "core", "system", "aegis"];
const ENTRYPOINT_KINDS: &[&str] = &["wasm", "native", "process"];
const LIFECYCLE_HOOK_NAMES: &[&str] = &[
    "on_install",
    "on_enable",
    "on_disable",
    "on_uninstall",
    "on_verify",
];

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ManifestError {
    #[error("module manifest schema is unsupported: {0}")]
    UnsupportedSchema(String),
    #[error("module identifier is invalid: {0}")]
    InvalidModuleId(String),
    #[error("module identifier is reserved: {0}")]
    ReservedModuleId(String),
    #[error("module name is invalid")]
    InvalidName,
    #[error("module version is invalid: {0}")]
    InvalidVersion(String),
    #[error("module publisher is invalid")]
    InvalidPublisher,
    #[error("module SDK compatibility is invalid: {0}")]
    InvalidSdk(String),
    #[error("module capability is invalid: {0}")]
    InvalidCapability(String),
    #[error("module capability is duplicated: {0}")]
    DuplicateCapability(String),
    #[error("module permission is invalid: {0}")]
    InvalidPermission(String),
    #[error("module permission is duplicated: {0}")]
    DuplicatePermission(String),
    #[error("module dependency is invalid: {0}")]
    InvalidDependency(String),
    #[error("module dependency is duplicated: {0}")]
    DuplicateDependency(String),
    #[error("module entrypoint is invalid: {0}")]
    InvalidEntrypoint(String),
    #[error("module asset is invalid: {0}")]
    InvalidAsset(String),
    #[error("module asset path is duplicated: {0}")]
    DuplicateAsset(String),
    #[error("module asset digest is invalid: {0}")]
    InvalidDigest(String),
    #[error("module config schema is invalid")]
    InvalidConfigSchema,
    #[error("module lifecycle hook is invalid: {0}")]
    InvalidLifecycleHook(String),
    #[error("module lifecycle hook is duplicated: {0}")]
    DuplicateLifecycleHook(String),
    #[error("module tool declaration is invalid: {0}")]
    InvalidTool(String),
    #[error("module tool declaration is duplicated: {0}")]
    DuplicateTool(String),
    #[error("module resource limits are invalid: {0}")]
    InvalidResourceLimits(String),
    #[error("module integrity algorithm is unsupported: {0}")]
    UnsupportedIntegrityAlgorithm(String),
    #[error("module integrity digest is invalid")]
    InvalidIntegrityDigest,
    #[error("module integrity digest does not match manifest content")]
    IntegrityMismatch,
    #[error("module manifest serialization failed: {0}")]
    Serialization(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModuleManifest {
    pub schema: String,
    pub module_id: String,
    pub name: String,
    pub version: String,
    pub publisher: String,
    pub sdk: ModuleSdkCompatibility,
    pub capabilities: Vec<String>,
    pub permissions: Vec<ModulePermission>,
    pub dependencies: Vec<ModuleDependency>,
    pub entrypoint: ModuleEntrypoint,
    pub assets: Vec<ModuleAsset>,
    pub config_schema: serde_json::Value,
    pub lifecycle: Vec<ModuleLifecycleHook>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tools: Vec<ModuleToolDeclaration>,
    pub resources: ModuleResourceLimits,
    pub integrity: ModuleIntegrity,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModuleSdkCompatibility {
    pub sdk: String,
    pub min_version: String,
    pub max_version: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModulePermission {
    pub name: String,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModuleDependency {
    pub module_id: String,
    pub version_requirement: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModuleEntrypoint {
    pub kind: String,
    pub runtime: String,
    pub path: String,
    pub symbol: Option<String>,
    pub args: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModuleAsset {
    pub path: String,
    pub sha256: String,
    pub bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModuleLifecycleHook {
    pub name: String,
    pub entrypoint: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModuleToolDeclaration {
    /// Tool name local to this module. The Platform exposes it as
    /// `module.<module_id>.<name>`.
    pub name: String,
    pub description: String,
    /// Permission token the Platform grant registry controls.
    pub permission: String,
    #[serde(default)]
    pub sensitive: bool,
    #[serde(default)]
    pub approval_required: bool,
    #[serde(default)]
    pub timeout_ms: Option<u64>,
    pub input_schema: serde_json::Value,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModuleResourceLimits {
    pub max_memory_bytes: u64,
    pub max_cpu_millis: u64,
    pub max_disk_bytes: u64,
    pub max_open_files: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModuleIntegrity {
    pub algorithm: String,
    pub digest: String,
}

impl ModuleManifest {
    /// Validates the manifest structure and its self-integrity digest.
    pub fn validate(&self) -> Result<(), ManifestError> {
        self.validate_structure()?;
        let computed = self.computed_digest()?;
        if self.integrity.digest != computed {
            return Err(ManifestError::IntegrityMismatch);
        }
        Ok(())
    }

    /// Validates the manifest without checking that the declared digest matches
    /// the manifest content. Used by verification so a stale inner digest is
    /// reported as a digest mismatch rather than an unrelated schema error.
    pub(crate) fn validate_structure(&self) -> Result<(), ManifestError> {
        if self.schema != MODULE_MANIFEST_SCHEMA {
            return Err(ManifestError::UnsupportedSchema(self.schema.clone()));
        }
        validate_module_id(&self.module_id)?;
        if is_reserved_module_id(&self.module_id) {
            return Err(ManifestError::ReservedModuleId(self.module_id.clone()));
        }
        if !is_display_text(&self.name, MAX_NAME_LEN) {
            return Err(ManifestError::InvalidName);
        }
        if parse_version(&self.version).is_none() {
            return Err(ManifestError::InvalidVersion(self.version.clone()));
        }
        if !is_display_text(&self.publisher, MAX_PUBLISHER_LEN) {
            return Err(ManifestError::InvalidPublisher);
        }
        self.validate_sdk()?;
        self.validate_capabilities()?;
        self.validate_permissions()?;
        self.validate_dependencies()?;
        self.validate_entrypoint()?;
        self.validate_assets()?;
        self.validate_config_schema()?;
        self.validate_lifecycle()?;
        self.validate_tools()?;
        self.validate_resources()?;
        self.validate_integrity()?;
        Ok(())
    }

    /// SHA-256 over the canonical manifest form with the digest field cleared.
    ///
    /// The digest establishes content consistency only. It is not a signature
    /// and does not establish publisher authenticity or installation trust.
    pub fn computed_digest(&self) -> Result<String, ManifestError> {
        let mut sealed = self.clone();
        sealed.integrity.digest.clear();
        let bytes = serde_json::to_vec(&sealed)
            .map_err(|error| ManifestError::Serialization(error.to_string()))?;
        Ok(sha256_hex(&bytes))
    }

    /// Replaces `integrity.digest` with the digest of the current manifest
    /// content and returns it.
    pub fn seal(&mut self) -> Result<String, ManifestError> {
        let digest = self.computed_digest()?;
        self.integrity.digest = digest.clone();
        Ok(digest)
    }

    pub fn verify_integrity(&self) -> Result<(), ManifestError> {
        if self.integrity.digest != self.computed_digest()? {
            return Err(ManifestError::IntegrityMismatch);
        }
        Ok(())
    }

    fn validate_sdk(&self) -> Result<(), ManifestError> {
        if self.sdk.sdk != MODULE_SDK_NAME {
            return Err(ManifestError::InvalidSdk(format!(
                "unsupported SDK: {}",
                self.sdk.sdk
            )));
        }
        let min = parse_version(&self.sdk.min_version).ok_or_else(|| {
            ManifestError::InvalidSdk(format!("invalid minimum version: {}", self.sdk.min_version))
        })?;
        let current = parse_version(MODULE_SDK_VERSION)
            .expect("the built-in module SDK version is a valid semantic version");
        if min > current {
            return Err(ManifestError::InvalidSdk(format!(
                "requires SDK {}+ but this build provides {}",
                self.sdk.min_version, MODULE_SDK_VERSION
            )));
        }
        if let Some(max_raw) = &self.sdk.max_version {
            let max = parse_version(max_raw).ok_or_else(|| {
                ManifestError::InvalidSdk(format!("invalid maximum version: {max_raw}"))
            })?;
            if min > max {
                return Err(ManifestError::InvalidSdk(
                    "minimum SDK version exceeds maximum SDK version".to_owned(),
                ));
            }
            if current > max {
                return Err(ManifestError::InvalidSdk(format!(
                    "requires SDK at most {max_raw} but this build provides {MODULE_SDK_VERSION}"
                )));
            }
        }
        Ok(())
    }

    fn validate_capabilities(&self) -> Result<(), ManifestError> {
        if self.capabilities.len() > MAX_CAPABILITIES {
            return Err(ManifestError::InvalidCapability(
                "too many declared capabilities".to_owned(),
            ));
        }
        let mut seen = BTreeSet::new();
        for capability in &self.capabilities {
            if !is_name_token(capability) {
                return Err(ManifestError::InvalidCapability(capability.clone()));
            }
            if !seen.insert(capability.as_str()) {
                return Err(ManifestError::DuplicateCapability(capability.clone()));
            }
        }
        Ok(())
    }

    fn validate_permissions(&self) -> Result<(), ManifestError> {
        if self.permissions.len() > MAX_PERMISSIONS {
            return Err(ManifestError::InvalidPermission(
                "too many permission requests".to_owned(),
            ));
        }
        let mut seen = BTreeSet::new();
        for permission in &self.permissions {
            if !is_name_token(&permission.name) {
                return Err(ManifestError::InvalidPermission(permission.name.clone()));
            }
            if !is_display_text(&permission.reason, 256) {
                return Err(ManifestError::InvalidPermission(permission.name.clone()));
            }
            if !seen.insert(permission.name.as_str()) {
                return Err(ManifestError::DuplicatePermission(permission.name.clone()));
            }
        }
        Ok(())
    }

    fn validate_dependencies(&self) -> Result<(), ManifestError> {
        if self.dependencies.len() > MAX_DEPENDENCIES {
            return Err(ManifestError::InvalidDependency(
                "too many dependencies".to_owned(),
            ));
        }
        let mut seen = BTreeSet::new();
        for dependency in &self.dependencies {
            validate_module_id(&dependency.module_id)
                .map_err(|_| ManifestError::InvalidDependency(dependency.module_id.clone()))?;
            if is_reserved_module_id(&dependency.module_id) {
                return Err(ManifestError::InvalidDependency(
                    dependency.module_id.clone(),
                ));
            }
            if dependency.module_id == self.module_id {
                return Err(ManifestError::InvalidDependency(
                    "a module cannot depend on itself".to_owned(),
                ));
            }
            if let Some(requirement) = &dependency.version_requirement {
                if !is_version_requirement(requirement) {
                    return Err(ManifestError::InvalidDependency(format!(
                        "invalid version requirement for {}",
                        dependency.module_id
                    )));
                }
            }
            if !seen.insert(dependency.module_id.as_str()) {
                return Err(ManifestError::DuplicateDependency(
                    dependency.module_id.clone(),
                ));
            }
        }
        Ok(())
    }

    fn validate_entrypoint(&self) -> Result<(), ManifestError> {
        if !ENTRYPOINT_KINDS.contains(&self.entrypoint.kind.as_str()) {
            return Err(ManifestError::InvalidEntrypoint(format!(
                "unsupported entrypoint kind: {}",
                self.entrypoint.kind
            )));
        }
        if !is_symbol_token(&self.entrypoint.runtime) {
            return Err(ManifestError::InvalidEntrypoint(
                "invalid runtime identifier".to_owned(),
            ));
        }
        if !is_relative_path(&self.entrypoint.path) {
            return Err(ManifestError::InvalidEntrypoint(
                "invalid entrypoint path".to_owned(),
            ));
        }
        if let Some(symbol) = &self.entrypoint.symbol {
            if !is_symbol_token(symbol) {
                return Err(ManifestError::InvalidEntrypoint(
                    "invalid entrypoint symbol".to_owned(),
                ));
            }
        }
        if self.entrypoint.args.len() > 32
            || self
                .entrypoint
                .args
                .iter()
                .any(|arg| arg.is_empty() || arg.len() > 256 || has_control_chars(arg))
        {
            return Err(ManifestError::InvalidEntrypoint(
                "invalid entrypoint arguments".to_owned(),
            ));
        }
        Ok(())
    }

    fn validate_assets(&self) -> Result<(), ManifestError> {
        if self.assets.len() > MAX_ASSETS {
            return Err(ManifestError::InvalidAsset(
                "too many declared assets".to_owned(),
            ));
        }
        let mut seen = BTreeSet::new();
        for asset in &self.assets {
            if !is_relative_path(&asset.path) {
                return Err(ManifestError::InvalidAsset(asset.path.clone()));
            }
            if !is_sha256(&asset.sha256) {
                return Err(ManifestError::InvalidDigest(asset.path.clone()));
            }
            if asset.bytes > MAX_DISK_BYTES {
                return Err(ManifestError::InvalidAsset(asset.path.clone()));
            }
            if !seen.insert(asset.path.as_str()) {
                return Err(ManifestError::DuplicateAsset(asset.path.clone()));
            }
        }
        Ok(())
    }

    fn validate_config_schema(&self) -> Result<(), ManifestError> {
        if !self.config_schema.is_object() {
            return Err(ManifestError::InvalidConfigSchema);
        }
        let encoded = serde_json::to_vec(&self.config_schema)
            .map_err(|error| ManifestError::Serialization(error.to_string()))?;
        if encoded.len() > MAX_CONFIG_SCHEMA_BYTES {
            return Err(ManifestError::InvalidConfigSchema);
        }
        Ok(())
    }

    fn validate_lifecycle(&self) -> Result<(), ManifestError> {
        if self.lifecycle.len() > MAX_LIFECYCLE_HOOKS {
            return Err(ManifestError::InvalidLifecycleHook(
                "too many lifecycle hooks".to_owned(),
            ));
        }
        let mut seen = BTreeSet::new();
        for hook in &self.lifecycle {
            if !LIFECYCLE_HOOK_NAMES.contains(&hook.name.as_str())
                || !is_symbol_token(&hook.entrypoint)
            {
                return Err(ManifestError::InvalidLifecycleHook(hook.name.clone()));
            }
            if !seen.insert(hook.name.as_str()) {
                return Err(ManifestError::DuplicateLifecycleHook(hook.name.clone()));
            }
        }
        Ok(())
    }

    fn validate_tools(&self) -> Result<(), ManifestError> {
        if self.tools.len() > MAX_TOOLS {
            return Err(ManifestError::InvalidTool(
                "too many tool declarations".to_owned(),
            ));
        }
        let mut seen = BTreeSet::new();
        for tool in &self.tools {
            if !is_name_token(&tool.name) || tool.name.len() > MAX_TOOL_NAME_LEN {
                return Err(ManifestError::InvalidTool(tool.name.clone()));
            }
            if tool.name.len() + self.module_id.len() + 8 > 160 {
                return Err(ManifestError::InvalidTool(
                    "effective tool name is too long".to_owned(),
                ));
            }
            if !is_display_text(&tool.description, MAX_TOOL_DESCRIPTION_LEN) {
                return Err(ManifestError::InvalidTool(tool.name.clone()));
            }
            if !is_name_token(&tool.permission) {
                return Err(ManifestError::InvalidTool(tool.permission.clone()));
            }
            if let Some(timeout) = tool.timeout_ms {
                if timeout == 0 || timeout > MAX_TOOL_TIMEOUT_MS {
                    return Err(ManifestError::InvalidTool(tool.name.clone()));
                }
            }
            if !tool.input_schema.is_object() {
                return Err(ManifestError::InvalidTool(tool.name.clone()));
            }
            let encoded = serde_json::to_vec(&tool.input_schema)
                .map_err(|error| ManifestError::Serialization(error.to_string()))?;
            if encoded.len() > MAX_TOOL_SCHEMA_BYTES {
                return Err(ManifestError::InvalidTool(tool.name.clone()));
            }
            if !seen.insert(tool.name.as_str()) {
                return Err(ManifestError::DuplicateTool(tool.name.clone()));
            }
        }
        Ok(())
    }

    fn validate_resources(&self) -> Result<(), ManifestError> {
        if self.resources.max_memory_bytes == 0
            || self.resources.max_memory_bytes > MAX_MEMORY_BYTES
        {
            return Err(ManifestError::InvalidResourceLimits(
                "max_memory_bytes".to_owned(),
            ));
        }
        if self.resources.max_cpu_millis == 0 || self.resources.max_cpu_millis > MAX_CPU_MILLIS {
            return Err(ManifestError::InvalidResourceLimits(
                "max_cpu_millis".to_owned(),
            ));
        }
        if self.resources.max_disk_bytes == 0 || self.resources.max_disk_bytes > MAX_DISK_BYTES {
            return Err(ManifestError::InvalidResourceLimits(
                "max_disk_bytes".to_owned(),
            ));
        }
        if self.resources.max_open_files == 0 || self.resources.max_open_files > MAX_OPEN_FILES {
            return Err(ManifestError::InvalidResourceLimits(
                "max_open_files".to_owned(),
            ));
        }
        Ok(())
    }

    fn validate_integrity(&self) -> Result<(), ManifestError> {
        if self.integrity.algorithm != DIGEST_ALGORITHM_SHA256 {
            return Err(ManifestError::UnsupportedIntegrityAlgorithm(
                self.integrity.algorithm.clone(),
            ));
        }
        if !is_sha256(&self.integrity.digest) {
            return Err(ManifestError::InvalidIntegrityDigest);
        }
        Ok(())
    }
}

pub fn validate_module_id(module_id: &str) -> Result<(), ManifestError> {
    if module_id.len() > MAX_MODULE_ID_LEN || !is_name_token(module_id) || module_id.contains("..")
    {
        return Err(ManifestError::InvalidModuleId(module_id.to_owned()));
    }
    Ok(())
}

pub fn is_reserved_module_id(module_id: &str) -> bool {
    module_id == "aegis"
        || module_id.starts_with("aegis.")
        || RESERVED_MODULE_IDS.contains(&module_id)
}

pub fn is_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn is_name_token(value: &str) -> bool {
    if value.is_empty() || value.len() > MAX_MODULE_ID_LEN {
        return false;
    }
    let bytes = value.as_bytes();
    if !bytes[0].is_ascii_lowercase() {
        return false;
    }
    value.bytes().all(|byte| {
        byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'.' | b'_' | b'-')
    }) && !value.starts_with('.')
        && !value.ends_with('.')
}

fn is_symbol_token(value: &str) -> bool {
    if value.is_empty() || value.len() > 128 {
        return false;
    }
    let bytes = value.as_bytes();
    if !bytes[0].is_ascii_alphabetic() && bytes[0] != b'_' {
        return false;
    }
    value
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.'))
}

fn is_relative_path(value: &str) -> bool {
    if value.is_empty()
        || value.len() > 512
        || value.starts_with('/')
        || value.starts_with('\\')
        || value.contains('\\')
        || value.contains(':')
        || value.contains("..")
    {
        return false;
    }
    value
        .split('/')
        .all(|component| !component.is_empty() && component != "." && !has_control_chars(component))
}

fn is_display_text(value: &str, max_len: usize) -> bool {
    !value.trim().is_empty()
        && value == value.trim()
        && value.len() <= max_len
        && !has_control_chars(value)
}

fn has_control_chars(value: &str) -> bool {
    value.chars().any(char::is_control)
}

fn is_version_requirement(value: &str) -> bool {
    if value == "*" {
        return true;
    }
    let candidate = strip_version_operator(value);
    parse_version(candidate).is_some()
}

fn strip_version_operator(value: &str) -> &str {
    value
        .strip_prefix(">=")
        .or_else(|| value.strip_prefix("<="))
        .or_else(|| value.strip_prefix('^'))
        .or_else(|| value.strip_prefix('~'))
        .or_else(|| value.strip_prefix('='))
        .or_else(|| value.strip_prefix('>'))
        .or_else(|| value.strip_prefix('<'))
        .unwrap_or(value)
}

/// Whether `version` satisfies a declared dependency requirement.
///
/// Supported forms: `*`, exact `1.2.3`, and the operators `=`, `>`, `>=`,
/// `<`, `<=`, `^` (same major), `~` (same minor). A malformed requirement or
/// version never satisfies a dependency (fail-closed).
pub fn version_satisfies(requirement: &str, version: &str) -> bool {
    if requirement == "*" {
        return true;
    }
    let Some(required) = parse_version(strip_version_operator(requirement)) else {
        return false;
    };
    let Some(actual) = parse_version(version) else {
        return false;
    };
    if requirement.starts_with(">=") {
        return actual >= required;
    }
    if requirement.starts_with("<=") {
        return actual <= required;
    }
    if let Some(rest) = requirement.strip_prefix('^') {
        if parse_version(rest).is_none() {
            return false;
        }
        if required.0 > 0 {
            return actual >= required && actual.0 == required.0;
        }
        if required.1 > 0 {
            return actual >= required && actual.0 == 0 && actual.1 == required.1;
        }
        return actual == required;
    }
    if let Some(rest) = requirement.strip_prefix('~') {
        if parse_version(rest).is_none() {
            return false;
        }
        return actual >= required && actual.0 == required.0 && actual.1 == required.1;
    }
    if requirement.starts_with('>') {
        return actual > required;
    }
    if requirement.starts_with('<') {
        return actual < required;
    }
    actual == required
}

fn parse_version(value: &str) -> Option<(u64, u64, u64)> {
    if value.is_empty() || value.len() > 32 {
        return None;
    }
    let mut parts = value.split('.');
    let major = parts.next()?.parse::<u64>().ok()?;
    let minor = parts.next()?.parse::<u64>().ok()?;
    let patch = parts.next()?.parse::<u64>().ok()?;
    if parts.next().is_some() {
        return None;
    }
    Some((major, minor, patch))
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn module_identifiers_are_bounded_and_lowercase() {
        assert!(validate_module_id("com.example.hello").is_ok());
        assert!(validate_module_id("hello").is_ok());
        assert!(validate_module_id("Com.Example").is_err());
        assert!(validate_module_id("com..example").is_err());
        assert!(validate_module_id("com.example.").is_err());
        assert!(validate_module_id("-example").is_err());
    }

    #[test]
    fn reserved_identifiers_are_rejected() {
        assert!(is_reserved_module_id("aegis"));
        assert!(is_reserved_module_id("aegis.platform"));
        assert!(is_reserved_module_id("platform"));
        assert!(!is_reserved_module_id("com.example.hello"));
    }

    #[test]
    fn digests_are_lowercase_sha256() {
        assert!(is_sha256(&"a".repeat(64)));
        assert!(!is_sha256(&"A".repeat(64)));
        assert!(!is_sha256("abc"));
    }

    #[test]
    fn tool_declarations_are_validated() {
        let mut manifest: ModuleManifest =
            serde_json::from_str(include_str!("../examples/hello.manifest.json"))
                .expect("example manifest");
        manifest.tools.push(ModuleToolDeclaration {
            name: "echo".to_owned(),
            description: "Echo input.".to_owned(),
            permission: "module.echo".to_owned(),
            sensitive: false,
            approval_required: false,
            timeout_ms: Some(1_000),
            input_schema: serde_json::json!({"type": "object", "required": ["value"]}),
        });
        manifest.validate_structure().expect("valid tool");
        manifest.tools.push(ModuleToolDeclaration {
            name: "echo".to_owned(),
            description: "Duplicate.".to_owned(),
            permission: "module.echo".to_owned(),
            sensitive: false,
            approval_required: false,
            timeout_ms: None,
            input_schema: serde_json::json!({"type": "object"}),
        });
        assert!(matches!(
            manifest.validate_structure(),
            Err(ManifestError::DuplicateTool(_))
        ));
    }

    #[test]
    fn invalid_tool_declarations_are_rejected() {
        let mut manifest: ModuleManifest =
            serde_json::from_str(include_str!("../examples/hello.manifest.json"))
                .expect("example manifest");
        manifest.tools.push(ModuleToolDeclaration {
            name: "Bad Name".to_owned(),
            description: "Invalid name.".to_owned(),
            permission: "module.echo".to_owned(),
            sensitive: false,
            approval_required: false,
            timeout_ms: None,
            input_schema: serde_json::json!({"type": "object"}),
        });
        assert!(matches!(
            manifest.validate_structure(),
            Err(ManifestError::InvalidTool(_))
        ));

        manifest.tools[0].name = "echo".to_owned();
        manifest.tools[0].timeout_ms = Some(0);
        assert!(matches!(
            manifest.validate_structure(),
            Err(ManifestError::InvalidTool(_))
        ));

        manifest.tools[0].timeout_ms = None;
        manifest.tools[0].input_schema = serde_json::json!("not-an-object");
        assert!(matches!(
            manifest.validate_structure(),
            Err(ManifestError::InvalidTool(_))
        ));
    }
}
