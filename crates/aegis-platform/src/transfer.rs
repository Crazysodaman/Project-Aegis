//! Platform-mediated cross-Core item transfers backed by `platform.db`.
//!
//! The Platform is the transfer authority. It records offers, enforces the
//! lifecycle, and performs delivery by coordinating the sender and recipient
//! [`PossessionsStore`] instances. Transfers are **not** a distributed
//! transaction: `platform.db` and each Core's `possessions.db` are committed
//! independently, so failures after delivery are handled with explicit,
//! best-effort compensation (see [`TransferStore::accept`]).

use aegis_contracts::core::validate_core_id;
use aegis_contracts::living::{TransferStatus, TransferType};
use aegis_core_sdk::{PossessionItem, PossessionsStore, SdkError};
use aegis_safe::SafeStore;
use aegis_storage::{now_rfc3339, open_initialized, StoragePaths, PLATFORM_APPLICATION_ID};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use uuid::Uuid;

/// Default number of transfer records returned by [`TransferStore::list`].
pub const TRANSFER_DEFAULT_LIMIT: usize = 50;
/// Maximum number of transfer records returned by [`TransferStore::list`].
pub const TRANSFER_MAX_LIMIT: usize = 200;
/// Maximum number of transfer events returned by [`TransferStore::events`].
pub const TRANSFER_MAX_EVENTS: usize = 500;
/// Maximum character length of a transfer note.
pub const TRANSFER_MAX_NOTE_CHARS: usize = 512;
/// Maximum character length of a stored transfer or instance identifier.
pub const TRANSFER_MAX_ID_CHARS: usize = 128;

const SCHEMA_SQL: &str = r#"
CREATE TABLE IF NOT EXISTS item_transfers (
    transfer_id TEXT NOT NULL PRIMARY KEY,
    from_core_uuid TEXT NOT NULL,
    to_core_uuid TEXT NOT NULL,
    instance_id TEXT NOT NULL,
    transfer_type TEXT NOT NULL,
    status TEXT NOT NULL CHECK(status IN (
        'pending', 'accepted', 'rejected', 'cancelled', 'failed'
    )),
    note TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    delivered_instance_id TEXT,
    revision INTEGER NOT NULL CHECK(revision >= 1)
);
CREATE INDEX IF NOT EXISTS item_transfers_status_idx
    ON item_transfers(status, updated_at);
CREATE INDEX IF NOT EXISTS item_transfers_from_idx
    ON item_transfers(from_core_uuid, updated_at);
CREATE INDEX IF NOT EXISTS item_transfers_to_idx
    ON item_transfers(to_core_uuid, updated_at);
CREATE TABLE IF NOT EXISTS item_transfer_events (
    sequence INTEGER NOT NULL PRIMARY KEY AUTOINCREMENT,
    transfer_id TEXT NOT NULL,
    event_type TEXT NOT NULL,
    status TEXT NOT NULL,
    detail_json TEXT NOT NULL,
    created_at TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS item_transfer_events_transfer_idx
    ON item_transfer_events(transfer_id, sequence);
CREATE TRIGGER IF NOT EXISTS item_transfer_events_append_only_update
BEFORE UPDATE ON item_transfer_events
BEGIN
    SELECT RAISE(ABORT, 'item_transfer_events is append-only');
END;
CREATE TRIGGER IF NOT EXISTS item_transfer_events_append_only_delete
BEFORE DELETE ON item_transfer_events
BEGIN
    SELECT RAISE(ABORT, 'item_transfer_events is append-only');
END;
"#;

/// One Platform-recorded cross-Core transfer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TransferRecord {
    pub transfer_id: String,
    pub from_core_uuid: String,
    pub to_core_uuid: String,
    pub instance_id: String,
    pub transfer_type: TransferType,
    pub status: TransferStatus,
    pub note: String,
    pub created_at: String,
    pub updated_at: String,
    pub delivered_instance_id: Option<String>,
    pub revision: u64,
}

/// One append-only transfer lifecycle event.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TransferEvent {
    pub sequence: u64,
    pub transfer_id: String,
    pub event_type: String,
    pub status: TransferStatus,
    pub detail: Value,
    pub created_at: String,
}

