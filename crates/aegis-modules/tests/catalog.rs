use std::collections::BTreeSet;

use aegis_modules::catalog::{
    catalog, count, counts, find, len, validate, CatalogCounts, ModuleCatalogCategory,
    ModuleCatalogStatus, ADAPTER_COUNT, COMPANION_COUNT, EXTERNAL_PARENTS, OPTIONAL_COUNT,
    SUBMODULE_COUNT, TOTAL_ENTRIES,
};

const EXPECTED_IDS: [&str; TOTAL_ENTRIES] = [
    "RM-DESKTOP",
    "RM-DISCORD",
    "RM-AVATAR2D",
    "RM-VOICE",
    "RM-MOBILE",
    "MOD-DEV",
    "MOD-RESEARCH",
    "MOD-CREATIVE",
    "MOD-VISION",
    "MOD-DESKTOP-ACT",
    "MOD-HOMELAB",
    "MOD-ROBOTICS-GAIA",
    "MOD-TRAINS",
    "MOD-GAMES",
    "MOD-PRODUCTIVITY",
    "MOD-ML",
    "MOD-GENERALIZATION",
    "MOD-VWORLD",
    "MOD-MENTOR",
    "MOD-BROWSER",
    "MOD-MEDIA",
    "MOD-DOCUMENTS",
    "MOD-COMMUNICATIONS",
    "MOD-SMART-HOME",
    "MOD-CAD",
    "MOD-SENSORS",
    "MOD-REMOTE-ACCESS",
    "MOD-IT-HELPDESK",
    "MOD-IT-NETWORK",
    "MOD-IT-SYSTEMS",
    "MOD-IT-CONTAINERS",
    "MOD-IT-STORAGE",
    "MOD-IT-DATABASE",
    "MOD-IT-BACKUP-DR",
    "MOD-IT-SECURITY",
    "MOD-IT-OBSERVABILITY",
    "MOD-IT-DEVOPS",
    "MOD-ENGINEERING",
    "MOD-ELECTRONICS",
    "MOD-3D-PRINT",
    "MOD-ENERGY",
    "MOD-AVATAR3D",
    "MOD-AVATAR-STUDIO",
    "MOD-WORKSHOP",
    "MOD-DIGITAL-TWIN",
    "MOD-KNOWLEDGE-LIBRARY",
    "MOD-PROJECT-FORGE",
    "MOD-FILE-WATCH",
    "MOD-HOUSEHOLD",
    "MOD-FINANCE",
    "MOD-DATA-ANALYTICS",
    "MOD-CALENDAR",
    "MOD-INTERACTION",
    "MOD-VTUBER-BRIDGE",
    "MOD-MEMORY-IMPORT",
];

#[test]
fn catalog_is_valid() {
    validate().expect("recovered catalog must satisfy its invariants");
}

#[test]
fn catalog_has_exactly_the_55_historical_ids() {
    assert_eq!(len(), 55);
    assert_eq!(TOTAL_ENTRIES, 55);

    let actual: BTreeSet<&str> = catalog().iter().map(|entry| entry.id).collect();
    let expected: BTreeSet<&str> = EXPECTED_IDS.into_iter().collect();
    assert_eq!(actual, expected);

    assert_eq!(catalog().first().map(|entry| entry.id), Some("RM-DESKTOP"));
    assert_eq!(
        catalog().last().map(|entry| entry.id),
        Some("MOD-MEMORY-IMPORT")
    );
}

#[test]
fn catalog_counts_match_historical_shape() {
    assert_eq!(
        counts(),
        CatalogCounts {
            companion: 5,
            optional: 47,
            submodule: 1,
            adapter: 2,
            total: 55,
        }
    );
    assert_eq!(count(ModuleCatalogCategory::Companion), COMPANION_COUNT);
    assert_eq!(count(ModuleCatalogCategory::Optional), OPTIONAL_COUNT);
    assert_eq!(count(ModuleCatalogCategory::Submodule), SUBMODULE_COUNT);
    assert_eq!(count(ModuleCatalogCategory::Adapter), ADAPTER_COUNT);
    assert_eq!(
        COMPANION_COUNT + OPTIONAL_COUNT + SUBMODULE_COUNT + ADAPTER_COUNT,
        TOTAL_ENTRIES
    );
}

#[test]
fn every_entry_is_design_only_and_has_text() {
    for entry in catalog() {
        assert_eq!(entry.status, ModuleCatalogStatus::DesignOnly);
        assert!(!entry.id.trim().is_empty());
        assert!(!entry.name.trim().is_empty());
        assert!(!entry.purpose.trim().is_empty());
    }
}

#[test]
fn ids_are_unique() {
    let mut seen = BTreeSet::new();
    for entry in catalog() {
        assert!(seen.insert(entry.id), "duplicate id {}", entry.id);
    }
    assert_eq!(seen.len(), 55);
}

#[test]
fn find_resolves_known_ids_and_rejects_unknown_or_wrong_case() {
    let dev = find("MOD-DEV").expect("MOD-DEV is cataloged");
    assert_eq!(dev.category, ModuleCatalogCategory::Optional);
    assert_eq!(dev.status, ModuleCatalogStatus::DesignOnly);

    assert!(find("MOD-DOES-NOT-EXIST").is_none());
    assert!(find("mod-dev").is_none());
    assert!(find("").is_none());
}

#[test]
fn attached_entries_declare_their_historical_parents() {
    let interaction = find("MOD-INTERACTION").expect("submodule");
    assert_eq!(interaction.category, ModuleCatalogCategory::Submodule);
    assert_eq!(
        interaction.parents,
        &["RM-AVATAR2D", "MOD-AVATAR3D", "MOD-VWORLD"]
    );

    let vtuber = find("MOD-VTUBER-BRIDGE").expect("adapter");
    assert_eq!(vtuber.category, ModuleCatalogCategory::Adapter);
    assert_eq!(vtuber.parents, &["RM-AVATAR2D"]);

    let memory = find("MOD-MEMORY-IMPORT").expect("adapter");
    assert_eq!(memory.category, ModuleCatalogCategory::Adapter);
    assert_eq!(memory.parents, &["PLAT-02", "core-memory"]);
}

#[test]
fn companion_and_optional_entries_are_standalone() {
    for entry in catalog() {
        if matches!(
            entry.category,
            ModuleCatalogCategory::Companion | ModuleCatalogCategory::Optional
        ) {
            assert!(
                entry.parents.is_empty(),
                "{} must not declare parents",
                entry.id
            );
        }
    }
}

#[test]
fn all_parent_references_resolve() {
    for entry in catalog() {
        for parent in entry.parents {
            assert!(
                find(parent).is_some() || EXTERNAL_PARENTS.contains(parent),
                "{} references unresolved parent {}",
                entry.id,
                parent
            );
        }
    }

    for parent in EXTERNAL_PARENTS {
        assert!(
            find(parent).is_none(),
            "external anchor {parent} must not also be a catalog entry"
        );
    }
}
