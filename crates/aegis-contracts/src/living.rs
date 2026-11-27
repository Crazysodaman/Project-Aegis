//! Living-environment contracts shared by Platform and Cores.
//!
//! These are authority-free data contracts for the original Aegis vision:
//! Core-authored items and possessions, outfits, written documents, world
//! objects, action definitions, consent grants and personal goals.
//!
//! Persistence, permissions and execution live in the owning crates; these
//! types only define shape and bounds so independent subsystems agree.

use serde::{Deserialize, Serialize};

pub const LIVING_CONTRACT_VERSION: &str = "aegis.living.v1";
pub const MAX_ID_CHARS: usize = 128;
pub const MAX_NAME_CHARS: usize = 160;
pub const MAX_TEXT_CHARS: usize = 4096;
pub const MAX_DOCUMENT_BODY_CHARS: usize = 1024 * 1024;
pub const MAX_TAGS: usize = 32;
pub const MAX_TAG_CHARS: usize = 64;
pub const MAX_LIST_ITEMS: usize = 256;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Visibility {
    Private,
    Shared,
    OwnerOnly,
    PublicCatalog,
}

impl Visibility {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Private => "private",
            Self::Shared => "shared",
            Self::OwnerOnly => "owner_only",
            Self::PublicCatalog => "public_catalog",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ItemCategory {
    Garment,
    Footwear,
    Accessory,
    Armor,
    Jewelry,
    Container,
    Book,
    Document,
    Artwork,
    Decoration,
    Tool,
    Other,
}

impl ItemCategory {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Garment => "garment",
            Self::Footwear => "footwear",
            Self::Accessory => "accessory",
            Self::Armor => "armor",
            Self::Jewelry => "jewelry",
            Self::Container => "container",
            Self::Book => "book",
            Self::Document => "document",
            Self::Artwork => "artwork",
            Self::Decoration => "decoration",
            Self::Tool => "tool",
            Self::Other => "other",
        }
    }
}

/// Shared catalog entry. Immutable identity plus a versioned editable body.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ItemDefinition {
    pub schema: String,
    pub definition_id: String,
    pub creator_core_uuid: Option<String>,
    pub name: String,
    pub description: String,
    pub category: ItemCategory,
    pub visibility: Visibility,
    pub tags: Vec<String>,
    pub sizes: Vec<String>,
    pub materials: Vec<String>,
    pub properties: serde_json::Value,
    pub asset_id: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    pub revision: u64,
}

/// One owned instance of a catalog definition (or a Core-authored unique item).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ItemInstance {
    pub schema: String,
    pub instance_id: String,
    pub definition_id: Option<String>,
    pub owner_core_uuid: String,
    pub holder_core_uuid: Option<String>,
    pub condition_percent: u8,
    pub location: Option<String>,
    pub state: String,
    pub visibility: Visibility,
    pub provenance: Vec<ProvenanceEntry>,
    pub created_at: String,
    pub revision: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProvenanceEntry {
    pub at: String,
    pub actor: String,
    pub action: String,
    pub note: String,
}

/// A saved outfit: an ordered set of owned/held item instances.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Outfit {
    pub schema: String,
    pub outfit_id: String,
    pub core_uuid: String,
    pub name: String,
    pub item_instance_ids: Vec<String>,
    pub visibility: Visibility,
    pub created_at: String,
    pub updated_at: String,
    pub revision: u64,
}

/// Written possession with real persisted content (not a memory claim).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Document {
    pub schema: String,
    pub document_id: String,
    pub owner_core_uuid: String,
    pub title: String,
    pub body: String,
    pub format: DocumentFormat,
    pub visibility: Visibility,
    pub created_at: String,
    pub updated_at: String,
    pub revision: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DocumentFormat {
    PlainText,
    Markdown,
    Note,
    Letter,
}

/// A persistent world/room object with holder and state.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorldObject {
    pub schema: String,
    pub object_id: String,
    pub definition_id: Option<String>,
    pub owner_core_uuid: Option<String>,
    pub holder_core_uuid: Option<String>,
    pub location: String,
    pub state: String,
    pub affordances: Vec<String>,
    pub visibility: Visibility,
    pub created_at: String,
    pub updated_at: String,
    pub revision: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActionCategory {
    Idle,
    Social,
    Emotional,
    Romantic,
    Adult,
    CoreSpecific,
    Accessibility,
    Object,
    Activity,
}

