//! Additive `.corepkg` package format for approved Core installation.
//!
//! A package is either a directory containing [`CORE_PACKAGE_MANIFEST_FILE`] at
//! its root, or the raw manifest JSON bytes for a self-contained package with
//! no external files. The manifest records the Core UUID, Core number, the
//! approved display name/designation expectation, package version, and a list
//! of referenced files/assets with SHA-256 digests.
//!
//! Validation is deliberately strict and happens before any Core storage is
//! created:
//!
//! * Cores #4 and #5 are prohibited.
//! * Core #7 requires an owner-supplied display name; this crate never invents
//!   one (or any other identity field).
//! * Cores #1, #2, #3, and #6 must use their approved display names, and any
//!   supplied designation must match the frozen roster.
//! * Every referenced file must stay inside the package root, be a regular
//!   file, and match its recorded SHA-256 digest.
//!
//! The format is additive: it does not change any existing Core identity field
//! or storage layout. Installation delegates to [`crate::CoreInstaller`] once
//! the package is fully validated, so no partial Core is created for a package
//! that fails validation.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::fs;
use std::ops::Deref;
use std::path::{Path, PathBuf};

use aegis_contracts::core::{canonicalize_core_id, MAX_DISPLAY_NAME_CHARS};
use aegis_contracts::initialization::{CoreInitializationRequest, CORE_INIT_REQUEST_SCHEMA};
use aegis_contracts::lifecycle::{is_approved_core_number, is_prohibited_core_number};
use aegis_core_integrity::{compute_file_sha256, IntegrityError};
use aegis_storage::now_rfc3339;
use thiserror::Error;

use crate::{CoreInstallation, CoreInstaller, InstallError};

/// Schema identifier for a `.corepkg` manifest.
pub const CORE_PACKAGE_SCHEMA: &str = "aegis.core.package.v1";
/// File name of the package manifest inside a package directory.
pub const CORE_PACKAGE_MANIFEST_FILE: &str = "manifest.json";
/// Current manifest format version (major version 1 is accepted).
pub const CORE_PACKAGE_FORMAT_VERSION: &str = "1.0.0";

/// Maximum manifest size accepted from disk or bytes (1 MiB).
pub const MAX_PACKAGE_MANIFEST_BYTES: u64 = 1024 * 1024;
/// Maximum number of file/asset references in one package.
pub const MAX_PACKAGE_FILES: usize = 4096;
/// Maximum size of one referenced package file (512 MiB).
pub const MAX_PACKAGE_FILE_BYTES: u64 = 512 * 1024 * 1024;
/// Maximum length of one relative package file path.
pub const MAX_PACKAGE_PATH_BYTES: usize = 4096;

const MAX_JSON_DEPTH: usize = 32;

/// One file or asset referenced by a package manifest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CorePackageFile {
    /// Relative, forward-slash separated path inside the package root.
    pub path: String,
    /// Lowercase SHA-256 digest of the exact file bytes.
    pub sha256: String,
}

impl CorePackageFile {
    fn to_json(&self) -> String {
        format!(
            "{{\"path\":{},\"sha256\":{}}}",
            json_quote(&self.path),
            json_quote(&self.sha256)
        )
    }
}

/// A parsed and structurally validated `.corepkg` manifest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CorePackageManifest {
    pub schema: String,
    pub version: String,
    /// Immutable Core UUID from the existing identity contract.
    pub core_uuid: String,
    pub core_number: u8,
    /// Display name the package expects to install.
    pub display_name: String,
    /// Optional catalog designation expectation.
    pub designation: Option<String>,
    pub files: Vec<CorePackageFile>,
    pub assets: Vec<CorePackageFile>,
}

impl CorePackageManifest {
    /// Serialize this manifest to compact JSON.
    ///
    /// Intended for package builders and tests; the parser accepts the same
    /// shape. Strings are JSON-escaped.
    pub fn to_json(&self) -> String {
        let files = self
            .files
            .iter()
            .map(CorePackageFile::to_json)
            .collect::<Vec<_>>()
            .join(",");
        let assets = self
            .assets
            .iter()
            .map(CorePackageFile::to_json)
            .collect::<Vec<_>>()
            .join(",");
        let designation = match &self.designation {
            Some(value) => json_quote(value),
            None => "null".to_owned(),
        };
        format!(
            "{{\"schema\":{},\"version\":{},\"core_uuid\":{},\"core_number\":{},\
             \"display_name\":{},\"designation\":{},\"files\":[{}],\"assets\":[{}]}}",
            json_quote(&self.schema),
            json_quote(&self.version),
            json_quote(&self.core_uuid),
            self.core_number,
            json_quote(&self.display_name),
            designation,
            files,
            assets,
        )
    }

