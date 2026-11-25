use aegis_contracts::status::{StopStatus, STOP_STATUS_SCHEMA};
use aegis_storage::{now_rfc3339, open_initialized, StoragePaths, SAFE_APPLICATION_ID};
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::json;
use sha2::{Digest, Sha256};

use crate::owner_proof::{LocalOwnerProof, OWNER_PROOF_KIND};
use crate::SafeError;

const GENESIS_HASH: &str = "GENESIS";

#[derive(Debug, Clone)]
pub struct SafeStore {
    paths: StoragePaths,
}

impl SafeStore {
    pub fn new(paths: StoragePaths) -> Self {
        Self { paths }
    }

    pub fn initialize(&self) -> Result<(), SafeError> {
        let connection = open_initialized(&self.paths.safe_db(), SAFE_APPLICATION_ID, "safe")?;
        connection.execute_batch(
            "CREATE TABLE IF NOT EXISTS safe_stop (
                singleton INTEGER PRIMARY KEY CHECK(singleton = 1),
                active INTEGER NOT NULL CHECK(active IN (0, 1)),
                revision INTEGER NOT NULL CHECK(revision >= 0),
                updated_at TEXT NOT NULL,
                updated_by TEXT NOT NULL,
                reason TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS safe_owner (
                singleton INTEGER PRIMARY KEY CHECK(singleton = 1),
                proof_kind TEXT NOT NULL,
                proof_file TEXT NOT NULL,
                created_at TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS safe_audit (
                sequence INTEGER PRIMARY KEY AUTOINCREMENT,
                event_type TEXT NOT NULL,
                detail_json TEXT NOT NULL,
                prev_hash TEXT NOT NULL,
                hash TEXT NOT NULL,
                created_at TEXT NOT NULL
            );",
        )?;
        let stop_count: i64 =
            connection.query_row("SELECT COUNT(*) FROM safe_stop", [], |row| row.get(0))?;
        if stop_count == 0 {
            connection.execute(
                "INSERT INTO safe_stop(
                    singleton, active, revision, updated_at, updated_by, reason
                ) VALUES(1, 0, 0, ?1, 'system', 'initial')",
                params![now_rfc3339()?],
            )?;
        }
        let audit_count: i64 =
            connection.query_row("SELECT COUNT(*) FROM safe_audit", [], |row| row.get(0))?;
        if audit_count == 0 {
            self.append_audit(
                &connection,
                "safe.initialized",
                json!({"owner_proof_kind": OWNER_PROOF_KIND}),
            )?;
        }
        Ok(())
    }

    /// Ensure the Safe store exists and owner proof is initialized.
    pub fn start(&self) -> Result<(), SafeError> {
        self.initialize()?;
        if !self.paths.owner_proof().is_file() {
            return Err(SafeError::OwnerNotInitialized);
        }
        Ok(())
    }

    pub fn initialize_owner(&self) -> Result<String, SafeError> {
        self.initialize()?;
        if self.paths.owner_proof().exists() {
            return Err(SafeError::OwnerAlreadyInitialized);
        }
        let proof = LocalOwnerProof::new(self.paths.owner_proof());
        let token = proof.initialize()?;
        let connection = open_initialized(&self.paths.safe_db(), SAFE_APPLICATION_ID, "safe")?;
        let owner_count: i64 =
            connection.query_row("SELECT COUNT(*) FROM safe_owner", [], |row| row.get(0))?;
        if owner_count != 0 {
            let _ = std::fs::remove_file(self.paths.owner_proof());
            return Err(SafeError::OwnerAlreadyInitialized);
        }
        if let Err(error) = connection.execute(
            "INSERT INTO safe_owner(singleton, proof_kind, proof_file, created_at)
             VALUES(1, ?1, 'owner.proof', ?2)",
            params![OWNER_PROOF_KIND, now_rfc3339()?],
        ) {
            let _ = std::fs::remove_file(self.paths.owner_proof());
            return Err(SafeError::Sqlite(error));
        }
        self.append_audit(
            &connection,
            "safe.owner.initialized",
            json!({"proof_kind": OWNER_PROOF_KIND}),
        )?;
        Ok(token)
    }

