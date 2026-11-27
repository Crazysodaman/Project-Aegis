//! Core-neutral Aegis Platform foundation for Batch 1.

mod catalog;
mod service;
mod store;
mod transfer;
mod weather;
mod world;

pub use catalog::{
    CatalogActor, CatalogError, CatalogFilter, CatalogStore, CATALOG_DEFAULT_LIMIT,
    CATALOG_MAX_LIMIT, CATALOG_MAX_SEARCH_CHARS,
};
pub use service::{InitializationResult, PlatformService, ServiceError};
pub use store::{PlatformIdentity, PlatformStore};
pub use transfer::{
    TransferError, TransferEvent, TransferRecord, TransferStore, TRANSFER_DEFAULT_LIMIT,
    TRANSFER_MAX_EVENTS, TRANSFER_MAX_LIMIT, TRANSFER_MAX_NOTE_CHARS,
};
pub use weather::{
    parse_open_meteo, wmo_condition, OpenMeteoProvider, OPEN_METEO_CACHE_TTL_SECONDS,
    OPEN_METEO_ENDPOINT, OPEN_METEO_PROVIDER_NAME,
};
pub use world::{
    UnavailableWeatherProvider, WeatherData, WeatherProvider, WeatherRequest, WeatherUnavailable,
    WorldClock, WorldClockReading, WorldConfig, WorldError, WorldService, WorldStore,
    CONFIG_LATITUDE, CONFIG_LONGITUDE, CONFIG_STALE_AFTER_SECONDS, CONFIG_TIMEZONE,
    DEFAULT_STALE_AFTER_SECONDS, DEFAULT_TIMEZONE,
};

use thiserror::Error;

#[derive(Debug, Error)]
pub enum PlatformError {
    #[error("Platform storage failed: {0}")]
    Storage(#[from] aegis_storage::StorageError),
    #[error("Platform SQLite operation failed: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("Platform identity is missing")]
    IdentityMissing,
    #[error("invalid Platform state: {0}")]
    InvalidState(&'static str),
}