    /// Parse a manifest from JSON bytes.
    ///
    /// This performs schema, type, path, and digest-shape validation. Frozen
    /// roster identity checks are applied by [`validate_package`].
    pub fn from_json(bytes: &[u8]) -> Result<Self, PackageError> {
        parse_manifest(bytes)
    }
}

/// A fully validated package, safe to hand to the Core installer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidatedCorePackage {
    pub manifest: CorePackageManifest,
    /// Number of referenced files/assets whose digests were verified.
    pub verified_files: usize,
    /// Total byte length of the verified referenced files/assets.
    pub total_bytes: u64,
}

/// Alias for callers that prefer the shorter package name.
pub type ValidatedPackage = ValidatedCorePackage;

impl Deref for ValidatedCorePackage {
    type Target = CorePackageManifest;

    fn deref(&self) -> &Self::Target {
        &self.manifest
    }
}

/// Error returned while validating or installing a `.corepkg`.
#[derive(Debug, Error)]
pub enum PackageError {
    #[error("Core installation failed: {0}")]
    Install(#[from] InstallError),
    #[error("Core integrity check failed: {0}")]
    Integrity(#[from] IntegrityError),
    #[error("package I/O failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("package root is not a directory: {0:?}")]
    NotADirectory(PathBuf),
    #[error("package root is redirected (symlink or reparse point): {0:?}")]
    RedirectedPackageRoot(PathBuf),
    #[error("package manifest is missing: {0:?}")]
    MissingManifest(PathBuf),
    #[error("package manifest is too large: {size} bytes")]
    ManifestTooLarge { size: u64 },
    #[error("package manifest is not valid UTF-8")]
    NonUtf8Manifest,
    #[error("package manifest JSON is invalid: {0}")]
    InvalidJson(String),
    #[error("package manifest is invalid: {0}")]
    InvalidManifest(String),
    #[error("unsupported package schema: {0}")]
    UnsupportedSchema(String),
    #[error("unsupported package version: {0}")]
    UnsupportedVersion(String),
    #[error("unknown package field: {0}")]
    UnknownField(String),
    #[error("Core number {0} is prohibited from implementation")]
    ProhibitedCoreNumber(u8),
    #[error("Core number {0} is not part of the approved roster")]
    UnknownCoreNumber(u8),
    #[error("Core {core_number} must use its approved display name: {expected}")]
    DisplayNameMismatch { core_number: u8, expected: String },
    #[error("Core {0} requires an owner-supplied display name")]
    DisplayNameRequired(u8),
    #[error("Core {core_number} must use its approved designation: {expected}")]
    DesignationMismatch { core_number: u8, expected: String },
    #[error("package display name is invalid")]
    InvalidDisplayName,
    #[error("package Core UUID is invalid")]
    InvalidCoreUuid,
    #[error("package file path is not a safe relative path: {0}")]
    UnsafePath(String),
    #[error("package file is missing or unreadable: {0} ({1})")]
    UnreadableFile(String, String),
    #[error("package file is not a regular file: {0}")]
    NotARegularFile(String),
    #[error("package file is redirected (symlink or reparse point): {0}")]
    RedirectedFile(String),
    #[error("package file is too large: {0}")]
    FileTooLarge(String),
    #[error("package contains too many referenced files")]
    TooManyFiles,
    #[error("package path is duplicated: {0}")]
    DuplicatePath(String),
    #[error("package file digest does not match: {path} (expected {expected}, actual {actual})")]
    DigestMismatch {
        path: String,
        expected: String,
        actual: String,
    },
    #[error("a byte package may not reference external files")]
    ExternalFilesNotSupported,
    #[error("Core {0} is already installed; refusing to overwrite it")]
    CoreNumberAlreadyInstalled(u8),
}

/// A package source: an on-disk package directory or raw manifest bytes.
pub trait CorePackageSource {
    fn validate(&self) -> Result<ValidatedCorePackage, PackageError>;
}

impl CorePackageSource for &Path {
    fn validate(&self) -> Result<ValidatedCorePackage, PackageError> {
        validate_package_dir(self)
    }
}

impl CorePackageSource for PathBuf {
    fn validate(&self) -> Result<ValidatedCorePackage, PackageError> {
        validate_package_dir(self.as_path())
    }
}

impl CorePackageSource for &PathBuf {
    fn validate(&self) -> Result<ValidatedCorePackage, PackageError> {
        validate_package_dir(self.as_path())
    }
}

impl CorePackageSource for &str {
    fn validate(&self) -> Result<ValidatedCorePackage, PackageError> {
        validate_package_dir(Path::new(self))
    }
}

impl CorePackageSource for String {
    fn validate(&self) -> Result<ValidatedCorePackage, PackageError> {
        validate_package_dir(Path::new(self))
    }
}

impl CorePackageSource for &String {
    fn validate(&self) -> Result<ValidatedCorePackage, PackageError> {
        validate_package_dir(Path::new(self))
    }
}

impl CorePackageSource for &[u8] {
    fn validate(&self) -> Result<ValidatedCorePackage, PackageError> {
        validate_bytes(self)
    }
}

impl CorePackageSource for Vec<u8> {
    fn validate(&self) -> Result<ValidatedCorePackage, PackageError> {
        validate_bytes(self)
    }
}

impl CorePackageSource for &Vec<u8> {
    fn validate(&self) -> Result<ValidatedCorePackage, PackageError> {
        validate_bytes(self)
    }
}

/// Validate a package directory, or manifest bytes when a byte source is used.
pub fn validate_package(
    package: impl CorePackageSource,
) -> Result<ValidatedCorePackage, PackageError> {
    package.validate()
}

/// Validate a byte package (a self-contained manifest with no external files).
pub fn validate_package_bytes(bytes: &[u8]) -> Result<ValidatedCorePackage, PackageError> {
    validate_bytes(bytes)
}

/// Validate a package and, only if it is valid, delegate to [`CoreInstaller`].
///
/// A package directory or manifest bytes may be passed. Existing Cores (by UUID
/// or Core number) are never overwritten.
pub fn install_package(
    root: impl AsRef<Path>,
    package: impl CorePackageSource,
    owner_token: &str,
) -> Result<CoreInstallation, PackageError> {
    let validated = package.validate()?;
    install_validated(root.as_ref(), &validated, owner_token)
}

/// Install from a byte package (a self-contained manifest with no files).
pub fn install_package_bytes(
    root: impl AsRef<Path>,
    bytes: &[u8],
    owner_token: &str,
) -> Result<CoreInstallation, PackageError> {
    install_package(root, bytes, owner_token)
}

fn install_validated(
    root: &Path,
    package: &ValidatedCorePackage,
    owner_token: &str,
) -> Result<CoreInstallation, PackageError> {
    let manifest = &package.manifest;
    let installer = CoreInstaller::new(root)?;

    if installer.get(&manifest.core_uuid)?.is_some() {
        return Err(InstallError::AlreadyInitialized.into());
    }
    if installer
        .list()?
        .iter()
        .any(|entry| entry.core_number == manifest.core_number)
    {
        return Err(PackageError::CoreNumberAlreadyInstalled(
            manifest.core_number,
        ));
    }
    let database_path = installer
        .layout()
        .database_path(&manifest.core_uuid)
        .map_err(InstallError::Storage)?;
    if database_path.exists() {
        return Err(InstallError::ExistingUnregisteredDatabase.into());
    }

    let request = CoreInitializationRequest {
        schema: CORE_INIT_REQUEST_SCHEMA.to_owned(),
        core_uuid: manifest.core_uuid.clone(),
        core_number: manifest.core_number,
        display_name: manifest.display_name.clone(),
        requested_at: now_rfc3339().map_err(InstallError::Paths)?,
    };
    Ok(installer.initialize_approved(&request, owner_token)?)
}

fn validate_package_dir(directory: &Path) -> Result<ValidatedCorePackage, PackageError> {
    if !directory.is_dir() {
        return Err(PackageError::NotADirectory(directory.to_path_buf()));
    }
    let root_metadata = fs::symlink_metadata(directory)?;
    if is_redirected(&root_metadata) {
        return Err(PackageError::RedirectedPackageRoot(directory.to_path_buf()));
    }

    let manifest_path = directory.join(CORE_PACKAGE_MANIFEST_FILE);
    let manifest_metadata = match fs::symlink_metadata(&manifest_path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Err(PackageError::MissingManifest(manifest_path));
        }
        Err(error) => return Err(error.into()),
    };
    if is_redirected(&manifest_metadata) || !manifest_metadata.is_file() {
        return Err(PackageError::UnreadableFile(
            CORE_PACKAGE_MANIFEST_FILE.to_owned(),
            "manifest must be a regular file".to_owned(),
        ));
    }
    if manifest_metadata.len() > MAX_PACKAGE_MANIFEST_BYTES {
        return Err(PackageError::ManifestTooLarge {
            size: manifest_metadata.len(),
        });
    }

    let bytes = fs::read(&manifest_path)?;
    let mut manifest = parse_manifest(&bytes)?;
    validate_identity(&mut manifest)?;

    let canonical_root = fs::canonicalize(directory)?;
    let mut verified_files = 0usize;
    let mut total_bytes = 0u64;
    for file in manifest.files.iter().chain(manifest.assets.iter()) {
        let (resolved, bytes) = resolve_package_file(&canonical_root, file)?;
        let actual = compute_file_sha256(&resolved)?;
        if actual != file.sha256 {
            return Err(PackageError::DigestMismatch {
                path: file.path.clone(),
                expected: file.sha256.clone(),
                actual,
            });
        }
        total_bytes = total_bytes
            .checked_add(bytes)
            .ok_or_else(|| PackageError::InvalidManifest("package size overflow".to_owned()))?;
        verified_files += 1;
    }

    Ok(ValidatedCorePackage {
        manifest,
        verified_files,
        total_bytes,
    })
}

fn validate_bytes(bytes: &[u8]) -> Result<ValidatedCorePackage, PackageError> {
    if bytes.len() as u64 > MAX_PACKAGE_MANIFEST_BYTES {
        return Err(PackageError::ManifestTooLarge {
            size: bytes.len() as u64,
        });
    }
    let mut manifest = parse_manifest(bytes)?;
    validate_identity(&mut manifest)?;
    if !manifest.files.is_empty() || !manifest.assets.is_empty() {
        return Err(PackageError::ExternalFilesNotSupported);
    }
    Ok(ValidatedCorePackage {
        manifest,
        verified_files: 0,
        total_bytes: 0,
    })
}

fn resolve_package_file(
    canonical_root: &Path,
    file: &CorePackageFile,
) -> Result<(PathBuf, u64), PackageError> {
    let candidate = canonical_root.join(&file.path);
    let metadata = match fs::symlink_metadata(&candidate) {
        Ok(metadata) => metadata,
        Err(error) => {
            return Err(PackageError::UnreadableFile(
                file.path.clone(),
                error.to_string(),
            ));
        }
    };
    if is_redirected(&metadata) {
        return Err(PackageError::RedirectedFile(file.path.clone()));
    }
    if !metadata.is_file() {
        return Err(PackageError::NotARegularFile(file.path.clone()));
    }
    if metadata.len() > MAX_PACKAGE_FILE_BYTES {
        return Err(PackageError::FileTooLarge(file.path.clone()));
    }
    let resolved = fs::canonicalize(&candidate)?;
    if !resolved.starts_with(canonical_root) || resolved == canonical_root {
        return Err(PackageError::UnsafePath(file.path.clone()));
    }
    Ok((resolved, metadata.len()))
}

/// Frozen roster mirror used for package identity expectations.
///
/// Core-number policy comes from the shared `aegis-contracts` lifecycle
/// constants. The display-name and designation expectations intentionally
/// mirror the frozen roster catalog from `crates/aegis-roster` without adding a
/// dependency or inventing any field. Cores #4 and #5 have no identity and are
/// never installable.
struct FrozenCoreDesign {
    core_number: u8,
    designation: &'static str,
    display_name: Option<&'static str>,
}

const FROZEN_CORE_DESIGNS: [FrozenCoreDesign; 7] = [
    FrozenCoreDesign {
        core_number: 1,
        designation: "Sofía Ada Lyra",
        display_name: Some("Sofía Ada Lyra"),
    },
    FrozenCoreDesign {
        core_number: 2,
        designation: "Mack Fuller",
        display_name: Some("Mack Fuller"),
    },
    FrozenCoreDesign {
        core_number: 3,
        designation: "Nova Arden",
        display_name: Some("Nova Arden"),
    },
    FrozenCoreDesign {
        core_number: 4,
        designation: "Core #4 (design-only twin)",
        display_name: None,
    },
    FrozenCoreDesign {
        core_number: 5,
        designation: "Core #5 (design-only twin)",
        display_name: None,
    },
    FrozenCoreDesign {
        core_number: 6,
        designation: "Vera / S-039",
        display_name: Some("Vera"),
    },
    FrozenCoreDesign {
        core_number: 7,
        designation: "Unwritten",
        display_name: None,
    },
];

fn frozen_design(core_number: u8) -> Option<&'static FrozenCoreDesign> {
    FROZEN_CORE_DESIGNS
        .iter()
        .find(|design| design.core_number == core_number)
}