    pub fn owner_token(&self) -> Result<String, SafeError> {
        if !self.paths.owner_proof().is_file() {
            return Err(SafeError::OwnerNotInitialized);
        }
        LocalOwnerProof::new(self.paths.owner_proof()).load_token()
    }

    pub fn verify_owner(&self, token: &str) -> Result<bool, SafeError> {
        if !self.paths.owner_proof().is_file() {
            return Err(SafeError::OwnerNotInitialized);
        }
        LocalOwnerProof::new(self.paths.owner_proof()).verify(token)
    }

    pub fn stop_status(&self) -> Result<StopStatus, SafeError> {
        let connection = open_initialized(&self.paths.safe_db(), SAFE_APPLICATION_ID, "safe")?;
        read_stop_status(&connection)
    }

    pub fn set_stop(
        &self,
        updated_by: &str,
        reason: &str,
        owner_token: &str,
    ) -> Result<StopStatus, SafeError> {
        validate_text(updated_by, 128, "updated_by")?;
        validate_text(reason, 512, "reason")?;
        if !self.verify_owner(owner_token)? {
            return Err(SafeError::Unauthorized);
        }
        let mut connection = open_initialized(&self.paths.safe_db(), SAFE_APPLICATION_ID, "safe")?;
        let transaction = connection.transaction()?;
        let current = read_stop_status(&transaction)?;
        let revision = current.revision + 1;
        let updated_at = now_rfc3339()?;
        transaction.execute(
            "UPDATE safe_stop SET active = 1, revision = ?1, updated_at = ?2,
             updated_by = ?3, reason = ?4 WHERE singleton = 1",
            params![revision as i64, updated_at, updated_by, reason],
        )?;
        self.append_audit(
            &transaction,
            "safe.stop.set",
            json!({"active": true, "revision": revision, "updated_by": updated_by}),
        )?;
        transaction.commit()?;
        Ok(StopStatus {
            schema: STOP_STATUS_SCHEMA.to_owned(),
            active: true,
            revision,
            updated_at,
            updated_by: updated_by.to_owned(),
            reason: reason.to_owned(),
        })
    }

    pub fn clear_stop(&self, updated_by: &str, owner_token: &str) -> Result<StopStatus, SafeError> {
        validate_text(updated_by, 128, "updated_by")?;
        if !self.verify_owner(owner_token)? {
            return Err(SafeError::Unauthorized);
        }
        let mut connection = open_initialized(&self.paths.safe_db(), SAFE_APPLICATION_ID, "safe")?;
        let transaction = connection.transaction()?;
        let current = read_stop_status(&transaction)?;
        let revision = current.revision + 1;
        let updated_at = now_rfc3339()?;
        transaction.execute(
            "UPDATE safe_stop SET active = 0, revision = ?1, updated_at = ?2,
             updated_by = ?3, reason = 'cleared by owner' WHERE singleton = 1",
            params![revision as i64, updated_at, updated_by],
        )?;
        self.append_audit(
            &transaction,
            "safe.stop.clear",
            json!({"active": false, "revision": revision, "updated_by": updated_by}),
        )?;
        transaction.commit()?;
        Ok(StopStatus {
            schema: STOP_STATUS_SCHEMA.to_owned(),
            active: false,
            revision,
            updated_at,
            updated_by: updated_by.to_owned(),
            reason: "cleared by owner".to_owned(),
        })
    }

    pub fn audit_count(&self) -> Result<u64, SafeError> {
        let connection = open_initialized(&self.paths.safe_db(), SAFE_APPLICATION_ID, "safe")?;
        let count: i64 =
            connection.query_row("SELECT COUNT(*) FROM safe_audit", [], |row| row.get(0))?;
        Ok(count as u64)
    }

