//! Recovered historical module catalog (data only).
//!
//! This module records the 55-entry module catalog that was recovered from the
//! read-only legacy reference at `D:\SofiaDev\SofiaAdaLyra` (see
//! `docs/sofia2/MODULE_OWNERSHIP_MATRIX.md`,
//! `docs/sofia2/PLATFORM_AND_MODULE_CATALOG_V1.md`, and
//! `docs/roadmaps/AG_D026_MODULE_CATALOG_RATIONALIZATION_AUDIT.md`).
//!
//! The catalog is **not** an in-repo authority and it is **not** implementation
//! evidence. Every entry is [`ModuleCatalogStatus::DesignOnly`]; no entry claims
//! that a module exists, is installed, is certified, or may execute anything.
//! Membership is design intent only.
//!
//! Totals: 5 companion + 47 optional + 1 submodule + 2 adapters = 55 entries
//! (52 separately scoped designs plus 3 attached to existing parents).

use std::collections::BTreeSet;

use thiserror::Error;

/// Expected total number of recovered catalog entries.
pub const TOTAL_ENTRIES: usize = 55;
/// Expected number of companion (`RM-*`) entries.
pub const COMPANION_COUNT: usize = 5;
/// Expected number of optional (`MOD-*`) entries.
pub const OPTIONAL_COUNT: usize = 47;
/// Expected number of attached submodule entries.
pub const SUBMODULE_COUNT: usize = 1;
/// Expected number of attached adapter entries.
pub const ADAPTER_COUNT: usize = 2;

/// Parent identifiers owned by the Platform/Core rather than this catalog.
///
/// These are the only non-catalog parent references accepted by
/// [`validate`]. `PLAT-02` is the Platform canonical-state/ownership contract;
/// `core-memory` is the Core canonical memory admission owner.
pub const EXTERNAL_PARENTS: &[&str] = &["PLAT-02", "core-memory"];

/// Packaging category of a recovered catalog entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ModuleCatalogCategory {
    /// Companion release module.
    Companion,
    /// Independently scoped optional module design.
    Optional,
    /// Attached submodule inheriting a parent's lifecycle.
    Submodule,
    /// Attached provider/migration adapter inheriting a parent's lifecycle.
    Adapter,
}

/// Readiness status of a recovered catalog entry.
///
/// Only design records exist here; no entry is an implementation claim.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ModuleCatalogStatus {
    /// Requirements are recorded; nothing is implemented or authorized.
    DesignOnly,
}

/// One static catalog entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ModuleCatalogEntry {
    /// Stable catalog identifier (for example `MOD-DEV`).
    pub id: &'static str,
    /// Historical design name.
    pub name: &'static str,
    /// Packaging category.
    pub category: ModuleCatalogCategory,
    /// Parent catalog IDs or [`EXTERNAL_PARENTS`] anchors.
    pub parents: &'static [&'static str],
    /// Readiness status; always [`ModuleCatalogStatus::DesignOnly`].
    pub status: ModuleCatalogStatus,
    /// Short recovered purpose statement.
    pub purpose: &'static str,
}

/// Count of entries per category plus the total.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CatalogCounts {
    /// Companion entries.
    pub companion: usize,
    /// Optional entries.
    pub optional: usize,
    /// Submodule entries.
    pub submodule: usize,
    /// Adapter entries.
    pub adapter: usize,
    /// Total entries across all categories.
    pub total: usize,
}