fn validate_identity(manifest: &mut CorePackageManifest) -> Result<(), PackageError> {
    let core_number = manifest.core_number;
    if is_prohibited_core_number(core_number) {
        return Err(PackageError::ProhibitedCoreNumber(core_number));
    }
    if !is_approved_core_number(core_number) {
        return Err(PackageError::UnknownCoreNumber(core_number));
    }
    let design = frozen_design(core_number).ok_or(PackageError::UnknownCoreNumber(core_number))?;

    let display_name = manifest.display_name.trim();
    if display_name.is_empty() {
        return Err(PackageError::DisplayNameRequired(core_number));
    }
    if display_name.chars().count() > MAX_DISPLAY_NAME_CHARS
        || display_name.chars().any(char::is_control)
    {
        return Err(PackageError::InvalidDisplayName);
    }
    if let Some(expected) = design.display_name {
        if display_name != expected {
            return Err(PackageError::DisplayNameMismatch {
                core_number,
                expected: expected.to_owned(),
            });
        }
    }
    manifest.display_name = display_name.to_owned();

    if let Some(designation) = manifest.designation.as_mut() {
        let trimmed = designation.trim();
        if trimmed.is_empty() {
            return Err(PackageError::InvalidManifest(
                "designation may not be blank".to_owned(),
            ));
        }
        if trimmed != design.designation {
            return Err(PackageError::DesignationMismatch {
                core_number,
                expected: design.designation.to_owned(),
            });
        }
        *designation = trimmed.to_owned();
    }

    Ok(())
}

