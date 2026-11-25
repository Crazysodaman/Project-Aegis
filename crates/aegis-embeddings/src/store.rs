//! Per-Core embedding cache.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use aegis_contracts::core::canonicalize_core_id;
use aegis_core_storage::{CoreDatabase, CoreStorageLayout};
use aegis_storage::{now_rfc3339, open_initialized};
use rusqlite::{params, Connection, OptionalExtension};

use crate::error::EmbeddingError;

/// File name of the per-Core embedding database.
pub const EMBEDDING_DB_FILE: &str = "embeddings.db";
/// SQLite application marker for an embedding database ("AEGE").
pub const EMBEDDING_APPLICATION_ID: i64 = 0x4145_4745;
/// Component name registered in the shared `aegis_schema` table.
pub const EMBEDDING_COMPONENT: &str = "embeddings";
/// Maximum accepted embedding dimensions.
pub const MAX_DIMENSIONS: usize = 8192;

#[derive(Debug, Clone, PartialEq)]
pub struct EmbeddingRecord {
    pub memory_id: String,
    pub provider: String,
    pub model: String,
    pub dimensions: usize,
    pub vector: Vec<f32>,
    pub content_sha256: String,
    pub updated_at: String,
}

/// Embedding cache bound to one Core.
#[derive(Debug)]
pub struct EmbeddingStore {
    connection: Connection,
    core_uuid: String,
    instance_uuid: String,
    path: PathBuf,
}

impl EmbeddingStore {
    pub fn open(root: impl AsRef<Path>, core_uuid: &str) -> Result<Self, EmbeddingError> {
        let core_uuid =
            canonicalize_core_id(core_uuid).map_err(|_| EmbeddingError::OwnershipMismatch)?;
        let layout = CoreStorageLayout::new(root.as_ref())?;
        let snapshot = CoreDatabase::read_snapshot(&layout, &core_uuid)?;
        if snapshot.identity.core_uuid != core_uuid {
            return Err(EmbeddingError::OwnershipMismatch);
        }
        let core_dir = layout.core_dir(&core_uuid)?;
        let path = core_dir.join(EMBEDDING_DB_FILE);
        let connection = open_initialized(&path, EMBEDDING_APPLICATION_ID, EMBEDDING_COMPONENT)?;
        ensure_tables(&connection)?;
        verify_ownership(
            &connection,
            &core_uuid,
            &snapshot.metadata.database_instance_uuid,
        )?;
        Ok(Self {
            connection,
            core_uuid,
            instance_uuid: snapshot.metadata.database_instance_uuid,
            path,
        })
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

    /// Return a cached vector only when provider, model, and content digest all
    /// match the requested memory.
    pub fn get(
        &self,
        memory_id: &str,
        provider: &str,
        model: &str,
        content_sha256: &str,
    ) -> Result<Option<Vec<f32>>, EmbeddingError> {
        let row: Option<(String, String, String, i64, Vec<u8>)> = self
            .connection
            .query_row(
                "SELECT provider, model, content_sha256, dimensions, vector
                 FROM memory_embeddings WHERE memory_id = ?1",
                [memory_id],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                    ))
                },
            )
            .optional()?;
        let Some((stored_provider, stored_model, stored_sha, dimensions, bytes)) = row else {
            return Ok(None);
        };
        if stored_provider != provider || stored_model != model || stored_sha != content_sha256 {
            return Ok(None);
        }
        decode_vector(&bytes, dimensions)
    }

    pub fn put(
        &self,
        memory_id: &str,
        provider: &str,
        model: &str,
        content_sha256: &str,
        vector: &[f32],
    ) -> Result<(), EmbeddingError> {
        if vector.is_empty()
            || vector.len() > MAX_DIMENSIONS
            || vector.iter().any(|value| !value.is_finite())
        {
            return Err(EmbeddingError::Dimensions);
        }
        let bytes = encode_vector(vector);
        self.connection.execute(
            "INSERT INTO memory_embeddings(
                memory_id, provider, model, dimensions, vector, content_sha256, updated_at
             ) VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT(memory_id) DO UPDATE SET
                 provider = excluded.provider,
                 model = excluded.model,
                 dimensions = excluded.dimensions,
                 vector = excluded.vector,
                 content_sha256 = excluded.content_sha256,
                 updated_at = excluded.updated_at",
            params![
                memory_id,
                provider,
                model,
                vector.len() as i64,
                bytes,
                content_sha256,
                now_rfc3339()?,
            ],
        )?;
        Ok(())
    }

    pub fn delete(&self, memory_id: &str) -> Result<(), EmbeddingError> {
        self.connection.execute(
            "DELETE FROM memory_embeddings WHERE memory_id = ?1",
            [memory_id],
        )?;
        Ok(())
    }

    pub fn clear_provider(&self, provider: &str, model: &str) -> Result<(), EmbeddingError> {
        self.connection.execute(
            "DELETE FROM memory_embeddings WHERE provider = ?1 AND model = ?2",
            params![provider, model],
        )?;
        Ok(())
    }

    pub fn count(&self) -> Result<u64, EmbeddingError> {
        let count: i64 =
            self.connection
                .query_row("SELECT COUNT(*) FROM memory_embeddings", [], |row| {
                    row.get(0)
                })?;
        Ok(count as u64)
    }

    /// Delete embeddings for memories no longer present in the Core.
    pub fn prune(&self, valid_memory_ids: &[String]) -> Result<usize, EmbeddingError> {
        let valid: BTreeSet<&str> = valid_memory_ids.iter().map(String::as_str).collect();
        let mut statement = self
            .connection
            .prepare("SELECT memory_id FROM memory_embeddings")?;
        let mut rows = statement.query([])?;
        let mut stale = Vec::new();
        while let Some(row) = rows.next()? {
            let memory_id: String = row.get(0)?;
            if !valid.contains(memory_id.as_str()) {
                stale.push(memory_id);
            }
        }
        drop(rows);
        drop(statement);
        for memory_id in &stale {
            self.delete(memory_id)?;
        }
        Ok(stale.len())
    }

    pub fn get_record(&self, memory_id: &str) -> Result<Option<EmbeddingRecord>, EmbeddingError> {
        let row: Option<(String, String, i64, Vec<u8>, String, String)> = self
            .connection
            .query_row(
                "SELECT provider, model, dimensions, vector, content_sha256, updated_at
                 FROM memory_embeddings WHERE memory_id = ?1",
                [memory_id],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                        row.get(5)?,
                    ))
                },
            )
            .optional()?;
        let Some((provider, model, dimensions, bytes, content_sha256, updated_at)) = row else {
            return Ok(None);
        };
        let Some(vector) = decode_vector(&bytes, dimensions)? else {
            return Ok(None);
        };
        Ok(Some(EmbeddingRecord {
            memory_id: memory_id.to_owned(),
            provider,
            model,
            dimensions: vector.len(),
            vector,
            content_sha256,
            updated_at,
        }))
    }
}

