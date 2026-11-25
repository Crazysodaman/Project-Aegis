//! Social runtime: Platform-owned private groups and per-Core relationships.
//!
//! Group membership and message access are enforced by the Platform. The owner
//! retains administrative access; a Core must be an explicit member. Private
//! relationship records stay inside each Core's own `social.db` and are never
//! exposed to other Cores by default.

pub mod error;
pub mod groups;
pub mod relationships;

pub use error::SocialError;
pub use groups::{
    group_store, GroupEvent, GroupMember, GroupMessage, GroupStore, MemberKind, SocialActor,
    SocialGroup, GROUP_EVENT_SCHEMA, GROUP_MEMBER_SCHEMA, GROUP_MESSAGE_SCHEMA, GROUP_SCHEMA,
    MAX_GROUP_MESSAGES, MAX_GROUP_MESSAGE_CHARS, MAX_GROUP_NAME_CHARS,
};
pub use relationships::{
    Relationship, RelationshipEvent, RelationshipStore, RelationshipTargetKind,
    RELATIONSHIP_APPLICATION_ID, RELATIONSHIP_COMPONENT, RELATIONSHIP_DB_FILE,
    RELATIONSHIP_EVENT_SCHEMA, RELATIONSHIP_SCHEMA,
};