fn parse_manifest(bytes: &[u8]) -> Result<CorePackageManifest, PackageError> {
    if bytes.len() as u64 > MAX_PACKAGE_MANIFEST_BYTES {
        return Err(PackageError::ManifestTooLarge {
            size: bytes.len() as u64,
        });
    }
    let text = std::str::from_utf8(bytes).map_err(|_| PackageError::NonUtf8Manifest)?;
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let value = JsonParser::new(text).parse()?;
    manifest_from_json(value)
}

fn manifest_from_json(value: JsonValue) -> Result<CorePackageManifest, PackageError> {
    let mut object = match value {
        JsonValue::Object(object) => object,
        _ => {
            return Err(PackageError::InvalidManifest(
                "manifest must be a JSON object".to_owned(),
            ));
        }
    };

    let schema = take_string(&mut object, "schema")?;
    if schema != CORE_PACKAGE_SCHEMA {
        return Err(PackageError::UnsupportedSchema(schema));
    }
    let version = take_string(&mut object, "version")?;
    validate_version(&version)?;
    let core_uuid_raw = take_string(&mut object, "core_uuid")?;
    let core_uuid =
        canonicalize_core_id(&core_uuid_raw).map_err(|_| PackageError::InvalidCoreUuid)?;
    let core_number_raw = take_number(&mut object, "core_number")?;
    let core_number = u8::try_from(core_number_raw).map_err(|_| {
        PackageError::InvalidManifest("core_number must be between 0 and 255".to_owned())
    })?;
    let display_name = take_string(&mut object, "display_name")?;
    let designation = take_optional_string(&mut object, "designation")?;
    let files = take_file_array(&mut object, "files")?;
    let assets = take_file_array(&mut object, "assets")?;

    if let Some(key) = object.keys().next() {
        return Err(PackageError::UnknownField(key.clone()));
    }

    let manifest = CorePackageManifest {
        schema,
        version,
        core_uuid,
        core_number,
        display_name,
        designation,
        files,
        assets,
    };
    validate_structure(&manifest)?;
    Ok(manifest)
}