/// Errors returned by [`TransferStore`].
#[derive(Debug, thiserror::Error)]
pub enum TransferError {
    #[error("transfer storage failed: {0}")]
    Storage(#[from] aegis_storage::StorageError),
    #[error("transfer Core SDK operation failed: {0}")]
    Sdk(#[from] aegis_core_sdk::SdkError),
    #[error("transfer SQLite operation failed: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("transfer Safe operation failed: {0}")]
    Safe(#[from] aegis_safe::SafeError),
    #[error("transfer JSON encoding failed: {0}")]
    Json(#[from] serde_json::Error),
    #[error("transfer owner authorization failed")]
    Unauthorized,
    #[error("Core UUID is invalid or not canonical")]
    InvalidCoreUuid,
    #[error("transfer source and destination must be different Cores")]
    SameCore,
    #[error("transfer identifier is invalid")]
    InvalidTransferId,
    #[error("transfer item identifier is invalid")]
    InvalidInstanceId,
    #[error("transfer note is invalid")]
    InvalidNote,
    #[error("transfer limit is invalid")]
    InvalidLimit,
    #[error("transfer not found: {0}")]
    NotFound(String),
    #[error("transfer is not pending: {0}")]
    NotPending(String),
    #[error("transfer item is not owned by the sender")]
    ForeignItem,
    #[error("transfer item no longer exists at the sender")]
    MissingItem,
    #[error("transfer record is corrupt")]
    Corrupt,
    #[error("transfer platform update failed after delivery")]
    PlatformUpdateFailed,
}

/// Platform-owned store for cross-Core item transfers.
///
/// Every mutation requires Safe owner proof. Reads are unauthenticated, like the
/// rest of the Platform registry.
#[derive(Debug, Clone)]
pub struct TransferStore {
    paths: StoragePaths,
    safe: SafeStore,
}

impl TransferStore {
    pub fn new(paths: StoragePaths) -> Self {
        Self {
            safe: SafeStore::new(paths.clone()),
            paths,
        }
    }

    /// Create the additive `item_transfers` and `item_transfer_events` tables.
    pub fn initialize(&self) -> Result<(), TransferError> {
        let connection = self.connection()?;
        connection.execute_batch(SCHEMA_SQL)?;
        Ok(())
    }

    /// Offer an item from one Core to another.
    ///
    /// The sender store is opened and the item is exported to prove current
    /// ownership before a pending transfer is recorded. A foreign or missing
    /// item is rejected and nothing is written.
    pub fn offer(
        &self,
        owner_token: &str,
        from_core_uuid: &str,
        to_core_uuid: &str,
        instance_id: &str,
        transfer_type: TransferType,
        note: &str,
    ) -> Result<TransferRecord, TransferError> {
        self.require_owner(owner_token)?;
        let from_core_uuid = canonical_core_uuid(from_core_uuid)?;
        let to_core_uuid = canonical_core_uuid(to_core_uuid)?;
        if from_core_uuid == to_core_uuid {
            return Err(TransferError::SameCore);
        }
        validate_instance_id(instance_id)?;
        validate_note(note)?;

        let sender = PossessionsStore::open(self.paths.root(), &from_core_uuid)?;
        let _ = export_owned(&sender, instance_id)?;

        let transfer_id = Uuid::new_v4().to_string();
        let now = now_rfc3339()?;
        let record = TransferRecord {
            transfer_id: transfer_id.clone(),
            from_core_uuid,
            to_core_uuid,
            instance_id: instance_id.to_owned(),
            transfer_type,
            status: TransferStatus::Pending,
            note: note.to_owned(),
            created_at: now.clone(),
            updated_at: now.clone(),
            delivered_instance_id: None,
            revision: 1,
        };
        let detail = json!({
            "from_core_uuid": record.from_core_uuid,
            "to_core_uuid": record.to_core_uuid,
            "instance_id": record.instance_id,
            "transfer_type": transfer_type.as_str(),
            "note": record.note,
        });

        let mut connection = self.connection()?;
        let transaction = connection.transaction()?;
        insert_record(&transaction, &record)?;
        append_event(
            &transaction,
            &transfer_id,
            "offered",
            TransferStatus::Pending,
            &detail,
            &now,
        )?;
        transaction.commit()?;
        Ok(record)
    }

    /// Fetch one transfer by identifier.
    pub fn get(&self, transfer_id: &str) -> Result<Option<TransferRecord>, TransferError> {
        let connection = self.connection()?;
        let record = connection
            .query_row(
                "SELECT transfer_id, from_core_uuid, to_core_uuid, instance_id,
                        transfer_type, status, note, created_at, updated_at,
                        delivered_instance_id, revision
                 FROM item_transfers WHERE transfer_id = ?1",
                [transfer_id],
                read_record_row,
            )
            .optional()?;
        record.transpose()
    }

    /// List transfers, most recently updated first.
    pub fn list(&self, limit: usize) -> Result<Vec<TransferRecord>, TransferError> {
        let limit = effective_limit(limit, TRANSFER_MAX_LIMIT)?;
        let connection = self.connection()?;
        let mut statement = connection.prepare(
            "SELECT transfer_id, from_core_uuid, to_core_uuid, instance_id,
                    transfer_type, status, note, created_at, updated_at,
                    delivered_instance_id, revision
             FROM item_transfers
             ORDER BY updated_at DESC, transfer_id ASC LIMIT ?1",
        )?;
        let rows = statement.query_map([limit], read_record_row)?;
        let mut records = Vec::new();
        for row in rows {
            records.push(row??);
        }
        Ok(records)
    }

    /// List lifecycle events for one transfer in append order.
    pub fn events(
        &self,
        transfer_id: &str,
        limit: usize,
    ) -> Result<Vec<TransferEvent>, TransferError> {
        let limit = effective_limit(limit, TRANSFER_MAX_EVENTS)?;
        let connection = self.connection()?;
        let mut statement = connection.prepare(
            "SELECT sequence, transfer_id, event_type, status, detail_json, created_at
             FROM item_transfer_events
             WHERE transfer_id = ?1
             ORDER BY sequence ASC LIMIT ?2",
        )?;
        let rows = statement.query_map(params![transfer_id, limit], read_event_row)?;
        let mut events = Vec::new();
        for row in rows {
            events.push(row??);
        }
        Ok(events)
    }

    /// Accept a pending transfer and deliver the item to the recipient.
    ///
    /// The current sender payload is re-exported, imported at the recipient,
    /// and removed from the sender. The Platform row becomes `accepted` only
    /// after delivery succeeds. Any failure before that point leaves the item
    /// undelivered and records a `failed` event (or rolls the import back when
    /// sender removal fails).
    pub fn accept(
        &self,
        owner_token: &str,
        transfer_id: &str,
    ) -> Result<TransferRecord, TransferError> {
        self.require_owner(owner_token)?;
        validate_transfer_id(transfer_id)?;
        let record = self
            .get(transfer_id)?
            .ok_or_else(|| TransferError::NotFound(transfer_id.to_owned()))?;
        if record.status != TransferStatus::Pending {
            return Err(TransferError::NotPending(record.status.as_str().to_owned()));
        }

        let sender = match PossessionsStore::open(self.paths.root(), &record.from_core_uuid) {
            Ok(sender) => sender,
            Err(error) => {
                let _ = self.mark_failed(&record, "sender_store_unavailable");
                return Err(TransferError::Sdk(error));
            }
        };
        let payload = match export_owned(&sender, &record.instance_id) {
            Ok(payload) => payload,
            Err(error) => {
                let reason = match &error {
                    TransferError::MissingItem => "item_missing",
                    TransferError::ForeignItem => "stale_ownership",
                    _ => "export_failed",
                };
                let _ = self.mark_failed(&record, reason);
                return Err(error);
            }
        };

        let recipient = match PossessionsStore::open(self.paths.root(), &record.to_core_uuid) {
            Ok(recipient) => recipient,
            Err(error) => {
                let _ = self.mark_failed(&record, "recipient_store_unavailable");
                return Err(TransferError::Sdk(error));
            }
        };
        let imported = match recipient.import_transferred_item(
            &payload,
            &record.from_core_uuid,
            transfer_id,
        ) {
            Ok(imported) => imported,
            Err(error) => {
                let _ = self.mark_failed(&record, "recipient_import_failed");
                return Err(TransferError::Sdk(error));
            }
        };

        if let Err(error) = sender.remove_transferred_item(&record.instance_id, transfer_id) {
            let _ = recipient.rollback_imported_item(imported.instance_id(), transfer_id);
            let _ = self.mark_failed(&record, "sender_removal_failed");
            return Err(TransferError::Sdk(error));
        }

        // Delivery has happened. Never report `accepted` if the Platform row
        // cannot be updated; attempt to restore both Core databases instead.
        match self.complete_accept(&record, imported.instance_id()) {
            Ok(updated) => Ok(updated),
            Err(_) => {
                self.compensate_delivery(&record, &sender, &recipient, imported.instance_id());
                let _ = self.mark_failed(&record, "platform_update_failed");
                Err(TransferError::PlatformUpdateFailed)
            }
        }
    }

    /// Reject a pending transfer without delivering anything.
    pub fn reject(
        &self,
        owner_token: &str,
        transfer_id: &str,
    ) -> Result<TransferRecord, TransferError> {
        self.require_owner(owner_token)?;
        validate_transfer_id(transfer_id)?;
        self.transition_terminal(transfer_id, TransferStatus::Rejected, "rejected")
    }

    /// Cancel a pending transfer without delivering anything.
    pub fn cancel(
        &self,
        owner_token: &str,
        transfer_id: &str,
    ) -> Result<TransferRecord, TransferError> {
        self.require_owner(owner_token)?;
        validate_transfer_id(transfer_id)?;
        self.transition_terminal(transfer_id, TransferStatus::Cancelled, "cancelled")
    }

    fn transition_terminal(
        &self,
        transfer_id: &str,
        next: TransferStatus,
        event_type: &str,
    ) -> Result<TransferRecord, TransferError> {
        let record = self
            .get(transfer_id)?
            .ok_or_else(|| TransferError::NotFound(transfer_id.to_owned()))?;
        if record.status != TransferStatus::Pending {
            return Err(TransferError::NotPending(record.status.as_str().to_owned()));
        }
        let detail = json!({
            "from_core_uuid": record.from_core_uuid,
            "to_core_uuid": record.to_core_uuid,
            "instance_id": record.instance_id,
        });
        let now = self.apply_transition(
            transfer_id,
            TransferStatus::Pending,
            next,
            None,
            event_type,
            &detail,
        )?;
        Ok(TransferRecord {
            status: next,
            updated_at: now,
            delivered_instance_id: None,
            revision: record.revision.saturating_add(1),
            ..record
        })
    }

    fn complete_accept(
        &self,
        record: &TransferRecord,
        delivered_instance_id: &str,
    ) -> Result<TransferRecord, TransferError> {
        let detail = json!({
            "from_core_uuid": record.from_core_uuid,
            "to_core_uuid": record.to_core_uuid,
            "source_instance_id": record.instance_id,
            "delivered_instance_id": delivered_instance_id,
        });
        let now = self.apply_transition(
            &record.transfer_id,
            TransferStatus::Pending,
            TransferStatus::Accepted,
            Some(delivered_instance_id),
            "accepted",
            &detail,
        )?;
        Ok(TransferRecord {
            status: TransferStatus::Accepted,
            updated_at: now,
            delivered_instance_id: Some(delivered_instance_id.to_owned()),
            revision: record.revision.saturating_add(1),
            ..record.clone()
        })
    }

    fn mark_failed(
        &self,
        record: &TransferRecord,
        reason: &str,
    ) -> Result<TransferRecord, TransferError> {
        let detail = json!({ "reason": reason });
        let now = self.apply_transition(
            &record.transfer_id,
            TransferStatus::Pending,
            TransferStatus::Failed,
            None,
            "failed",
            &detail,
        )?;
        Ok(TransferRecord {
            status: TransferStatus::Failed,
            updated_at: now,
            delivered_instance_id: None,
            revision: record.revision.saturating_add(1),
            ..record.clone()
        })
    }

    #[allow(clippy::too_many_arguments)]
    fn apply_transition(
        &self,
        transfer_id: &str,
        expected: TransferStatus,
        next: TransferStatus,
        delivered_instance_id: Option<&str>,
        event_type: &str,
        detail: &Value,
    ) -> Result<String, TransferError> {
        let now = now_rfc3339()?;
        let mut connection = self.connection()?;
        let transaction = connection.transaction()?;
        let changed = transaction.execute(
            "UPDATE item_transfers
             SET status = ?1, updated_at = ?2, delivered_instance_id = ?3,
                 revision = revision + 1
             WHERE transfer_id = ?4 AND status = ?5",
            params![
                next.as_str(),
                &now,
                delivered_instance_id,
                transfer_id,
                expected.as_str(),
            ],
        )?;
        if changed != 1 {
            return Err(TransferError::NotPending(expected.as_str().to_owned()));
        }
        append_event(&transaction, transfer_id, event_type, next, detail, &now)?;
        transaction.commit()?;
        Ok(now)
    }

    /// Best-effort reversal of a delivery whose Platform update failed.
    ///
    /// There is no cross-database transaction, so this moves the delivered item
    /// back to the sender and then removes it from the recipient. Every step is
    /// best effort; a partial failure leaves the databases for manual
    /// reconciliation and is reported to the caller as an error.
    fn compensate_delivery(
        &self,
        record: &TransferRecord,
        sender: &PossessionsStore,
        recipient: &PossessionsStore,
        delivered_instance_id: &str,
    ) {
        if let Ok(payload) = recipient.export_for_transfer(delivered_instance_id) {
            if sender
                .import_transferred_item(&payload, &record.to_core_uuid, &record.transfer_id)
                .is_ok()
            {
                let _ =
                    recipient.remove_transferred_item(delivered_instance_id, &record.transfer_id);
            }
        }
    }

    fn require_owner(&self, owner_token: &str) -> Result<(), TransferError> {
        match self.safe.verify_owner(owner_token) {
            Ok(true) => Ok(()),
            Ok(false) => Err(TransferError::Unauthorized),
            Err(error) => Err(TransferError::Safe(error)),
        }
    }

    fn connection(&self) -> Result<Connection, TransferError> {
        Ok(open_initialized(
            &self.paths.platform_db(),
            PLATFORM_APPLICATION_ID,
            "platform",
        )?)
    }
}

fn export_owned(
    store: &PossessionsStore,
    instance_id: &str,
) -> Result<PossessionItem, TransferError> {
    match store.export_for_transfer(instance_id) {
        Ok(item) => Ok(item),
        Err(SdkError::NotFound) => Err(TransferError::MissingItem),
        Err(SdkError::OwnershipMismatch) | Err(SdkError::ForeignCore) => {
            Err(TransferError::ForeignItem)
        }
        Err(error) => Err(TransferError::Sdk(error)),
    }
}

fn insert_record(connection: &Connection, record: &TransferRecord) -> Result<(), TransferError> {
    let revision = revision_value(record.revision)?;
    connection.execute(
        "INSERT INTO item_transfers(
            transfer_id, from_core_uuid, to_core_uuid, instance_id, transfer_type,
            status, note, created_at, updated_at, delivered_instance_id, revision
         ) VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
        params![
            record.transfer_id,
            record.from_core_uuid,
            record.to_core_uuid,
            record.instance_id,
            record.transfer_type.as_str(),
            record.status.as_str(),
            record.note,
            record.created_at,
            record.updated_at,
            record.delivered_instance_id,
            revision,
        ],
    )?;
    Ok(())
}

fn append_event(
    connection: &Connection,
    transfer_id: &str,
    event_type: &str,
    status: TransferStatus,
    detail: &Value,
    created_at: &str,
) -> Result<(), TransferError> {
    let detail_json = serde_json::to_string(detail)?;
    connection.execute(
        "INSERT INTO item_transfer_events(
            transfer_id, event_type, status, detail_json, created_at
         ) VALUES(?1, ?2, ?3, ?4, ?5)",
        params![
            transfer_id,
            event_type,
            status.as_str(),
            detail_json,
            created_at,
        ],
    )?;
    Ok(())
}

fn read_record_row(
    row: &rusqlite::Row<'_>,
) -> rusqlite::Result<Result<TransferRecord, TransferError>> {
    let transfer_id: String = row.get(0)?;
    let from_core_uuid: String = row.get(1)?;
    let to_core_uuid: String = row.get(2)?;
    let instance_id: String = row.get(3)?;
    let transfer_type: String = row.get(4)?;
    let status: String = row.get(5)?;
    let note: String = row.get(6)?;
    let created_at: String = row.get(7)?;
    let updated_at: String = row.get(8)?;
    let delivered_instance_id: Option<String> = row.get(9)?;
    let revision: i64 = row.get(10)?;
    Ok((|| {
        let transfer_type = TransferType::parse(&transfer_type).ok_or(TransferError::Corrupt)?;
        let status = TransferStatus::parse(&status).ok_or(TransferError::Corrupt)?;
        let revision = u64::try_from(revision).map_err(|_| TransferError::Corrupt)?;
        Ok(TransferRecord {
            transfer_id,
            from_core_uuid,
            to_core_uuid,
            instance_id,
            transfer_type,
            status,
            note,
            created_at,
            updated_at,
            delivered_instance_id,
            revision,
        })
    })())
}

fn read_event_row(
    row: &rusqlite::Row<'_>,
) -> rusqlite::Result<Result<TransferEvent, TransferError>> {
    let sequence: i64 = row.get(0)?;
    let transfer_id: String = row.get(1)?;
    let event_type: String = row.get(2)?;
    let status: String = row.get(3)?;
    let detail_json: String = row.get(4)?;
    let created_at: String = row.get(5)?;
    Ok((|| {
        let sequence = u64::try_from(sequence).map_err(|_| TransferError::Corrupt)?;
        let status = TransferStatus::parse(&status).ok_or(TransferError::Corrupt)?;
        let detail = serde_json::from_str(&detail_json).map_err(|_| TransferError::Corrupt)?;
        Ok(TransferEvent {
            sequence,
            transfer_id,
            event_type,
            status,
            detail,
            created_at,
        })
    })())
}

fn canonical_core_uuid(value: &str) -> Result<String, TransferError> {
    let parsed = validate_core_id(value).map_err(|_| TransferError::InvalidCoreUuid)?;
    let canonical = parsed.hyphenated().to_string();
    if canonical != value {
        return Err(TransferError::InvalidCoreUuid);
    }
    Ok(canonical)
}

fn validate_transfer_id(value: &str) -> Result<(), TransferError> {
    if value.is_empty()
        || value.chars().count() > TRANSFER_MAX_ID_CHARS
        || value.chars().any(char::is_control)
    {
        return Err(TransferError::InvalidTransferId);
    }
    Ok(())
}

fn validate_instance_id(value: &str) -> Result<(), TransferError> {
    if value.is_empty()
        || value.chars().count() > TRANSFER_MAX_ID_CHARS
        || value.chars().any(char::is_control)
    {
        return Err(TransferError::InvalidInstanceId);
    }
    Ok(())
}

fn validate_note(note: &str) -> Result<(), TransferError> {
    if note.chars().count() > TRANSFER_MAX_NOTE_CHARS || note.chars().any(char::is_control) {
        return Err(TransferError::InvalidNote);
    }
    Ok(())
}

fn effective_limit(limit: usize, maximum: usize) -> Result<i64, TransferError> {
    if limit == 0 || limit > maximum {
        return Err(TransferError::InvalidLimit);
    }
    i64::try_from(limit).map_err(|_| TransferError::InvalidLimit)
}

fn revision_value(revision: u64) -> Result<i64, TransferError> {
    i64::try_from(revision).map_err(|_| TransferError::Corrupt)
}