fn ensure_tables(connection: &Connection) -> Result<(), EmbeddingError> {
    connection.execute_batch(
        "CREATE TABLE IF NOT EXISTS embedding_ownership (
            singleton INTEGER PRIMARY KEY CHECK(singleton = 1),
            core_uuid TEXT NOT NULL UNIQUE,
            database_instance_uuid TEXT NOT NULL,
            created_at TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS memory_embeddings (
            memory_id TEXT PRIMARY KEY,
            provider TEXT NOT NULL,
            model TEXT NOT NULL,
            dimensions INTEGER NOT NULL CHECK(dimensions > 0 AND dimensions <= 8192),
            vector BLOB NOT NULL,
            content_sha256 TEXT NOT NULL,
            updated_at TEXT NOT NULL
        );
        CREATE TRIGGER IF NOT EXISTS embedding_ownership_no_update
        BEFORE UPDATE ON embedding_ownership
        BEGIN
            SELECT RAISE(ABORT, 'embedding ownership is immutable');
        END;
        CREATE TRIGGER IF NOT EXISTS embedding_ownership_no_delete
        BEFORE DELETE ON embedding_ownership
        BEGIN
            SELECT RAISE(ABORT, 'embedding ownership is immutable');
        END;",
    )?;
    Ok(())
}

fn verify_ownership(
    connection: &Connection,
    core_uuid: &str,
    instance_uuid: &str,
) -> Result<(), EmbeddingError> {
    let existing: Option<(String, String)> = connection
        .query_row(
            "SELECT core_uuid, database_instance_uuid FROM embedding_ownership WHERE singleton = 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    match existing {
        None => {
            connection.execute(
                "INSERT INTO embedding_ownership(
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
        Some(_) => Err(EmbeddingError::OwnershipMismatch),
    }
}

fn encode_vector(vector: &[f32]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(vector.len() * 4);
    for value in vector {
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    bytes
}

fn decode_vector(bytes: &[u8], dimensions: i64) -> Result<Option<Vec<f32>>, EmbeddingError> {
    if dimensions <= 0 || dimensions as usize > MAX_DIMENSIONS {
        return Err(EmbeddingError::Dimensions);
    }
    let dimensions = dimensions as usize;
    if bytes.len() != dimensions * 4 {
        return Err(EmbeddingError::Dimensions);
    }
    let mut vector = Vec::with_capacity(dimensions);
    for chunk in bytes.chunks_exact(4) {
        let value = f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
        if !value.is_finite() {
            return Err(EmbeddingError::Dimensions);
        }
        vector.push(value);
    }
    if vector.is_empty() {
        Ok(None)
    } else {
        Ok(Some(vector))
    }
}