/// Declarative action: execution lives elsewhere; consent is explicit.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ActionDefinition {
    pub schema: String,
    pub action_id: String,
    pub name: String,
    pub description: String,
    pub category: ActionCategory,
    pub supported_rigs: Vec<String>,
    pub capabilities: Vec<String>,
    pub parameters: serde_json::Value,
    pub requires_consent: bool,
    pub interruptible: bool,
    pub cancellable: bool,
    pub version: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConsentScope {
    Conversation,
    Affection,
    Romantic,
    Adult,
    Object,
    SharedProject,
}

/// Affirmative, scoped, revocable consent. Never inferred from trust.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConsentGrant {
    pub schema: String,
    pub grant_id: String,
    pub core_uuid: String,
    pub scope: ConsentScope,
    pub capability: String,
    pub participants: Vec<String>,
    pub granted_at: String,
    pub expires_at: Option<String>,
    pub revoked_at: Option<String>,
    pub context: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GoalStatus {
    Proposed,
    Active,
    Paused,
    Completed,
    Abandoned,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Goal {
    pub schema: String,
    pub goal_id: String,
    pub core_uuid: String,
    pub title: String,
    pub description: String,
    pub status: GoalStatus,
    pub priority: u8,
    pub due_at: Option<String>,
    pub trigger: Option<String>,
    pub progress_percent: u8,
    pub provenance: String,
    pub created_at: String,
    pub updated_at: String,
    pub revision: u64,
}

fn valid_id(value: &str) -> bool {
    !value.is_empty()
        && value.chars().count() <= MAX_ID_CHARS
        && value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | ':'))
}

fn bounded(value: &str, max: usize) -> bool {
    !value.chars().any(char::is_control) && value.chars().count() <= max
}

fn bounded_opt(value: &Option<String>, max: usize) -> bool {
    value.as_deref().is_none_or(|v| bounded(v, max))
}

fn valid_visibility(_value: Visibility) -> bool {
    true
}

fn valid_tags(tags: &[String]) -> bool {
    tags.len() <= MAX_TAGS
        && tags
            .iter()
            .all(|t| bounded(t, MAX_TAG_CHARS) && !t.is_empty())
}

fn valid_list(values: &[String], max_len: usize) -> bool {
    values.len() <= MAX_LIST_ITEMS
        && values
            .iter()
            .all(|v| valid_id(v) && v.chars().count() <= max_len)
}

impl ItemDefinition {
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.schema != LIVING_CONTRACT_VERSION
            || !valid_id(&self.definition_id)
            || self.name.trim().is_empty()
            || !bounded(&self.name, MAX_NAME_CHARS)
            || !bounded(&self.description, MAX_TEXT_CHARS)
            || !valid_tags(&self.tags)
            || !valid_list(&self.sizes, MAX_TAG_CHARS)
            || !valid_list(&self.materials, MAX_TAG_CHARS)
            || !bounded_opt(&self.asset_id, MAX_ID_CHARS)
            || !bounded(&self.created_at, 64)
            || !bounded(&self.updated_at, 64)
            || self.revision == 0
            || !valid_visibility(self.visibility)
        {
            return Err("invalid item definition");
        }
        Ok(())
    }
}

impl ItemInstance {
    pub fn new(core_uuid: &str, created_at: &str) -> Self {
        Self {
            schema: LIVING_CONTRACT_VERSION.to_owned(),
            instance_id: String::new(),
            definition_id: None,
            owner_core_uuid: core_uuid.to_owned(),
            holder_core_uuid: None,
            condition_percent: 100,
            location: None,
            state: "stored".to_owned(),
            visibility: Visibility::Private,
            provenance: Vec::new(),
            created_at: created_at.to_owned(),
            revision: 1,
        }
    }

    pub fn validate(&self) -> Result<(), &'static str> {
        if self.schema != LIVING_CONTRACT_VERSION
            || !valid_id(&self.instance_id)
            || !bounded_opt(&self.definition_id, MAX_ID_CHARS)
            || !valid_id(&self.owner_core_uuid)
            || !bounded_opt(&self.holder_core_uuid, MAX_ID_CHARS)
            || !bounded(&self.state, MAX_TAG_CHARS)
            || !bounded_opt(&self.location, MAX_NAME_CHARS)
            || !bounded(&self.created_at, 64)
            || self.revision == 0
            || self.condition_percent > 100
            || !valid_visibility(self.visibility)
            || self.provenance.len() > MAX_LIST_ITEMS
            || self.provenance.iter().any(|p| {
                !bounded(&p.actor, MAX_ID_CHARS)
                    || !bounded(&p.action, MAX_TAG_CHARS)
                    || !bounded(&p.note, MAX_TEXT_CHARS)
                    || !bounded(&p.at, 64)
            })
        {
            return Err("invalid item instance");
        }
        Ok(())
    }

    pub fn is_owned_by(&self, core_uuid: &str) -> bool {
        self.owner_core_uuid == core_uuid
    }
}

