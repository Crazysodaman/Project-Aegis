//! Per-Core embodiment store: asset library plus appearance configuration.

use std::fs;
use std::path::{Path, PathBuf};

use aegis_contracts::core::canonicalize_core_id;
use aegis_core_storage::{CoreDatabase, CoreStorageLayout};
use aegis_storage::{now_rfc3339, open_initialized};
use rusqlite::{params, Connection, OptionalExtension};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::appearance::{validate_animation_name, validate_theme, Appearance, Theme, VoiceProfile};
use crate::asset::{safe_file_name, validate_new_asset, Asset, AssetKind, NewAsset, ASSET_SCHEMA};
use crate::error::EmbodimentError;

/// File name of the per-Core embodiment database.
pub const EMBODIMENT_DB_FILE: &str = "assets.db";
/// SQLite application marker for an embodiment database ("AEGA").
pub const EMBODIMENT_APPLICATION_ID: i64 = 0x4145_4741;
/// Component name registered in the shared `aegis_schema` table.
pub const EMBODIMENT_COMPONENT: &str = "embodiment";
/// Schema identifier for an asset-library event.
pub const ASSET_EVENT_SCHEMA: &str = "aegis.embodiment.event.v1";
/// Schema identifier for the full asset-library export.
pub const ASSET_EXPORT_SCHEMA: &str = "aegis.embodiment.export.v1";

const APPEARANCE_SINGLETON: i64 = 1;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct AssetExport {
    pub schema: String,
    pub core_uuid: String,
    pub exported_at: String,
    pub assets: Vec<Asset>,
    pub appearance: Appearance,
}

/// Per-Core asset library and appearance store.
#[derive(Debug)]
pub struct EmbodimentStore {
    connection: Connection,
    core_uuid: String,
    instance_uuid: String,
    core_dir: PathBuf,
    path: PathBuf,
}

impl EmbodimentStore {
    pub fn open(root: impl AsRef<Path>, core_uuid: &str) -> Result<Self, EmbodimentError> {
        let core_uuid =
            canonicalize_core_id(core_uuid).map_err(|_| EmbodimentError::InvalidCoreUuid)?;
        let layout = CoreStorageLayout::new(root.as_ref())?;
        let snapshot = CoreDatabase::read_snapshot(&layout, &core_uuid)?;
        if snapshot.identity.core_uuid != core_uuid {
            return Err(EmbodimentError::OwnershipMismatch);
        }
        let core_dir = layout.core_dir(&core_uuid)?;
        let path = core_dir.join(EMBODIMENT_DB_FILE);
        if path.parent() != Some(core_dir.as_path()) {
            return Err(EmbodimentError::InvalidInput("embodiment database path"));
        }
        let connection = open_initialized(&path, EMBODIMENT_APPLICATION_ID, EMBODIMENT_COMPONENT)?;
        ensure_tables(&connection)?;
        verify_ownership(
            &connection,
            &core_uuid,
            &snapshot.metadata.database_instance_uuid,
        )?;
        let store = Self {
            connection,
            core_uuid,
            instance_uuid: snapshot.metadata.database_instance_uuid,
            core_dir,
            path,
        };
        store.ensure_appearance()?;
        Ok(store)
    }

    pub fn core_uuid(&self) -> &str {
        &self.core_uuid
    }

