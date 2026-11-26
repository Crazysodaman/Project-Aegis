//! Social runtime: Platform-owned private groups and per-Core relationships.
//!
//! Group membership and message access are enforced by the Platform. The owner
//! retains administrative access; a Core must be an explicit member. Private
//! relationship records stay inside each Core's own `social.db` and are never
//! exposed to other Cores by default.

pub mod awareness;
pub mod error;
pub mod groups;
pub mod relationships;

pub use awareness::{
    social_awareness_summary, social_awareness_summary_limited, AwarenessMembership,
    AwarenessRelationship, SocialAwarenessSummary, AWARENESS_MEMBERSHIP_SCHEMA,
    AWARENESS_RELATIONSHIP_SCHEMA, MAX_AWARENESS_MEMBERSHIPS, MAX_AWARENESS_RELATIONSHIPS,
    MAX_AWARENESS_TAGS, SOCIAL_AWARENESS_SCHEMA,
};
pub use error::SocialError;
pub use groups::{
    group_store, CoreGroupContext, GroupActivitySummary, GroupEvent, GroupMember, GroupMessage,
    GroupParticipation, GroupStore, MemberKind, ParticipationSummary, SocialActor, SocialGroup,
    GROUP_ACTIVITY_SCHEMA, GROUP_CONTEXT_SCHEMA, GROUP_EVENT_SCHEMA, GROUP_MEMBER_SCHEMA,
    GROUP_MESSAGE_SCHEMA, GROUP_SCHEMA, MAX_GROUP_MESSAGES, MAX_GROUP_MESSAGE_CHARS,
    MAX_GROUP_NAME_CHARS, PARTICIPATION_ENTRY_SCHEMA, PARTICIPATION_SUMMARY_SCHEMA,
};
pub use relationships::{
    Relationship, RelationshipEvent, RelationshipStore, RelationshipTargetKind,
    MAX_RELATIONSHIP_EVENT_DETAIL_BYTES, MAX_RELATIONSHIP_EVENT_TYPE_CHARS,
    RELATIONSHIP_APPLICATION_ID, RELATIONSHIP_COMPONENT, RELATIONSHIP_DB_FILE,
    RELATIONSHIP_EVENT_SCHEMA, RELATIONSHIP_SCHEMA,
};