impl Outfit {
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.schema != LIVING_CONTRACT_VERSION
            || !valid_id(&self.outfit_id)
            || !valid_id(&self.core_uuid)
            || !bounded(&self.name, MAX_NAME_CHARS)
            || !valid_list(&self.item_instance_ids, MAX_ID_CHARS)
            || !bounded(&self.created_at, 64)
            || !bounded(&self.updated_at, 64)
            || self.revision == 0
            || !valid_visibility(self.visibility)
        {
            return Err("invalid outfit");
        }
        Ok(())
    }
}

impl Document {
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.schema != LIVING_CONTRACT_VERSION
            || !valid_id(&self.document_id)
            || !valid_id(&self.owner_core_uuid)
            || !bounded(&self.title, MAX_NAME_CHARS)
            || !bounded(&self.body, MAX_DOCUMENT_BODY_CHARS)
            || !bounded(&self.created_at, 64)
            || !bounded(&self.updated_at, 64)
            || self.revision == 0
            || !valid_visibility(self.visibility)
        {
            return Err("invalid document");
        }
        Ok(())
    }
}

impl WorldObject {
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.schema != LIVING_CONTRACT_VERSION
            || !valid_id(&self.object_id)
            || !bounded_opt(&self.definition_id, MAX_ID_CHARS)
            || !bounded_opt(&self.owner_core_uuid, MAX_ID_CHARS)
            || !bounded_opt(&self.holder_core_uuid, MAX_ID_CHARS)
            || !bounded(&self.location, MAX_NAME_CHARS)
            || !bounded(&self.state, MAX_TAG_CHARS)
            || self.affordances.len() > MAX_LIST_ITEMS
            || self.affordances.iter().any(|a| !bounded(a, MAX_TAG_CHARS))
            || !bounded(&self.created_at, 64)
            || !bounded(&self.updated_at, 64)
            || self.revision == 0
            || !valid_visibility(self.visibility)
        {
            return Err("invalid world object");
        }
        Ok(())
    }
}

impl ActionDefinition {
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.schema != LIVING_CONTRACT_VERSION
            || !valid_id(&self.action_id)
            || !bounded(&self.name, MAX_NAME_CHARS)
            || !bounded(&self.description, MAX_TEXT_CHARS)
            || self.supported_rigs.len() > MAX_LIST_ITEMS
            || self.capabilities.len() > MAX_LIST_ITEMS
            || self.version == 0
        {
            return Err("invalid action definition");
        }
        Ok(())
    }
}

impl ConsentGrant {
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.schema != LIVING_CONTRACT_VERSION
            || !valid_id(&self.grant_id)
            || !valid_id(&self.core_uuid)
            || !bounded(&self.capability, MAX_TAG_CHARS)
            || self.participants.len() > MAX_LIST_ITEMS
            || self.participants.iter().any(|p| !bounded(p, MAX_ID_CHARS))
            || !bounded(&self.granted_at, 64)
            || !bounded_opt(&self.expires_at, 64)
            || !bounded_opt(&self.revoked_at, 64)
            || !bounded(&self.context, MAX_TEXT_CHARS)
        {
            return Err("invalid consent grant");
        }
        Ok(())
    }

    pub fn is_active(&self, now: &str) -> bool {
        self.revoked_at.is_none()
            && now >= self.granted_at.as_str()
            && self.expires_at.as_deref().is_none_or(|e| e > now)
    }
}

impl Goal {
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.schema != LIVING_CONTRACT_VERSION
            || !valid_id(&self.goal_id)
            || !valid_id(&self.core_uuid)
            || !bounded(&self.title, MAX_NAME_CHARS)
            || !bounded(&self.description, MAX_TEXT_CHARS)
            || !bounded_opt(&self.due_at, 64)
            || !bounded_opt(&self.trigger, MAX_TAG_CHARS)
            || !bounded(&self.provenance, MAX_TEXT_CHARS)
            || !bounded(&self.created_at, 64)
            || !bounded(&self.updated_at, 64)
            || self.progress_percent > 100
            || self.revision == 0
        {
            return Err("invalid goal");
        }
        Ok(())
    }
}