    pub fn database_instance_uuid(&self) -> &str {
        &self.instance_uuid
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn import_asset(&self, asset: NewAsset<'_>) -> Result<Asset, EmbodimentError> {
        validate_new_asset(&asset)?;
        let asset_id = Uuid::new_v4().to_string();
        let digest = sha256_hex(asset.bytes);
        let file_name = safe_file_name(asset.name);
        let relative_path = format!("assets/{asset_id}/{file_name}");
        let absolute = self.core_dir.join(&relative_path);
        if !absolute.starts_with(&self.core_dir) {
            return Err(EmbodimentError::InvalidInput("asset path"));
        }
        if let Some(parent) = absolute.parent() {
            fs::create_dir_all(parent)?;
        }
        write_atomic(&absolute, asset.bytes)?;
        let written = fs::read(&absolute)?;
        if sha256_hex(&written) != digest || written.len() as u64 != asset.bytes.len() as u64 {
            let _ = fs::remove_file(&absolute);
            return Err(EmbodimentError::IntegrityMismatch);
        }

        let tags_json = serde_json::to_string(&asset.tags)?;
        let metadata_json = match asset.metadata {
            Some(metadata) => serde_json::to_string(metadata)?,
            None => "null".to_owned(),
        };
        let now = now_rfc3339()?;
        let result = self.connection.execute(
            "INSERT INTO assets(
                asset_id, core_uuid, kind, name, media_type, relative_path,
                sha256, bytes, tags_json, metadata_json, created_at, updated_at,
                deleted, revision
             ) VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?11, 0, 1)",
            params![
                asset_id,
                self.core_uuid,
                asset.kind.as_str(),
                asset.name,
                asset.media_type,
                relative_path,
                digest,
                asset.bytes.len() as i64,
                tags_json,
                metadata_json,
                now,
            ],
        );
        if let Err(error) = result {
            let _ = fs::remove_file(&absolute);
            return Err(error.into());
        }
        self.record_event(
            &asset_id,
            "asset.imported",
            &serde_json::json!({
                "kind": asset.kind.as_str(),
                "sha256": digest,
            }),
        )?;
        self.asset(&asset_id)
    }

    pub fn asset(&self, asset_id: &str) -> Result<Asset, EmbodimentError> {
        validate_asset_id(asset_id)?;
        self.connection
            .query_row(
                &format!(
                    "SELECT {ASSET_COLUMNS} FROM assets WHERE asset_id = ?1 AND core_uuid = ?2"
                ),
                params![asset_id, self.core_uuid],
                read_asset,
            )
            .optional()?
            .ok_or(EmbodimentError::NotFound)
    }

    pub fn list_assets(
        &self,
        kind: Option<AssetKind>,
        include_deleted: bool,
    ) -> Result<Vec<Asset>, EmbodimentError> {
        let kind_value = kind.map(AssetKind::as_str);
        let mut statement = self.connection.prepare(&format!(
            "SELECT {ASSET_COLUMNS} FROM assets
             WHERE core_uuid = ?1
               AND (?2 IS NULL OR kind = ?2)
               AND (?3 = 1 OR deleted = 0)
             ORDER BY created_at ASC, asset_id ASC"
        ))?;
        let mut rows = statement.query(params![
            self.core_uuid,
            kind_value,
            i64::from(u8::from(include_deleted))
        ])?;
        let mut assets = Vec::new();
        while let Some(row) = rows.next()? {
            assets.push(read_asset(row)?);
        }
        Ok(assets)
    }

    /// Read asset bytes after verifying path containment and content integrity.
    pub fn read_asset(&self, asset_id: &str) -> Result<Vec<u8>, EmbodimentError> {
        let asset = self.asset(asset_id)?;
        if asset.deleted {
            return Err(EmbodimentError::Deleted);
        }
        let path = self.core_dir.join(&asset.relative_path);
        if !path.starts_with(&self.core_dir) {
            return Err(EmbodimentError::InvalidInput("asset path"));
        }
        let metadata =
            fs::symlink_metadata(&path).map_err(|_| EmbodimentError::AssetFileMissing)?;
        if is_redirected(&metadata) || !metadata.is_file() {
            return Err(EmbodimentError::AssetFileMissing);
        }
        let bytes = fs::read(&path)?;
        if bytes.len() as u64 != asset.bytes || sha256_hex(&bytes) != asset.sha256 {
            return Err(EmbodimentError::IntegrityMismatch);
        }
        Ok(bytes)
    }

    /// Soft-delete an asset; its file is retained for owner recovery.
    pub fn delete_asset(&self, asset_id: &str) -> Result<Asset, EmbodimentError> {
        let asset = self.asset(asset_id)?;
        if asset.deleted {
            return Ok(asset);
        }
        let now = now_rfc3339()?;
        self.connection.execute(
            "UPDATE assets SET deleted = 1, updated_at = ?1, revision = revision + 1
             WHERE asset_id = ?2",
            params![now, asset_id],
        )?;
        self.record_event(
            asset_id,
            "asset.deleted",
            &serde_json::json!({"recovered": false}),
        )?;
        self.asset(asset_id)
    }

    pub fn appearance(&self) -> Result<Appearance, EmbodimentError> {
        let (json, _): (String, i64) = self.connection.query_row(
            "SELECT appearance_json, revision FROM core_appearance WHERE singleton = ?1",
            [APPEARANCE_SINGLETON],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        let appearance: Appearance = serde_json::from_str(&json)?;
        appearance.validate(&self.core_uuid)?;
        Ok(appearance)
    }

    pub fn set_avatar(&self, asset_id: Option<&str>) -> Result<Appearance, EmbodimentError> {
        if let Some(asset_id) = asset_id {
            self.require_kind(asset_id, AssetKind::Avatar)?;
        }
        self.update_appearance(|appearance| {
            appearance.avatar_asset_id = asset_id.map(str::to_owned);
            Ok(())
        })
    }

    pub fn set_expression(
        &self,
        name: &str,
        asset_id: &str,
    ) -> Result<Appearance, EmbodimentError> {
        validate_animation_name(name)?;
        self.require_kind(asset_id, AssetKind::Expression)?;
        self.update_appearance(|appearance| {
            appearance
                .expressions
                .insert(name.trim().to_owned(), asset_id.to_owned());
            Ok(())
        })
    }

    pub fn remove_expression(&self, name: &str) -> Result<Appearance, EmbodimentError> {
        self.update_appearance(|appearance| {
            appearance.expressions.remove(name.trim());
            Ok(())
        })
    }

    pub fn set_gesture(&self, name: &str, asset_id: &str) -> Result<Appearance, EmbodimentError> {
        validate_animation_name(name)?;
        self.require_kind(asset_id, AssetKind::Gesture)?;
        self.update_appearance(|appearance| {
            appearance
                .gestures
                .insert(name.trim().to_owned(), asset_id.to_owned());
            Ok(())
        })
    }

    pub fn remove_gesture(&self, name: &str) -> Result<Appearance, EmbodimentError> {
        self.update_appearance(|appearance| {
            appearance.gestures.remove(name.trim());
            Ok(())
        })
    }

    pub fn set_wardrobe(&self, asset_ids: Vec<String>) -> Result<Appearance, EmbodimentError> {
        for asset_id in &asset_ids {
            self.require_kind(asset_id, AssetKind::Wardrobe)?;
        }
        self.update_appearance(|appearance| {
            appearance.wardrobe = asset_ids.clone();
            Ok(())
        })
    }

    pub fn set_theme(&self, theme: Theme) -> Result<Appearance, EmbodimentError> {
        validate_theme(&theme)?;
        self.update_appearance(|appearance| {
            appearance.theme = theme.clone();
            Ok(())
        })
    }

    pub fn set_voice(&self, voice: Option<VoiceProfile>) -> Result<Appearance, EmbodimentError> {
        if let Some(voice) = &voice {
            if let Some(sample) = &voice.sample_asset_id {
                self.require_kind(sample, AssetKind::VoiceSample)?;
            }
        }
        self.update_appearance(|appearance| {
            appearance.voice = voice.clone();
            Ok(())
        })
    }

    pub fn export(&self) -> Result<AssetExport, EmbodimentError> {
        Ok(AssetExport {
            schema: ASSET_EXPORT_SCHEMA.to_owned(),
            core_uuid: self.core_uuid.clone(),
            exported_at: now_rfc3339()?,
            assets: self.list_assets(None, true)?,
            appearance: self.appearance()?,
        })
    }

    fn require_kind(&self, asset_id: &str, expected: AssetKind) -> Result<Asset, EmbodimentError> {
        let asset = self.asset(asset_id)?;
        if asset.deleted {
            return Err(EmbodimentError::Deleted);
        }
        if asset.kind != expected {
            return Err(EmbodimentError::WrongAssetKind(asset_id.to_owned()));
        }
        Ok(asset)
    }

    fn ensure_appearance(&self) -> Result<(), EmbodimentError> {
        let existing: Option<i64> = self
            .connection
            .query_row(
                "SELECT revision FROM core_appearance WHERE singleton = ?1",
                [APPEARANCE_SINGLETON],
                |row| row.get(0),
            )
            .optional()?;
        if existing.is_some() {
            return Ok(());
        }
        let appearance = Appearance::new(&self.core_uuid, &now_rfc3339()?);
        self.write_appearance(&appearance)?;
        Ok(())
    }

    fn update_appearance(
        &self,
        update: impl FnOnce(&mut Appearance) -> Result<(), EmbodimentError>,
    ) -> Result<Appearance, EmbodimentError> {
        let mut appearance = self.appearance()?;
        update(&mut appearance)?;
        appearance.updated_at = now_rfc3339()?;
        appearance.revision = appearance.revision.saturating_add(1);
        if appearance.revision == u64::MAX {
            return Err(EmbodimentError::CorruptData("appearance revision"));
        }
        self.write_appearance(&appearance)?;
        Ok(appearance)
    }

    fn write_appearance(&self, appearance: &Appearance) -> Result<(), EmbodimentError> {
        appearance.validate(&self.core_uuid)?;
        let json = serde_json::to_string(appearance)?;
        self.connection.execute(
            "INSERT INTO core_appearance(singleton, core_uuid, appearance_json, updated_at, revision)
             VALUES(?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(singleton) DO UPDATE SET
                 appearance_json = excluded.appearance_json,
                 updated_at = excluded.updated_at,
                 revision = excluded.revision",
            params![
                APPEARANCE_SINGLETON,
                self.core_uuid,
                json,
                appearance.updated_at,
                appearance.revision as i64
            ],
        )?;
        self.record_event(
            "appearance",
            "appearance.updated",
            &serde_json::json!({"revision": appearance.revision}),
        )?;
        Ok(())
    }

    fn record_event(
        &self,
        asset_id: &str,
        event_type: &str,
        detail: &serde_json::Value,
    ) -> Result<(), EmbodimentError> {
        self.connection.execute(
            "INSERT INTO asset_events(asset_id, schema, event_type, detail_json, created_at)
             VALUES(?1, ?2, ?3, ?4, ?5)",
            params![
                asset_id,
                ASSET_EVENT_SCHEMA,
                event_type,
                serde_json::to_string(detail)?,
                now_rfc3339()?
            ],
        )?;
        Ok(())
    }
}

fn ensure_tables(connection: &Connection) -> Result<(), EmbodimentError> {
    connection.execute_batch(
        "CREATE TABLE IF NOT EXISTS asset_ownership (
            singleton INTEGER PRIMARY KEY CHECK(singleton = 1),
            core_uuid TEXT NOT NULL UNIQUE,
            database_instance_uuid TEXT NOT NULL,
            created_at TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS assets (
            asset_id TEXT PRIMARY KEY,
            core_uuid TEXT NOT NULL,
            kind TEXT NOT NULL CHECK(kind IN (
                'avatar', 'expression', 'gesture', 'wardrobe',
                'voice_sample', 'theme', 'texture', 'other'
            )),
            name TEXT NOT NULL,
            media_type TEXT NOT NULL,
            relative_path TEXT NOT NULL UNIQUE,
            sha256 TEXT NOT NULL,
            bytes INTEGER NOT NULL CHECK(bytes > 0),
            tags_json TEXT NOT NULL,
            metadata_json TEXT NOT NULL,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL,
            deleted INTEGER NOT NULL CHECK(deleted IN (0, 1)),
            revision INTEGER NOT NULL CHECK(revision >= 1)
        );
        CREATE INDEX IF NOT EXISTS assets_by_core ON assets(core_uuid, kind, created_at);
        CREATE TABLE IF NOT EXISTS asset_events (
            event_id INTEGER PRIMARY KEY AUTOINCREMENT,
            asset_id TEXT NOT NULL,
            schema TEXT NOT NULL,
            event_type TEXT NOT NULL,
            detail_json TEXT NOT NULL,
            created_at TEXT NOT NULL
        );
        CREATE INDEX IF NOT EXISTS asset_events_by_asset ON asset_events(asset_id, event_id);
        CREATE TABLE IF NOT EXISTS core_appearance (
            singleton INTEGER PRIMARY KEY CHECK(singleton = 1),
            core_uuid TEXT NOT NULL,
            appearance_json TEXT NOT NULL,
            updated_at TEXT NOT NULL,
            revision INTEGER NOT NULL CHECK(revision >= 1)
        );
        CREATE TRIGGER IF NOT EXISTS asset_ownership_no_update
        BEFORE UPDATE ON asset_ownership
        BEGIN
            SELECT RAISE(ABORT, 'asset ownership is immutable');
        END;
        CREATE TRIGGER IF NOT EXISTS asset_ownership_no_delete
        BEFORE DELETE ON asset_ownership
        BEGIN
            SELECT RAISE(ABORT, 'asset ownership is immutable');
        END;
        CREATE TRIGGER IF NOT EXISTS asset_events_no_update
        BEFORE UPDATE ON asset_events
        BEGIN
            SELECT RAISE(ABORT, 'asset events are append-only');
        END;
        CREATE TRIGGER IF NOT EXISTS asset_events_no_delete
        BEFORE DELETE ON asset_events
        BEGIN
            SELECT RAISE(ABORT, 'asset events are append-only');
        END;",
    )?;
    Ok(())
}

fn verify_ownership(
    connection: &Connection,
    core_uuid: &str,
    instance_uuid: &str,
) -> Result<(), EmbodimentError> {
    let existing: Option<(String, String)> = connection
        .query_row(
            "SELECT core_uuid, database_instance_uuid FROM asset_ownership WHERE singleton = 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    match existing {
        None => {
            connection.execute(
                "INSERT INTO asset_ownership(
                    singleton, core_uuid, database_instance_uuid, created_at
                 ) VALUES(1, ?1, ?2, ?3)",
                params![core_uuid, instance_uuid, now_rfc3339()?],
            )?;
            Ok(())
        }
        Some((stored_core, stored_instance))
            if stored_core == core_uuid && stored_instance == instance_uuid =>
        {
            Ok(())
        }
        Some(_) => Err(EmbodimentError::OwnershipMismatch),
    }
}