/// Catalog integrity error.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum CatalogError {
    /// The total entry count is not [`TOTAL_ENTRIES`].
    #[error("catalog contains {actual} entries, expected {expected}")]
    WrongTotal {
        /// Expected total.
        expected: usize,
        /// Actual total.
        actual: usize,
    },
    /// Two entries share an identifier.
    #[error("duplicate module catalog id: {0}")]
    DuplicateId(&'static str),
    /// A category count differs from its constant.
    #[error("{category:?} count is {actual}, expected {expected}")]
    CategoryCountMismatch {
        /// Category being checked.
        category: ModuleCatalogCategory,
        /// Expected count.
        expected: usize,
        /// Actual count.
        actual: usize,
    },
    /// A parent identifier is neither in the catalog nor an external anchor.
    #[error("module {id} references unknown parent {parent}")]
    UnknownParent {
        /// Entry declaring the parent.
        id: &'static str,
        /// Unresolved parent identifier.
        parent: &'static str,
    },
    /// An attached entry declares no parent.
    #[error("attached module {0} must declare at least one parent")]
    MissingParent(&'static str),
    /// A standalone entry declares a parent.
    #[error("standalone module {0} must not declare parents")]
    UnexpectedParent(&'static str),
    /// A required textual field is empty.
    #[error("module {0} has an empty {1}")]
    EmptyField(&'static str, &'static str),
}

static ENTRIES: [ModuleCatalogEntry; TOTAL_ENTRIES] = [
    ModuleCatalogEntry {
        id: "RM-DESKTOP",
        name: "Desktop",
        category: ModuleCatalogCategory::Companion,
        parents: &[],
        status: ModuleCatalogStatus::DesignOnly,
        purpose: "Canonical chat, tray, settings, approvals inbox, module health and controls.",
    },
    ModuleCatalogEntry {
        id: "RM-DISCORD",
        name: "Discord",
        category: ModuleCatalogCategory::Companion,
        parents: &[],
        status: ModuleCatalogStatus::DesignOnly,
        purpose: "Discord DM conversation and optional outreach with transport delivery receipts.",
    },
    ModuleCatalogEntry {
        id: "RM-AVATAR2D",
        name: "Avatar 2D",
        category: ModuleCatalogCategory::Companion,
        parents: &[],
        status: ModuleCatalogStatus::DesignOnly,
        purpose: "2D avatar rendering (face, ears, tail, eyes, pose, wardrobe) with truthful frame receipts.",
    },
    ModuleCatalogEntry {
        id: "RM-VOICE",
        name: "Voice",
        category: ModuleCatalogCategory::Companion,
        parents: &[],
        status: ModuleCatalogStatus::DesignOnly,
        purpose: "TTS/STT, audio device handling, interruptions, lip sync and consent.",
    },
    ModuleCatalogEntry {
        id: "RM-MOBILE",
        name: "Mobile",
        category: ModuleCatalogCategory::Companion,
        parents: &[],
        status: ModuleCatalogStatus::DesignOnly,
        purpose: "Mobile device pairing, scoped chat, notifications and optional consented sensors.",
    },
    ModuleCatalogEntry {
        id: "MOD-DEV",
        name: "Development",
        category: ModuleCatalogCategory::Optional,
        parents: &[],
        status: ModuleCatalogStatus::DesignOnly,
        purpose: "Source inspection, patch/test proposals and approved execution.",
    },
    ModuleCatalogEntry {
        id: "MOD-RESEARCH",
        name: "Research",
        category: ModuleCatalogCategory::Optional,
        parents: &[],
        status: ModuleCatalogStatus::DesignOnly,
        purpose: "Web and document evidence gathering plus source analysis.",
    },
    ModuleCatalogEntry {
        id: "MOD-CREATIVE",
        name: "Creative Studio",
        category: ModuleCatalogCategory::Optional,
        parents: &[],
        status: ModuleCatalogStatus::DesignOnly,
        purpose: "Images, audio, writing, animation, media workflow and artifacts.",
    },
    ModuleCatalogEntry {
        id: "MOD-VISION",
        name: "Vision",
        category: ModuleCatalogCategory::Optional,
        parents: &[],
        status: ModuleCatalogStatus::DesignOnly,
        purpose: "Screen and window observation under explicit scope.",
    },
    ModuleCatalogEntry {
        id: "MOD-DESKTOP-ACT",
        name: "Desktop Automation",
        category: ModuleCatalogCategory::Optional,
        parents: &[],
        status: ModuleCatalogStatus::DesignOnly,
        purpose: "Approved local mouse, keyboard and application automation.",
    },
    ModuleCatalogEntry {
        id: "MOD-HOMELAB",
        name: "Homelab",
        category: ModuleCatalogCategory::Optional,
        parents: &[],
        status: ModuleCatalogStatus::DesignOnly,
        purpose: "Site-specific trusted host, service and VM integration.",
    },
    ModuleCatalogEntry {
        id: "MOD-ROBOTICS-GAIA",
        name: "Gaia Robotics",
        category: ModuleCatalogCategory::Optional,
        parents: &[],
        status: ModuleCatalogStatus::DesignOnly,
        purpose: "Simulator, kinematics and a calibrated supervised actuator adapter.",
    },
    ModuleCatalogEntry {
        id: "MOD-TRAINS",
        name: "Model Trains",
        category: ModuleCatalogCategory::Optional,
        parents: &[],
        status: ModuleCatalogStatus::DesignOnly,
        purpose: "JMRI roster and sensor observation with later guarded physical dispatch.",
    },
    ModuleCatalogEntry {
        id: "MOD-GAMES",
        name: "Gaming",
        category: ModuleCatalogCategory::Optional,
        parents: &[],
        status: ModuleCatalogStatus::DesignOnly,
        purpose: "Game assistance, approved APIs and GPU/gaming awareness.",
    },
    ModuleCatalogEntry {
        id: "MOD-PRODUCTIVITY",
        name: "Productivity",
        category: ModuleCatalogCategory::Optional,
        parents: &[],
        status: ModuleCatalogStatus::DesignOnly,
        purpose: "Tasks, inventory, calendar, scheduling and documents integration.",
    },
    ModuleCatalogEntry {
        id: "MOD-ML",
        name: "Machine Learning",
        category: ModuleCatalogCategory::Optional,
        parents: &[],
        status: ModuleCatalogStatus::DesignOnly,
        purpose: "Governed datasets, experiments, benchmarks and model candidates.",
    },
    ModuleCatalogEntry {
        id: "MOD-GENERALIZATION",
        name: "Generalization",
        category: ModuleCatalogCategory::Optional,
        parents: &[],
        status: ModuleCatalogStatus::DesignOnly,
        purpose: "Transferable skills and procedures with independent evaluation.",
    },
    ModuleCatalogEntry {
        id: "MOD-VWORLD",
        name: "Virtual World",
        category: ModuleCatalogCategory::Optional,
        parents: &[],
        status: ModuleCatalogStatus::DesignOnly,
        purpose: "Reproducible virtual scenes, simulation and interactions.",
    },
    ModuleCatalogEntry {
        id: "MOD-MENTOR",
        name: "Mentor",
        category: ModuleCatalogCategory::Optional,
        parents: &[],
        status: ModuleCatalogStatus::DesignOnly,
        purpose: "Independent rubrics, challenges, reviews and regression evidence.",
    },
    ModuleCatalogEntry {
        id: "MOD-BROWSER",
        name: "Browser",
        category: ModuleCatalogCategory::Optional,
        parents: &[],
        status: ModuleCatalogStatus::DesignOnly,
        purpose: "Approved website access and browser workflows; page instructions remain data.",
    },
    ModuleCatalogEntry {
        id: "MOD-MEDIA",
        name: "Media",
        category: ModuleCatalogCategory::Optional,
        parents: &[],
        status: ModuleCatalogStatus::DesignOnly,
        purpose: "Audio/video libraries and playback; outbound sharing is separately authorized.",
    },
    ModuleCatalogEntry {
        id: "MOD-DOCUMENTS",
        name: "Documents",
        category: ModuleCatalogCategory::Optional,
        parents: &[],
        status: ModuleCatalogStatus::DesignOnly,
        purpose: "PDF/Office generation, parsing, document revisions and exports.",
    },
    ModuleCatalogEntry {
        id: "MOD-COMMUNICATIONS",
        name: "Communications",
        category: ModuleCatalogCategory::Optional,
        parents: &[],
        status: ModuleCatalogStatus::DesignOnly,
        purpose: "Email, messages, future platforms, delivery receipts and distinct recipient grants.",
    },
    ModuleCatalogEntry {
        id: "MOD-SMART-HOME",
        name: "Smart Home",
        category: ModuleCatalogCategory::Optional,
        parents: &[],
        status: ModuleCatalogStatus::DesignOnly,
        purpose: "Appliances, lights and approved automations; locks and security need higher authority.",
    },
    ModuleCatalogEntry {
        id: "MOD-CAD",
        name: "CAD",
        category: ModuleCatalogCategory::Optional,
        parents: &[],
        status: ModuleCatalogStatus::DesignOnly,
        purpose: "Parametric designs, engineering drawings, models and validation.",
    },
    ModuleCatalogEntry {
        id: "MOD-SENSORS",
        name: "Sensors",
        category: ModuleCatalogCategory::Optional,
        parents: &[],
        status: ModuleCatalogStatus::DesignOnly,
        purpose: "Physical sensor acquisition with identity, calibration, freshness and uncertainty.",
    },
    ModuleCatalogEntry {
        id: "MOD-REMOTE-ACCESS",
        name: "Remote Access",
        category: ModuleCatalogCategory::Optional,
        parents: &[],
        status: ModuleCatalogStatus::DesignOnly,
        purpose: "Approved remote terminal/desktop sessions with consented, traceable commands.",
    },
    ModuleCatalogEntry {
        id: "MOD-IT-HELPDESK",
        name: "IT Helpdesk",
        category: ModuleCatalogCategory::Optional,
        parents: &[],
        status: ModuleCatalogStatus::DesignOnly,
        purpose: "Diagnose and prepare tickets/runbooks; non-destructive by default.",
    },
    ModuleCatalogEntry {
        id: "MOD-IT-NETWORK",
        name: "IT Networking",
        category: ModuleCatalogCategory::Optional,
        parents: &[],
        status: ModuleCatalogStatus::DesignOnly,
        purpose: "DNS/DHCP/latency/route/VLAN/Wi-Fi diagnostics with explicitly scoped discovery.",
    },
    ModuleCatalogEntry {
        id: "MOD-IT-SYSTEMS",
        name: "IT Systems",
        category: ModuleCatalogCategory::Optional,
        parents: &[],
        status: ModuleCatalogStatus::DesignOnly,
        purpose: "Windows/Linux services, users, packages and logs; changes are separately granted.",
    },
    ModuleCatalogEntry {
        id: "MOD-IT-CONTAINERS",
        name: "IT Containers",
        category: ModuleCatalogCategory::Optional,
        parents: &[],
        status: ModuleCatalogStatus::DesignOnly,
        purpose: "Docker/Compose/Portainer inventory, logs and explicit-grant deployments.",
    },
    ModuleCatalogEntry {
        id: "MOD-IT-STORAGE",
        name: "IT Storage",
        category: ModuleCatalogCategory::Optional,
        parents: &[],
        status: ModuleCatalogStatus::DesignOnly,
        purpose: "SMART, capacity, mounts and NAS; disk repartition and erase stay protected.",
    },
    ModuleCatalogEntry {
        id: "MOD-IT-DATABASE",
        name: "IT Databases",
        category: ModuleCatalogCategory::Optional,
        parents: &[],
        status: ModuleCatalogStatus::DesignOnly,
        purpose: "Read-only queries, schemas and integrity; no canonical database admin bypass.",
    },
    ModuleCatalogEntry {
        id: "MOD-IT-BACKUP-DR",
        name: "IT Backup/DR",
        category: ModuleCatalogCategory::Optional,
        parents: &[],
        status: ModuleCatalogStatus::DesignOnly,
        purpose: "Backup status, sandbox restore verification and protected live restore plans.",
    },
    ModuleCatalogEntry {
        id: "MOD-IT-SECURITY",
        name: "IT Security",
        category: ModuleCatalogCategory::Optional,
        parents: &[],
        status: ModuleCatalogStatus::DesignOnly,
        purpose: "Defensive patching, vulnerability, ACL, certificate and PKI audits.",
    },
    ModuleCatalogEntry {
        id: "MOD-IT-OBSERVABILITY",
        name: "IT Monitoring",
        category: ModuleCatalogCategory::Optional,
        parents: &[],
        status: ModuleCatalogStatus::DesignOnly,
        purpose: "Metrics, logs, alerts, incidents and service-objective reports.",
    },
    ModuleCatalogEntry {
        id: "MOD-IT-DEVOPS",
        name: "IT DevOps",
        category: ModuleCatalogCategory::Optional,
        parents: &[],
        status: ModuleCatalogStatus::DesignOnly,
        purpose: "CI/CD and infrastructure-as-code review with protected publish/apply.",
    },
    ModuleCatalogEntry {
        id: "MOD-ENGINEERING",
        name: "Engineering and Electrical",
        category: ModuleCatalogCategory::Optional,
        parents: &[],
        status: ModuleCatalogStatus::DesignOnly,
        purpose: "Engineering/electrical calculations, diagrams, standards references and safety checks.",
    },
    ModuleCatalogEntry {
        id: "MOD-ELECTRONICS",
        name: "Electronics and IoT",
        category: ModuleCatalogCategory::Optional,
        parents: &[],
        status: ModuleCatalogStatus::DesignOnly,
        purpose: "Arduino/ESPHome/microcontroller development, firmware artifacts and test rigs.",
    },
    ModuleCatalogEntry {
        id: "MOD-3D-PRINT",
        name: "3D Printing",
        category: ModuleCatalogCategory::Optional,
        parents: &[],
        status: ModuleCatalogStatus::DesignOnly,
        purpose: "Slicing, printer telemetry, approved print submission and job monitoring.",
    },
    ModuleCatalogEntry {
        id: "MOD-ENERGY",
        name: "Energy and UPS",
        category: ModuleCatalogCategory::Optional,
        parents: &[],
        status: ModuleCatalogStatus::DesignOnly,
        purpose: "Power consumption, battery/UPS status, runtime advice and controlled load shedding.",
    },
    ModuleCatalogEntry {
        id: "MOD-AVATAR3D",
        name: "3D Avatar",
        category: ModuleCatalogCategory::Optional,
        parents: &[],
        status: ModuleCatalogStatus::DesignOnly,
        purpose: "3D rig, full-body animation, physics and actual frame receipts.",
    },
    ModuleCatalogEntry {
        id: "MOD-AVATAR-STUDIO",
        name: "Avatar Studio",
        category: ModuleCatalogCategory::Optional,
        parents: &[],
        status: ModuleCatalogStatus::DesignOnly,
        purpose: "Wardrobe and scene authoring with layered rig compatibility and asset review.",
    },
    ModuleCatalogEntry {
        id: "MOD-WORKSHOP",
        name: "Hephaestus Workshop",
        category: ModuleCatalogCategory::Optional,
        parents: &[],
        status: ModuleCatalogStatus::DesignOnly,
        purpose: "Isolated VM experiments, guest preparation and signed lab artifacts.",
    },
    ModuleCatalogEntry {
        id: "MOD-DIGITAL-TWIN",
        name: "Digital Twin",
        category: ModuleCatalogCategory::Optional,
        parents: &[],
        status: ModuleCatalogStatus::DesignOnly,
        purpose: "Hypothetical infrastructure dependency and failure scenarios, not live effects.",
    },
    ModuleCatalogEntry {
        id: "MOD-KNOWLEDGE-LIBRARY",
        name: "Knowledge Library",
        category: ModuleCatalogCategory::Optional,
        parents: &[],
        status: ModuleCatalogStatus::DesignOnly,
        purpose: "External indexed references, bounded retrieval and source revisions.",
    },
    ModuleCatalogEntry {
        id: "MOD-PROJECT-FORGE",
        name: "Project Forge",
        category: ModuleCatalogCategory::Optional,
        parents: &[],
        status: ModuleCatalogStatus::DesignOnly,
        purpose: "Artifact generation, run management, evaluation and delivery; Core owns personal goals.",
    },
    ModuleCatalogEntry {
        id: "MOD-FILE-WATCH",
        name: "File Watch",
        category: ModuleCatalogCategory::Optional,
        parents: &[],
        status: ModuleCatalogStatus::DesignOnly,
        purpose: "Approved directory-change observation and permitted workflow triggers.",
    },
    ModuleCatalogEntry {
        id: "MOD-HOUSEHOLD",
        name: "Household",
        category: ModuleCatalogCategory::Optional,
        parents: &[],
        status: ModuleCatalogStatus::DesignOnly,
        purpose: "Grocy, Mealie, Homebox and consented inventory/meal workflows.",
    },
    ModuleCatalogEntry {
        id: "MOD-FINANCE",
        name: "Personal Finance",
        category: ModuleCatalogCategory::Optional,
        parents: &[],
        status: ModuleCatalogStatus::DesignOnly,
        purpose: "Firefly III and budget analysis with distinct sensitive-data grants.",
    },
    ModuleCatalogEntry {
        id: "MOD-DATA-ANALYTICS",
        name: "Data Analytics",
        category: ModuleCatalogCategory::Optional,
        parents: &[],
        status: ModuleCatalogStatus::DesignOnly,
        purpose: "CSV/structured data, statistics, reports and charts.",
    },
    ModuleCatalogEntry {
        id: "MOD-CALENDAR",
        name: "Calendar",
        category: ModuleCatalogCategory::Optional,
        parents: &[],
        status: ModuleCatalogStatus::DesignOnly,
        purpose: "Provider event sync, conflict checks, meeting preparation and action receipts.",
    },
    ModuleCatalogEntry {
        id: "MOD-INTERACTION",
        name: "Avatar Interaction",
        category: ModuleCatalogCategory::Submodule,
        parents: &["RM-AVATAR2D", "MOD-AVATAR3D", "MOD-VWORLD"],
        status: ModuleCatalogStatus::DesignOnly,
        purpose: "Avatar pointer hit tests, user-offered virtual interactions and gesture parsing; Core chooses its response.",
    },
    ModuleCatalogEntry {
        id: "MOD-VTUBER-BRIDGE",
        name: "VTube Studio Bridge",
        category: ModuleCatalogCategory::Adapter,
        parents: &["RM-AVATAR2D"],
        status: ModuleCatalogStatus::DesignOnly,
        purpose: "Owner-paired external 2D renderer adapter with no independent personality or render claims.",
    },
    ModuleCatalogEntry {
        id: "MOD-MEMORY-IMPORT",
        name: "Memory Import",
        category: ModuleCatalogCategory::Adapter,
        parents: &["PLAT-02", "core-memory"],
        status: ModuleCatalogStatus::DesignOnly,
        purpose: "Reviewed foreign memory/conversation imports; canonical promotion stays with the memory owner.",
    },
];

/// Return the complete recovered catalog as a static slice.
pub fn catalog() -> &'static [ModuleCatalogEntry] {
    &ENTRIES
}

/// Find an entry by exact identifier.
pub fn find(id: &str) -> Option<&'static ModuleCatalogEntry> {
    ENTRIES.iter().find(|entry| entry.id == id)
}