fn validate_version(version: &str) -> Result<(), PackageError> {
    let trimmed = version.trim();
    if trimmed.is_empty() || trimmed.len() > 64 || trimmed.chars().any(char::is_control) {
        return Err(PackageError::InvalidManifest(
            "version must be a short, non-empty string".to_owned(),
        ));
    }
    if trimmed.split('.').next() != Some("1") {
        return Err(PackageError::UnsupportedVersion(version.to_owned()));
    }
    Ok(())
}

fn validate_structure(manifest: &CorePackageManifest) -> Result<(), PackageError> {
    if manifest.files.len() + manifest.assets.len() > MAX_PACKAGE_FILES {
        return Err(PackageError::TooManyFiles);
    }
    let mut paths = BTreeSet::new();
    for file in manifest.files.iter().chain(manifest.assets.iter()) {
        if !paths.insert(file.path.as_str()) {
            return Err(PackageError::DuplicatePath(file.path.clone()));
        }
    }
    Ok(())
}

fn take_string(
    object: &mut BTreeMap<String, JsonValue>,
    key: &str,
) -> Result<String, PackageError> {
    match object.remove(key) {
        Some(JsonValue::String(value)) => Ok(value),
        Some(_) => Err(PackageError::InvalidManifest(format!(
            "{key} must be a string"
        ))),
        None => Err(PackageError::InvalidManifest(format!("{key} is required"))),
    }
}