const ASSET_COLUMNS: &str = "asset_id, core_uuid, kind, name, media_type, relative_path, \
                             sha256, bytes, tags_json, metadata_json, created_at, updated_at, \
                             deleted, revision";

fn read_asset(row: &rusqlite::Row<'_>) -> rusqlite::Result<Asset> {
    let kind: String = row.get(2)?;
    let kind = AssetKind::parse(&kind).map_err(|_| {
        rusqlite::Error::FromSqlConversionFailure(
            2,
            rusqlite::types::Type::Text,
            Box::new(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "invalid asset kind",
            )),
        )
    })?;
    let tags_json: String = row.get(8)?;
    let metadata_json: String = row.get(9)?;
    let bytes: i64 = row.get(7)?;
    let deleted: i64 = row.get(12)?;
    let revision: i64 = row.get(13)?;
    if bytes <= 0 || revision < 1 {
        return Err(rusqlite::Error::InvalidColumnType(
            7,
            "asset bounds".to_owned(),
            rusqlite::types::Type::Integer,
        ));
    }
    Ok(Asset {
        schema: ASSET_SCHEMA.to_owned(),
        asset_id: row.get(0)?,
        core_uuid: row.get(1)?,
        kind,
        name: row.get(3)?,
        media_type: row.get(4)?,
        relative_path: row.get(5)?,
        sha256: row.get(6)?,
        bytes: bytes as u64,
        tags: serde_json::from_str(&tags_json).map_err(|error| {
            rusqlite::Error::FromSqlConversionFailure(
                8,
                rusqlite::types::Type::Text,
                Box::new(error),
            )
        })?,
        metadata: serde_json::from_str(&metadata_json).map_err(|error| {
            rusqlite::Error::FromSqlConversionFailure(
                9,
                rusqlite::types::Type::Text,
                Box::new(error),
            )
        })?,
        created_at: row.get(10)?,
        updated_at: row.get(11)?,
        deleted: deleted != 0,
        revision: revision as u64,
    })
}