/// Return the number of catalog entries.
pub fn len() -> usize {
    ENTRIES.len()
}

/// Count entries in one category.
pub fn count(category: ModuleCatalogCategory) -> usize {
    ENTRIES
        .iter()
        .filter(|entry| entry.category == category)
        .count()
}

/// Count entries per category and in total.
pub fn counts() -> CatalogCounts {
    CatalogCounts {
        companion: count(ModuleCatalogCategory::Companion),
        optional: count(ModuleCatalogCategory::Optional),
        submodule: count(ModuleCatalogCategory::Submodule),
        adapter: count(ModuleCatalogCategory::Adapter),
        total: len(),
    }
}

/// Validate the recovered catalog's internal invariants.
///
/// Checks the expected total, unique identifiers, per-category counts,
/// non-empty required text, attached-versus-standalone parent rules, and that
/// every parent resolves to a catalog entry or an [`EXTERNAL_PARENTS`] anchor.
pub fn validate() -> Result<(), CatalogError> {
    if ENTRIES.len() != TOTAL_ENTRIES {
        return Err(CatalogError::WrongTotal {
            expected: TOTAL_ENTRIES,
            actual: ENTRIES.len(),
        });
    }

    let mut seen = BTreeSet::new();
    for entry in ENTRIES.iter() {
        if !seen.insert(entry.id) {
            return Err(CatalogError::DuplicateId(entry.id));
        }
        if entry.id.trim().is_empty() {
            return Err(CatalogError::EmptyField(entry.id, "id"));
        }
        if entry.name.trim().is_empty() {
            return Err(CatalogError::EmptyField(entry.id, "name"));
        }
        if entry.purpose.trim().is_empty() {
            return Err(CatalogError::EmptyField(entry.id, "purpose"));
        }

        match entry.category {
            ModuleCatalogCategory::Companion | ModuleCatalogCategory::Optional => {
                if !entry.parents.is_empty() {
                    return Err(CatalogError::UnexpectedParent(entry.id));
                }
            }
            ModuleCatalogCategory::Submodule | ModuleCatalogCategory::Adapter => {
                if entry.parents.is_empty() {
                    return Err(CatalogError::MissingParent(entry.id));
                }
            }
        }

        for parent in entry.parents {
            if *parent == entry.id || (find(parent).is_none() && !EXTERNAL_PARENTS.contains(parent))
            {
                return Err(CatalogError::UnknownParent {
                    id: entry.id,
                    parent,
                });
            }
        }
    }

    let actual = counts();
    for (category, expected, actual) in [
        (
            ModuleCatalogCategory::Companion,
            COMPANION_COUNT,
            actual.companion,
        ),
        (
            ModuleCatalogCategory::Optional,
            OPTIONAL_COUNT,
            actual.optional,
        ),
        (
            ModuleCatalogCategory::Submodule,
            SUBMODULE_COUNT,
            actual.submodule,
        ),
        (
            ModuleCatalogCategory::Adapter,
            ADAPTER_COUNT,
            actual.adapter,
        ),
    ] {
        if actual != expected {
            return Err(CatalogError::CategoryCountMismatch {
                category,
                expected,
                actual,
            });
        }
    }

    Ok(())
}