fn take_optional_string(
    object: &mut BTreeMap<String, JsonValue>,
    key: &str,
) -> Result<Option<String>, PackageError> {
    match object.remove(key) {
        None | Some(JsonValue::Null) => Ok(None),
        Some(JsonValue::String(value)) => Ok(Some(value)),
        Some(_) => Err(PackageError::InvalidManifest(format!(
            "{key} must be a string or null"
        ))),
    }
}

fn take_number(object: &mut BTreeMap<String, JsonValue>, key: &str) -> Result<i64, PackageError> {
    match object.remove(key) {
        Some(JsonValue::Number(value)) => Ok(value),
        Some(_) => Err(PackageError::InvalidManifest(format!(
            "{key} must be an integer"
        ))),
        None => Err(PackageError::InvalidManifest(format!("{key} is required"))),
    }
}

fn take_file_array(
    object: &mut BTreeMap<String, JsonValue>,
    key: &str,
) -> Result<Vec<CorePackageFile>, PackageError> {
    match object.remove(key) {
        None | Some(JsonValue::Null) => Ok(Vec::new()),
        Some(JsonValue::Array(values)) => {
            if values.len() > MAX_PACKAGE_FILES {
                return Err(PackageError::TooManyFiles);
            }
            values
                .into_iter()
                .map(parse_package_file)
                .collect::<Result<Vec<_>, _>>()
        }
        Some(_) => Err(PackageError::InvalidManifest(format!(
            "{key} must be an array"
        ))),
    }
}

fn parse_package_file(value: JsonValue) -> Result<CorePackageFile, PackageError> {
    let mut object = match value {
        JsonValue::Object(object) => object,
        _ => {
            return Err(PackageError::InvalidManifest(
                "each file reference must be a JSON object".to_owned(),
            ));
        }
    };
    let path = take_string(&mut object, "path")?;
    let sha256 = take_string(&mut object, "sha256")?;
    if let Some(key) = object.keys().next() {
        return Err(PackageError::UnknownField(key.clone()));
    }
    validate_relative_path(&path)?;
    Ok(CorePackageFile {
        path,
        sha256: normalize_sha256(&sha256)?,
    })
}

fn normalize_sha256(value: &str) -> Result<String, PackageError> {
    if value.len() != 64 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(PackageError::InvalidManifest(
            "sha256 must be 64 hexadecimal characters".to_owned(),
        ));
    }
    Ok(value.to_ascii_lowercase())
}

fn validate_relative_path(path: &str) -> Result<(), PackageError> {
    if path.is_empty()
        || path.len() > MAX_PACKAGE_PATH_BYTES
        || path.contains('\0')
        || path.contains('\\')
        || path.contains(':')
        || path.starts_with('/')
        || path.chars().any(char::is_control)
    {
        return Err(PackageError::UnsafePath(path.to_owned()));
    }
    for component in path.split('/') {
        if component.is_empty() || component == "." || component == ".." {
            return Err(PackageError::UnsafePath(path.to_owned()));
        }
    }
    if path == CORE_PACKAGE_MANIFEST_FILE {
        return Err(PackageError::UnsafePath(path.to_owned()));
    }
    Ok(())
}

