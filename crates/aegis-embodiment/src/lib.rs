//! Per-Core embodiment interfaces: asset library, appearance, expressions,
//! gestures, wardrobe, and voice profile configuration.
//!
//! Assets and appearance are stored only in the owning Core's directory and are
//! bound to that Core's immutable UUID and database instance UUID. A copied
//! database is rejected. Asset content is verified by SHA-256 on read; this is
//! content integrity, not publisher authenticity.
//!
//! No renderer or audio provider is required for these interfaces. They expose
//! the stable contracts, configuration, and asset pipeline that a renderer,
//! voice provider, or GUI consumes.

pub mod appearance;
pub mod asset;
pub mod error;
pub mod store;

pub use appearance::{
    is_hex_color, validate_animation_name, validate_theme, Appearance, Theme, VoiceProfile,
    AEGIS_ACCENT, AEGIS_PRIMARY, AEGIS_TERTIARY, APPEARANCE_SCHEMA, MAX_EXPRESSIONS, MAX_GESTURES,
    MAX_WARDROBE,
};
pub use asset::{
    safe_file_name, Asset, AssetKind, NewAsset, ASSET_SCHEMA, MAX_ASSET_BYTES,
    MAX_ASSET_METADATA_BYTES, MAX_ASSET_NAME_CHARS, MAX_ASSET_TAGS, MAX_MEDIA_TYPE_CHARS,
};
pub use error::EmbodimentError;
pub use store::{
    AssetExport, EmbodimentStore, ASSET_EVENT_SCHEMA, ASSET_EXPORT_SCHEMA,
    EMBODIMENT_APPLICATION_ID, EMBODIMENT_COMPONENT, EMBODIMENT_DB_FILE,
};