    pub fn audit_chain_valid(&self) -> Result<bool, SafeError> {
        let connection = open_initialized(&self.paths.safe_db(), SAFE_APPLICATION_ID, "safe")?;
        let mut statement = connection.prepare(
            "SELECT event_type, detail_json, prev_hash, hash, created_at
             FROM safe_audit ORDER BY sequence ASC",
        )?;
        let rows = statement.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
            ))
        })?;
        let mut previous = GENESIS_HASH.to_owned();
        for row in rows {
            let (event_type, detail_json, prev_hash, hash, created_at) = row?;
            if prev_hash != previous {
                return Ok(false);
            }
            let expected = audit_hash(&event_type, &detail_json, &prev_hash, &created_at);
            if hash != expected {
                return Ok(false);
            }
            previous = hash;
        }
        Ok(true)
    }

    /// Append an audited security/operational event.
    ///
    /// This is an internal trusted-code entry point. Callers must not include
    /// owner proof, credentials, private conversation text, or full memory
    /// content in `detail`.
    pub fn record_event(
        &self,
        event_type: &str,
        detail: serde_json::Value,
    ) -> Result<(), SafeError> {
        self.initialize()?;
        let trimmed = event_type.trim();
        if trimmed.is_empty() || trimmed.len() > 128 || trimmed.chars().any(char::is_control) {
            return Err(SafeError::InvalidInput("audit event type"));
        }
        let encoded =
            serde_json::to_string(&detail).map_err(|_| SafeError::InvalidInput("audit detail"))?;
        if encoded.len() > 8192 {
            return Err(SafeError::InvalidInput("audit detail"));
        }
        let connection = open_initialized(&self.paths.safe_db(), SAFE_APPLICATION_ID, "safe")?;
        self.append_audit(&connection, trimmed, detail)
    }

    fn append_audit(
        &self,
        connection: &Connection,
        event_type: &str,
        detail: serde_json::Value,
    ) -> Result<(), SafeError> {
        let previous: Option<String> = connection
            .query_row(
                "SELECT hash FROM safe_audit ORDER BY sequence DESC LIMIT 1",
                [],
                |row| row.get(0),
            )
            .optional()?;
        let prev_hash = previous.unwrap_or_else(|| GENESIS_HASH.to_owned());
        let detail_json =
            serde_json::to_string(&detail).map_err(|_| SafeError::InvalidInput("audit detail"))?;
        let created_at = now_rfc3339()?;
        let hash = audit_hash(event_type, &detail_json, &prev_hash, &created_at);
        connection.execute(
            "INSERT INTO safe_audit(event_type, detail_json, prev_hash, hash, created_at)
             VALUES(?1, ?2, ?3, ?4, ?5)",
            params![event_type, detail_json, prev_hash, hash, created_at],
        )?;
        Ok(())
    }
}

fn read_stop_status(connection: &Connection) -> Result<StopStatus, SafeError> {
    let row = connection.query_row(
        "SELECT active, revision, updated_at, updated_by, reason
         FROM safe_stop WHERE singleton = 1",
        [],
        |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
            ))
        },
    )?;
    if row.1 < 0 {
        return Err(SafeError::InvalidInput("stop revision"));
    }
    Ok(StopStatus {
        schema: STOP_STATUS_SCHEMA.to_owned(),
        active: row.0 != 0,
        revision: row.1 as u64,
        updated_at: row.2,
        updated_by: row.3,
        reason: row.4,
    })
}

fn validate_text(value: &str, maximum: usize, field: &'static str) -> Result<(), SafeError> {
    let trimmed = value.trim();
    if trimmed.is_empty() || trimmed.len() > maximum {
        return Err(SafeError::InvalidInput(field));
    }
    Ok(())
}

fn audit_hash(event_type: &str, detail_json: &str, prev_hash: &str, created_at: &str) -> String {
    let payload = format!("{event_type}\n{detail_json}\n{prev_hash}\n{created_at}");
    let digest = Sha256::digest(payload.as_bytes());
    let mut output = String::with_capacity(64);
    const HEX: &[u8; 16] = b"0123456789abcdef";
    for byte in digest {
        output.push(HEX[(byte >> 4) as usize] as char);
        output.push(HEX[(byte & 0x0f) as usize] as char);
    }
    output
}