fn validate_asset_id(value: &str) -> Result<(), EmbodimentError> {
    let parsed = Uuid::parse_str(value).map_err(|_| EmbodimentError::InvalidInput("asset id"))?;
    if parsed.is_nil() {
        return Err(EmbodimentError::InvalidInput("asset id"));
    }
    Ok(())
}

fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), EmbodimentError> {
    let temporary = path.with_file_name(format!(
        ".{}.{}.tmp",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or(0)
    ));
    fs::write(&temporary, bytes)?;
    set_file_permissions(&temporary)?;
    match fs::rename(&temporary, path) {
        Ok(()) => Ok(()),
        Err(error) => {
            let _ = fs::remove_file(&temporary);
            Err(error.into())
        }
    }
}

#[cfg(unix)]
fn set_file_permissions(path: &Path) -> Result<(), EmbodimentError> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    Ok(())
}

#[cfg(not(unix))]
fn set_file_permissions(_path: &Path) -> Result<(), EmbodimentError> {
    Ok(())
}

fn sha256_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let digest = Sha256::digest(bytes);
    let mut output = String::with_capacity(64);
    for byte in digest {
        output.push(HEX[(byte >> 4) as usize] as char);
        output.push(HEX[(byte & 0x0f) as usize] as char);
    }
    output
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