fn is_redirected(metadata: &fs::Metadata) -> bool {
    if metadata.file_type().is_symlink() {
        return true;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
        if metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            return true;
        }
    }
    false
}

fn json_quote(value: &str) -> String {
    let mut quoted = String::with_capacity(value.len() + 2);
    quoted.push('"');
    for character in value.chars() {
        match character {
            '"' => quoted.push_str("\\\""),
            '\\' => quoted.push_str("\\\\"),
            '\n' => quoted.push_str("\\n"),
            '\r' => quoted.push_str("\\r"),
            '\t' => quoted.push_str("\\t"),
            '\u{0008}' => quoted.push_str("\\b"),
            '\u{000c}' => quoted.push_str("\\f"),
            control if control < '\u{0020}' => {
                let _ = write!(quoted, "\\u{:04x}", control as u32);
            }
            other => quoted.push(other),
        }
    }
    quoted.push('"');
    quoted
}

#[derive(Debug)]
enum JsonValue {
    Null,
    #[allow(dead_code)]
    Bool(bool),
    Number(i64),
    String(String),
    Array(Vec<JsonValue>),
    Object(BTreeMap<String, JsonValue>),
}

struct JsonParser<'a> {
    input: &'a str,
    index: usize,
    depth: usize,
}

impl<'a> JsonParser<'a> {
    fn new(input: &'a str) -> Self {
        Self {
            input,
            index: 0,
            depth: 0,
        }
    }

    fn parse(mut self) -> Result<JsonValue, PackageError> {
        self.skip_whitespace();
        let value = self.parse_value()?;
        self.skip_whitespace();
        if self.index != self.input.len() {
            return Err(self.error("trailing characters after JSON value"));
        }
        Ok(value)
    }

    fn error(&self, message: impl Into<String>) -> PackageError {
        let message = message.into();
        PackageError::InvalidJson(format!("{message} at byte {}", self.index))
    }

    fn skip_whitespace(&mut self) {
        while let Some(character) = self.peek_char() {
            if matches!(character, ' ' | '\t' | '\n' | '\r') {
                self.index += character.len_utf8();
            } else {
                break;
            }
        }
    }

    fn peek_char(&self) -> Option<char> {
        self.input[self.index..].chars().next()
    }

    fn next_char(&mut self) -> Option<char> {
        let character = self.peek_char()?;
        self.index += character.len_utf8();
        Some(character)
    }

    fn expect_char(&mut self, expected: char) -> Result<(), PackageError> {
        match self.next_char() {
            Some(actual) if actual == expected => Ok(()),
            _ => Err(self.error(format!("expected {expected:?}"))),
        }
    }

    fn enter(&mut self) -> Result<(), PackageError> {
        self.depth += 1;
        if self.depth > MAX_JSON_DEPTH {
            return Err(self.error("JSON nesting is too deep"));
        }
        Ok(())
    }

    fn leave(&mut self) {
        self.depth -= 1;
    }

    fn parse_value(&mut self) -> Result<JsonValue, PackageError> {
        self.skip_whitespace();
        match self.peek_char() {
            Some('{') => self.parse_object(),
            Some('[') => self.parse_array(),
            Some('"') => Ok(JsonValue::String(self.parse_string()?)),
            Some('t') => {
                self.expect_literal("true")?;
                Ok(JsonValue::Bool(true))
            }
            Some('f') => {
                self.expect_literal("false")?;
                Ok(JsonValue::Bool(false))
            }
            Some('n') => {
                self.expect_literal("null")?;
                Ok(JsonValue::Null)
            }
            Some(character) if character == '-' || character.is_ascii_digit() => {
                self.parse_number()
            }
            Some(_) => Err(self.error("unexpected character")),
            None => Err(self.error("unexpected end of input")),
        }
    }

    fn parse_object(&mut self) -> Result<JsonValue, PackageError> {
        self.enter()?;
        self.expect_char('{')?;
        let mut object = BTreeMap::new();
        self.skip_whitespace();
        if self.peek_char() == Some('}') {
            self.next_char();
            self.leave();
            return Ok(JsonValue::Object(object));
        }
        loop {
            self.skip_whitespace();
            let key = self.parse_string()?;
            self.skip_whitespace();
            self.expect_char(':')?;
            let value = self.parse_value()?;
            if object.insert(key.clone(), value).is_some() {
                return Err(self.error(format!("duplicate key {key:?}")));
            }
            self.skip_whitespace();
            match self.next_char() {
                Some(',') => continue,
                Some('}') => break,
                _ => return Err(self.error("expected ',' or '}' in object")),
            }
        }
        self.leave();
        Ok(JsonValue::Object(object))
    }

