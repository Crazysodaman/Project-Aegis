//! Bounded per-Core social-awareness summary.
//!
//! The summary is a read-only projection over two things a Core actually owns
//! or is actually entitled to see:
//!
//! - current group memberships, enforced by [`GroupStore`] using live membership
//!   rows; and
//! - the Core's own explicit private relationship records.
//!
//! It never opens another Core's `social.db`, never joins relationship data into
//! group projections, and never infers trust, familiarity, awareness, or an
//! interaction that was not explicitly recorded. A group the Core is not a
//! current member of does not appear, even if the Core used to belong.

use aegis_storage::now_rfc3339;
use serde::{Deserialize, Serialize};

use crate::error::SocialError;
use crate::groups::{GroupStore, ParticipationSummary};
use crate::relationships::{Relationship, RelationshipStore, RelationshipTargetKind};

/// Schema identifier for a per-Core social-awareness summary.
pub const SOCIAL_AWARENESS_SCHEMA: &str = "aegis.social.awareness_summary.v1";
/// Schema identifier for one membership entry in a social-awareness summary.
pub const AWARENESS_MEMBERSHIP_SCHEMA: &str = "aegis.social.awareness_membership.v1";
/// Schema identifier for one relationship entry in a social-awareness summary.
pub const AWARENESS_RELATIONSHIP_SCHEMA: &str = "aegis.social.awareness_relationship.v1";
/// Maximum memberships returned by one summary.
pub const MAX_AWARENESS_MEMBERSHIPS: usize = 64;
/// Maximum relationships returned by one summary.
pub const MAX_AWARENESS_RELATIONSHIPS: usize = 64;
/// Maximum tags copied from a relationship into the summary.
pub const MAX_AWARENESS_TAGS: usize = 8;

/// One current group membership in a social-awareness summary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AwarenessMembership {
    pub schema: String,
    pub group_id: String,
    pub group_name: String,
    pub joined_at: String,
    /// Messages this Core authored in this group only.
    pub message_count: u64,
    /// Last activity attributable to this Core in this group.
    pub last_activity_at: String,
}

/// One explicit relationship owned by the requesting Core.
///
/// Only the requesting Core's own relationship records can appear. Free-form
/// `notes` are intentionally omitted so the summary stays a compact projection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AwarenessRelationship {
    pub schema: String,
    pub relationship_id: String,
    pub target_kind: RelationshipTargetKind,
    pub target_id: String,
    pub label: String,
    pub trust: u8,
    pub tags: Vec<String>,
    pub revision: u64,
    pub updated_at: String,
}

/// Bounded, explicitly-sourced social-awareness projection for one Core.
///
/// `membership_count` and `relationship_count` are the totals that exist before
/// truncation. `*_truncated` says whether the returned vectors were cut down to
/// the configured limit.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SocialAwarenessSummary {
    pub schema: String,
    pub core_uuid: String,
    pub generated_at: String,
    pub membership_limit: usize,
    pub relationship_limit: usize,
    pub membership_count: u64,
    pub relationship_count: u64,
    pub memberships_truncated: bool,
    pub relationships_truncated: bool,
    pub memberships: Vec<AwarenessMembership>,
    pub relationships: Vec<AwarenessRelationship>,
}

impl SocialAwarenessSummary {
    /// Build a summary using the default bounds.
    pub fn build(
        groups: &GroupStore,
        relationships: &RelationshipStore,
    ) -> Result<Self, SocialError> {
        social_awareness_summary(groups, relationships)
    }

    /// Build a summary with caller-supplied bounds.
    pub fn build_limited(
        groups: &GroupStore,
        relationships: &RelationshipStore,
        max_memberships: usize,
        max_relationships: usize,
    ) -> Result<Self, SocialError> {
        social_awareness_summary_limited(groups, relationships, max_memberships, max_relationships)
    }
}

/// Build a bounded social-awareness summary using the default limits.
pub fn social_awareness_summary(
    groups: &GroupStore,
    relationships: &RelationshipStore,
) -> Result<SocialAwarenessSummary, SocialError> {
    social_awareness_summary_limited(
        groups,
        relationships,
        MAX_AWARENESS_MEMBERSHIPS,
        MAX_AWARENESS_RELATIONSHIPS,
    )
}

/// Build a bounded social-awareness summary with explicit limits.
///
/// The core identity is taken from the relationship store, which is bound to
/// one Core and one database instance. Memberships come only from the group
/// store's live participation query, and relationships come only from the
/// Core-owned store. Limits are clamped to the documented maxima, and a lower
/// bound of one avoids returning a misleading empty projection on `0`.
pub fn social_awareness_summary_limited(
    groups: &GroupStore,
    relationships: &RelationshipStore,
    max_memberships: usize,
    max_relationships: usize,
) -> Result<SocialAwarenessSummary, SocialError> {
    let membership_limit = max_memberships.clamp(1, MAX_AWARENESS_MEMBERSHIPS);
    let relationship_limit = max_relationships.clamp(1, MAX_AWARENESS_RELATIONSHIPS);
    let core_uuid = relationships.core_uuid().to_owned();

    let participation: ParticipationSummary = groups.participation_summary(&core_uuid)?;
    let membership_count = participation.memberships.len() as u64;
    let memberships_truncated = participation.memberships.len() > membership_limit;
    let memberships = participation
        .memberships
        .into_iter()
        .take(membership_limit)
        .map(|entry| AwarenessMembership {
            schema: AWARENESS_MEMBERSHIP_SCHEMA.to_owned(),
            group_id: entry.group_id,
            group_name: entry.group_name,
            joined_at: entry.joined_at,
            message_count: entry.message_count,
            last_activity_at: entry.last_activity_at,
        })
        .collect();

    let owned: Vec<Relationship> = relationships.list()?;
    let relationship_count = owned.len() as u64;
    let relationships_truncated = owned.len() > relationship_limit;
    let relationships = owned
        .into_iter()
        .take(relationship_limit)
        .map(read_awareness_relationship)
        .collect();

    Ok(SocialAwarenessSummary {
        schema: SOCIAL_AWARENESS_SCHEMA.to_owned(),
        core_uuid,
        generated_at: now_rfc3339()?,
        membership_limit,
        relationship_limit,
        membership_count,
        relationship_count,
        memberships_truncated,
        relationships_truncated,
        memberships,
        relationships,
    })
}

fn read_awareness_relationship(relationship: Relationship) -> AwarenessRelationship {
    let mut tags = relationship.tags;
    if tags.len() > MAX_AWARENESS_TAGS {
        tags.truncate(MAX_AWARENESS_TAGS);
    }
    AwarenessRelationship {
        schema: AWARENESS_RELATIONSHIP_SCHEMA.to_owned(),
        relationship_id: relationship.relationship_id,
        target_kind: relationship.target_kind,
        target_id: relationship.target_id,
        label: relationship.label,
        trust: relationship.trust,
        tags,
        revision: relationship.revision,
        updated_at: relationship.updated_at,
    }
}