impl ItemCategory {
    pub fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "garment" => Self::Garment,
            "footwear" => Self::Footwear,
            "accessory" => Self::Accessory,
            "armor" => Self::Armor,
            "jewelry" => Self::Jewelry,
            "container" => Self::Container,
            "book" => Self::Book,
            "document" => Self::Document,
            "artwork" => Self::Artwork,
            "decoration" => Self::Decoration,
            "tool" => Self::Tool,
            "other" => Self::Other,
            _ => return None,
        })
    }
}

impl Visibility {
    pub fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "private" => Self::Private,
            "shared" => Self::Shared,
            "owner_only" => Self::OwnerOnly,
            "public_catalog" => Self::PublicCatalog,
            _ => return None,
        })
    }
}

impl DocumentFormat {
    pub fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "plain_text" => Self::PlainText,
            "markdown" => Self::Markdown,
            "note" => Self::Note,
            "letter" => Self::Letter,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TransferType {
    Gift,
    Loan,
    Trade,
    Swap,
    Return,
}

impl TransferType {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Gift => "gift",
            Self::Loan => "loan",
            Self::Trade => "trade",
            Self::Swap => "swap",
            Self::Return => "return",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "gift" => Self::Gift,
            "loan" => Self::Loan,
            "trade" => Self::Trade,
            "swap" => Self::Swap,
            "return" => Self::Return,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TransferStatus {
    Pending,
    Accepted,
    Rejected,
    Cancelled,
    Failed,
}

impl TransferStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Accepted => "accepted",
            Self::Rejected => "rejected",
            Self::Cancelled => "cancelled",
            Self::Failed => "failed",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "pending" => Self::Pending,
            "accepted" => Self::Accepted,
            "rejected" => Self::Rejected,
            "cancelled" => Self::Cancelled,
            "failed" => Self::Failed,
            _ => return None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn definition() -> ItemDefinition {
        ItemDefinition {
            schema: LIVING_CONTRACT_VERSION.to_owned(),
            definition_id: "def-1".to_owned(),
            creator_core_uuid: None,
            name: "Test dress".to_owned(),
            description: "A test garment".to_owned(),
            category: ItemCategory::Garment,
            visibility: Visibility::Private,
            tags: vec!["summer".to_owned()],
            sizes: vec!["m".to_owned()],
            materials: vec!["linen".to_owned()],
            properties: serde_json::json!({}),
            asset_id: None,
            created_at: "2026-01-01T00:00:00Z".to_owned(),
            updated_at: "2026-01-01T00:00:00Z".to_owned(),
            revision: 1,
        }
    }

    #[test]
    fn definitions_validate() {
        assert!(definition().validate().is_ok());
        let mut bad = definition();
        bad.name.clear();
        assert!(bad.validate().is_err());
    }

    #[test]
    fn instances_bind_owner_and_validate() {
        let mut instance = ItemInstance::new(
            "a9a0a604-a4e7-4a88-a985-3f071e2c8e7d",
            "2026-01-01T00:00:00Z",
        );
        instance.instance_id = "instance-1".to_owned();
        assert!(instance.validate().is_ok());
        assert!(instance.is_owned_by("a9a0a604-a4e7-4a88-a985-3f071e2c8e7d"));
        assert!(!instance.is_owned_by("other"));
    }

    #[test]
    fn consent_is_revocable() {
        let grant = ConsentGrant {
            schema: LIVING_CONTRACT_VERSION.to_owned(),
            grant_id: "grant-1".to_owned(),
            core_uuid: "a9a0a604-a4e7-4a88-a985-3f071e2c8e7d".to_owned(),
            scope: ConsentScope::Romantic,
            capability: "hold_hands".to_owned(),
            participants: vec!["owner".to_owned()],
            granted_at: "2026-01-01T00:00:00Z".to_owned(),
            expires_at: None,
            revoked_at: None,
            context: "test".to_owned(),
        };
        assert!(grant.is_active("2026-01-02T00:00:00Z"));
        assert!(!grant.is_active("2025-12-31T00:00:00Z"));
    }
}