    fn parse_array(&mut self) -> Result<JsonValue, PackageError> {
        self.enter()?;
        self.expect_char('[')?;
        let mut values = Vec::new();
        self.skip_whitespace();
        if self.peek_char() == Some(']') {
            self.next_char();
            self.leave();
            return Ok(JsonValue::Array(values));
        }
        loop {
            let value = self.parse_value()?;
            values.push(value);
            self.skip_whitespace();
            match self.next_char() {
                Some(',') => continue,
                Some(']') => break,
                _ => return Err(self.error("expected ',' or ']' in array")),
            }
        }
        self.leave();
        Ok(JsonValue::Array(values))
    }

    fn expect_literal(&mut self, literal: &str) -> Result<(), PackageError> {
        if self.input[self.index..].starts_with(literal) {
            self.index += literal.len();
            Ok(())
        } else {
            Err(self.error(format!("expected {literal}")))
        }
    }

    fn parse_string(&mut self) -> Result<String, PackageError> {
        self.expect_char('"')?;
        let mut value = String::new();
        loop {
            let character = self
                .next_char()
                .ok_or_else(|| self.error("unterminated string"))?;
            match character {
                '"' => return Ok(value),
                '\\' => {
                    let escape = self
                        .next_char()
                        .ok_or_else(|| self.error("unterminated escape sequence"))?;
                    match escape {
                        '"' => value.push('"'),
                        '\\' => value.push('\\'),
                        '/' => value.push('/'),
                        'b' => value.push('\u{0008}'),
                        'f' => value.push('\u{000c}'),
                        'n' => value.push('\n'),
                        'r' => value.push('\r'),
                        't' => value.push('\t'),
                        'u' => self.parse_unicode_escape(&mut value)?,
                        _ => return Err(self.error("invalid escape sequence")),
                    }
                }
                control if control < '\u{0020}' => {
                    return Err(self.error("control character in string"));
                }
                other => value.push(other),
            }
        }
    }

    fn parse_unicode_escape(&mut self, value: &mut String) -> Result<(), PackageError> {
        let first = self.parse_hex_u16()?;
        let code_point = if (0xD800..=0xDBFF).contains(&first) {
            if self.next_char() != Some('\\') || self.next_char() != Some('u') {
                return Err(self.error("missing low surrogate"));
            }
            let second = self.parse_hex_u16()?;
            if !(0xDC00..=0xDFFF).contains(&second) {
                return Err(self.error("invalid low surrogate"));
            }
            let high = u32::from(first) - 0xD800;
            let low = u32::from(second) - 0xDC00;
            0x1_0000 + ((high << 10) | low)
        } else if (0xDC00..=0xDFFF).contains(&first) {
            return Err(self.error("unexpected low surrogate"));
        } else {
            u32::from(first)
        };
        let character =
            char::from_u32(code_point).ok_or_else(|| self.error("invalid Unicode scalar"))?;
        value.push(character);
        Ok(())
    }

    fn parse_hex_u16(&mut self) -> Result<u16, PackageError> {
        let mut value = 0u16;
        for _ in 0..4 {
            let character = self
                .next_char()
                .ok_or_else(|| self.error("truncated unicode escape"))?;
            let digit = character
                .to_digit(16)
                .ok_or_else(|| self.error("invalid unicode escape"))?;
            value = (value << 4) | digit as u16;
        }
        Ok(value)
    }

    fn parse_number(&mut self) -> Result<JsonValue, PackageError> {
        let start = self.index;
        if self.peek_char() == Some('-') {
            self.next_char();
        }
        let mut digits = 0usize;
        while matches!(self.peek_char(), Some(character) if character.is_ascii_digit()) {
            self.next_char();
            digits += 1;
        }
        if digits == 0 {
            return Err(self.error("invalid number"));
        }
        if matches!(self.peek_char(), Some('.' | 'e' | 'E')) {
            return Err(self.error("only integer numbers are supported"));
        }
        let text = &self.input[start..self.index];
        let number = text
            .parse::<i64>()
            .map_err(|_| self.error("number is out of range"))?;
        Ok(JsonValue::Number(number))
    }
}
