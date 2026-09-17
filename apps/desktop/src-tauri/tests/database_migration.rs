use partnest_desktop_lib::db::{new_id, BoxRecord, Database, Migration, PartRecord};
use rusqlite::params;
use std::fs;
use std::path::{Path, PathBuf};

fn test_database() -> Database {
    let path = test_path("db");
    Database::open_with_migrations(
        &path,
        &[
            Migration {
                version: 1,
                sql: include_str!("../migrations/0001_initial.sql"),
            },
            Migration {
                version: 2,
                sql: include_str!("../migrations/0002_welding_movement_metadata.sql"),
            },
            Migration {
                version: 3,
                sql: include_str!("../migrations/0003_movement_audit_and_active_session.sql"),
            },
            Migration {
                version: 4,
                sql: include_str!("../migrations/0004_lcsc_cache.sql"),
            },
            Migration {
                version: 5,
                sql: include_str!("../migrations/0005_allow_overconsumption.sql"),
            },
            Migration {
                version: 6,
                sql: include_str!("../migrations/0006_normalize_lcsc_codes.sql"),
            },
        ],
    )
    .expect("open test database")
}

/// A migration file that is never registered would silently leave databases
/// behind, so the file set and the code set must stay identical.
#[test]
fn every_migration_file_is_registered_in_order_and_matches_the_schema_version() {
    let directory = Path::new(env!("CARGO_MANIFEST_DIR")).join("migrations");
    let mut on_disk = fs::read_dir(&directory)
        .expect("migrations directory")
        .map(|entry| {
            let name = entry.expect("migration entry").file_name();
            let name = name.to_string_lossy().into_owned();
            name.split('_')
                .next()
                .and_then(|prefix| prefix.parse::<i64>().ok())
                .unwrap_or_else(|| panic!("{name} does not start with a version prefix"))
        })
        .collect::<Vec<_>>();
    on_disk.sort_unstable();

    let registered = partnest_desktop_lib::db::migrations()
        .into_iter()
        .map(|migration| migration.version)
        .collect::<Vec<_>>();
    assert_eq!(
        registered, on_disk,
        "registered migrations must match migrations/"
    );
    assert_eq!(
        on_disk.last().copied(),
        Some(partnest_desktop_lib::backup::CURRENT_SCHEMA_VERSION),
        "the newest migration must be the schema version backups are validated against"
    );
}

fn test_path(suffix: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "partnest-test-{}-{}-{suffix}",
        std::process::id(),
        new_id()
    ))
}

fn uuid7_id() -> String {
    let id = new_id();
    assert_eq!(uuid::Uuid::parse_str(&id).unwrap().get_version_num(), 7);
    id
}

fn insert_box(db: &Database, id: &str) {
    db.connection()
        .execute(
            "INSERT INTO boxes (id, name, rows, cols) VALUES (?1, ?2, ?3, ?4)",
            params![id, "Test box", 4_i64, 4_i64],
        )
        .expect("insert box");
}

fn insert_part(db: &Database, box_id: &str, slot: &str, quantity: i64) -> rusqlite::Result<usize> {
    db.connection().execute(
        "INSERT INTO parts (id, name, category, package, manufacturer, mpn, lcsc_code, quantity, box_id, slot, note, version)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, NULL, ?7, ?8, ?9, ?10, 1)",
        params![uuid7_id(), "Part", "resistor", "0603", "Acme", "R-1", quantity, box_id, slot, ""],
    )
}

#[test]
fn migration_enforces_slot_and_quantity_constraints() {
    let db = test_database();
    insert_box(&db, "box-1");
    assert!(insert_part(&db, "box-1", "A0", 10).is_ok());
    assert!(insert_part(&db, "box-1", "A0", 2).is_err());
    assert!(insert_part(&db, "box-1", "A1", -1).is_err());
}

#[test]
fn migration_enforces_side_and_lcsc_constraints() {
    let db = test_database();
    insert_box(&db, "box-1");
    assert!(
        db.connection()
            .execute(
                "INSERT INTO parts (id, name, quantity, box_id, slot, lcsc_code) VALUES (?1, 'A', 1, ?2, 'A0', 'C1')",
                params![uuid7_id(), "box-1"],
            )
            .is_ok()
    );
    assert!(
        db.connection()
            .execute(
                "INSERT INTO parts (id, name, quantity, box_id, slot, lcsc_code) VALUES (?1, 'B', 1, ?2, 'A1', 'C1')",
                params![uuid7_id(), "box-1"],
            )
            .is_err()
    );

    db.connection()
        .execute(
            "INSERT INTO bom_files (id, original_name, display_name, sha256, cache_name) VALUES ('bom-1', 'a.html', 'A', 'hash', 'hash.html')",
            [],
        )
        .unwrap();
    db.connection()
        .execute(
            "INSERT INTO welding_sessions (id, bom_file_id, status) VALUES ('session-1', 'bom-1', 'active')",
            [],
        )
        .unwrap();
    assert!(db
        .connection()
        .execute(
            "INSERT INTO welding_progress (id, session_id, component_key, side, required_quantity, taken_quantity) VALUES ('progress-1', 'session-1', 'R1', 'left', 1, 0)",
            [],
        )
        .is_err());
}

#[test]
fn migration_creates_lcsc_cache_without_changing_inventory_records() {
    let db = test_database();
    db.connection()
        .execute(
            "INSERT INTO lcsc_cache (lcsc_code, name, category, package, manufacturer, mpn, fetched_at) VALUES ('C25804', '100k', '电阻', '0402', 'UNI-ROYAL', '0402WGF1003TEE', '2026-08-24T00:00:00Z')",
            [],
        )
        .expect("cache table accepts a lookup record");
    let stored: String = db
        .connection()
        .query_row(
            "SELECT mpn FROM lcsc_cache WHERE lcsc_code = 'C25804'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(stored, "0402WGF1003TEE");
    assert_eq!(
        db.connection()
            .query_row::<i64, _, _>("SELECT COUNT(*) FROM parts", [], |row| row.get(0))
            .unwrap(),
        0
    );
}

#[test]
fn migration_normalizes_existing_lcsc_codes_before_rebuilding_uniqueness() {
    let path = test_path("lcsc-normalization");
    let initial = include_str!("../migrations/0001_initial.sql");
    let welding = include_str!("../migrations/0002_welding_movement_metadata.sql");
    let audit = include_str!("../migrations/0003_movement_audit_and_active_session.sql");
    let cache = include_str!("../migrations/0004_lcsc_cache.sql");
    let db = Database::open_with_migrations(
        &path,
        &[
            Migration {
                version: 1,
                sql: initial,
            },
            Migration {
                version: 2,
                sql: welding,
            },
            Migration {
                version: 3,
                sql: audit,
            },
            Migration {
                version: 4,
                sql: cache,
            },
        ],
    )
    .unwrap();
    insert_box(&db, "box-1");
    db.connection()
        .execute(
            "INSERT INTO parts (id, name, quantity, box_id, slot, lcsc_code) VALUES (?1, 'Part', 1, 'box-1', 'A0', ' c123 ')",
            params![uuid7_id()],
        )
        .unwrap();
    drop(db);

    let upgraded = Database::open(&path).unwrap();
    let code: String = upgraded
        .connection()
        .query_row("SELECT lcsc_code FROM parts WHERE slot = 'A0'", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(code, "C123");
}

#[test]
fn migration_rebuilds_box_ids_as_integers_and_clears_zero_stock_locations() {
    let path = test_path("integer-box-ids");
    let migrations = [
        Migration {
            version: 1,
            sql: include_str!("../migrations/0001_initial.sql"),
        },
        Migration {
            version: 2,
            sql: include_str!("../migrations/0002_welding_movement_metadata.sql"),
        },
        Migration {
            version: 3,
            sql: include_str!("../migrations/0003_movement_audit_and_active_session.sql"),
        },
        Migration {
            version: 4,
            sql: include_str!("../migrations/0004_lcsc_cache.sql"),
        },
        Migration {
            version: 5,
            sql: include_str!("../migrations/0005_allow_overconsumption.sql"),
        },
        Migration {
            version: 6,
            sql: include_str!("../migrations/0006_normalize_lcsc_codes.sql"),
        },
    ];
    let db = Database::open_with_migrations(&path, &migrations).unwrap();
    insert_box(&db, "legacy-box-a");
    db.connection()
        .execute(
            "INSERT INTO boxes (id, name, rows, cols) VALUES ('legacy-box-b', 'Second', 2, 2)",
            [],
        )
        .unwrap();
    db.connection()
        .execute(
            "INSERT INTO parts (id, name, quantity, box_id, slot, version) VALUES ('part-positive', 'Positive', 2, 'legacy-box-a', 'A0', 1)",
            [],
        )
        .unwrap();
    db.connection()
        .execute(
            "INSERT INTO parts (id, name, quantity, box_id, slot, version) VALUES ('part-empty', 'Empty', 0, 'legacy-box-b', 'A1', 1)",
            [],
        )
        .unwrap();
    db.connection()
        .execute(
            "INSERT INTO bom_files (id, original_name, display_name, sha256, cache_name) VALUES ('bom-1', 'board.html', 'Board', 'hash', 'hash.html')",
            [],
        )
        .unwrap();
    db.connection()
        .execute(
            "INSERT INTO welding_sessions (id, bom_file_id, status) VALUES ('session-1', 'bom-1', 'active')",
            [],
        )
        .unwrap();
    db.connection()
        .execute(
            "INSERT INTO welding_progress (id, session_id, component_key, side, part_id, required_quantity, taken_quantity) VALUES ('progress-1', 'session-1', 'R1', 'top', 'part-positive', 1, 0)",
            [],
        )
        .unwrap();
    db.connection()
        .execute(
            "INSERT INTO inventory_movements (id, part_id, session_id, movement_type, quantity, reason) VALUES ('movement-1', 'part-positive', 'session-1', 'consume', -1, 'take')",
            [],
        )
        .unwrap();
    drop(db);

    let upgraded = Database::open(&path).unwrap();
    let box_id: i64 = upgraded
        .connection()
        .query_row("SELECT id FROM boxes WHERE name = 'Second'", [], |row| {
            row.get(0)
        })
        .unwrap();
    let positive_box_id: i64 = upgraded
        .connection()
        .query_row(
            "SELECT box_id FROM parts WHERE id = 'part-positive'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert!(box_id > 0);
    assert_eq!(
        upgraded
            .connection()
            .query_row(
                "SELECT typeof(id) FROM boxes WHERE name = 'Second'",
                [],
                |row| row.get::<_, String>(0),
            )
            .unwrap(),
        "integer"
    );
    assert_eq!(
        upgraded
            .connection()
            .query_row(
                "SELECT box_id, slot FROM parts WHERE id = 'part-positive'",
                [],
                |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)),
            )
            .unwrap(),
        (positive_box_id, "A0".to_owned())
    );
    assert_eq!(
        upgraded
            .connection()
            .query_row(
                "SELECT box_id, slot FROM parts WHERE id = 'part-empty'",
                [],
                |row| Ok((
                    row.get::<_, Option<i64>>(0)?,
                    row.get::<_, Option<String>>(1)?
                )),
            )
            .unwrap(),
        (None, None)
    );
    assert_eq!(
        upgraded
            .connection()
            .query_row(
                "SELECT part_id FROM welding_progress WHERE id = 'progress-1'",
                [],
                |row| row.get::<_, Option<String>>(0),
            )
            .unwrap(),
        Some("part-positive".to_owned())
    );
    assert_eq!(
        upgraded
            .connection()
            .query_row(
                "SELECT part_id FROM inventory_movements WHERE id = 'movement-1'",
                [],
                |row| row.get::<_, Option<String>>(0),
            )
            .unwrap(),
        Some("part-positive".to_owned())
    );
}

#[test]
fn migration_enforces_progress_uniqueness_and_quantity_bounds() {
    let db = test_database();
    insert_box(&db, "box-1");
    db.connection().execute("INSERT INTO bom_files (id, original_name, display_name, sha256, cache_name) VALUES ('bom-1', 'a.html', 'A', 'hash', 'hash.html')", []).unwrap();
    db.connection().execute("INSERT INTO welding_sessions (id, bom_file_id, status) VALUES ('session-1', 'bom-1', 'active')", []).unwrap();
    let valid = "INSERT INTO welding_progress (id, session_id, component_key, side, required_quantity, taken_quantity) VALUES (?1, 'session-1', 'R1', 'top', 2, 1)";
    db.connection()
        .execute(valid, rusqlite::params![uuid7_id()])
        .unwrap();
    assert!(db
        .connection()
        .execute(valid, rusqlite::params![uuid7_id()])
        .is_err());
    assert!(db.connection().execute("INSERT INTO welding_progress (id, session_id, component_key, side, required_quantity, taken_quantity) VALUES (?1, 'session-1', 'R2', 'top', 1, 2)", rusqlite::params![uuid7_id()]).is_ok());
    assert!(db.connection().execute("INSERT INTO welding_progress (id, session_id, component_key, side, required_quantity, taken_quantity) VALUES (?1, 'session-1', 'R3', 'top', -1, 0)", rusqlite::params![uuid7_id()]).is_err());
    assert!(db.connection().execute("INSERT INTO welding_progress (id, session_id, component_key, side, required_quantity, taken_quantity) VALUES (?1, 'session-1', 'R4', 'top', 1, -1)", rusqlite::params![uuid7_id()]).is_err());
}

#[test]
fn empty_and_null_lcsc_codes_are_not_unique() {
    let db = test_database();
    insert_box(&db, "box-1");
    for (slot, code) in [("A0", ""), ("A1", ""), ("A2", " ")] {
        assert!(db.connection().execute("INSERT INTO parts (id, name, quantity, box_id, slot, lcsc_code) VALUES (?1, 'Part', 1, 'box-1', ?2, ?3)", rusqlite::params![uuid7_id(), slot, code]).is_ok());
    }
    assert!(db.connection().execute("INSERT INTO parts (id, name, quantity, box_id, slot, lcsc_code) VALUES (?1, 'Part', 1, 'box-1', 'A3', NULL)", rusqlite::params![uuid7_id()]).is_ok());
    assert!(db.connection().execute("INSERT INTO parts (id, name, quantity, box_id, slot, lcsc_code) VALUES (?1, 'Part', 1, 'box-1', 'A4', NULL)", rusqlite::params![uuid7_id()]).is_ok());
}

#[test]
fn migration_initializes_pragmas_and_all_business_tables() {
    let db = test_database();
    assert_eq!(
        db.connection()
            .query_row("PRAGMA foreign_keys", [], |row| row.get::<_, i64>(0))
            .unwrap(),
        1
    );
    assert_eq!(
        db.connection()
            .query_row("PRAGMA busy_timeout", [], |row| row.get::<_, i64>(0))
            .unwrap(),
        5000
    );
    assert_eq!(
        db.connection()
            .query_row("PRAGMA journal_mode", [], |row| row.get::<_, String>(0))
            .unwrap(),
        "wal"
    );

    let mut statement = db
        .connection()
        .prepare("SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%'")
        .unwrap();
    let names = statement
        .query_map([], |row| row.get::<_, String>(0))
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap();
    for expected in [
        "boxes",
        "parts",
        "bom_files",
        "welding_sessions",
        "welding_progress",
        "inventory_movements",
    ] {
        assert!(
            names.iter().any(|name| name == expected),
            "missing {expected}"
        );
    }
}

#[test]
fn migrations_are_applied_transactionally_and_foreign_keys_keep_audit_rows() {
    let db = test_database();
    insert_box(&db, "box-1");
    let result = db.transaction().and_then(|tx| {
        tx.execute(
            "INSERT INTO boxes (id, name, rows, cols) VALUES ('box-2', 'Second', 1, 1)",
            [],
        )?;
        tx.rollback()
    });
    assert!(result.is_ok());
    assert_eq!(
        db.connection()
            .query_row("SELECT COUNT(*) FROM boxes WHERE id = 'box-2'", [], |row| {
                row.get::<_, i64>(0)
            })
            .unwrap(),
        0
    );
}

#[test]
fn migration_runner_rolls_back_failures_and_reopens_idempotently() {
    let path = test_path("migration");
    let failing = [Migration {
        version: 1,
        sql: "CREATE TABLE partial (id INTEGER); INSERT INTO missing_table VALUES (1);",
    }];
    assert!(Database::open_with_migrations(&path, &failing).is_err());
    let valid = [Migration {
        version: 1,
        sql: "CREATE TABLE marker (id INTEGER PRIMARY KEY);",
    }];
    let db = Database::open_with_migrations(&path, &valid).unwrap();
    assert_eq!(
        db.connection()
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE name = 'partial'",
                [],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
        0
    );
    drop(db);
    let reopened = Database::open_with_migrations(&path, &valid).unwrap();
    assert_eq!(
        reopened
            .connection()
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'marker'",
                [],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
        1
    );
    assert_eq!(
        reopened
            .connection()
            .query_row(
                "SELECT COUNT(*) FROM schema_migrations WHERE version = 1",
                [],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
        1
    );
}

#[test]
fn constructors_keep_part_uuid_ids_and_use_zero_for_unsaved_boxes() {
    let box_record = BoxRecord::new("Box".into(), 2, 3);
    let part_record = PartRecord::new("Part".into(), Some(1), Some("A0".into()), 1);
    assert_eq!(box_record.id, 0);
    assert_eq!(
        uuid::Uuid::parse_str(&part_record.id)
            .unwrap()
            .get_version_num(),
        7
    );
}

#[test]
fn app_data_dir_seam_opens_partnest_database() {
    let app_data_dir = test_path("app-data");
    let db = Database::open_app_data_dir(&app_data_dir).unwrap();
    assert!(app_data_dir.join("partnest.db").is_file());
    assert_eq!(
        db.connection()
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE name = 'parts'",
                [],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
        1
    );
    drop(db);
    fs::remove_dir_all(&app_data_dir).unwrap();
}

#[test]
fn foreign_key_actions_preserve_audit_rows_and_enforce_restricts() {
    let db = test_database();
    insert_box(&db, "box-1");
    let part_id = uuid7_id();
    db.connection().execute("INSERT INTO parts (id, name, quantity, box_id, slot) VALUES (?1, 'Part', 1, 'box-1', 'A0')", rusqlite::params![part_id]).unwrap();
    db.connection().execute("INSERT INTO bom_files (id, original_name, display_name, sha256, cache_name) VALUES ('bom-1', 'a.html', 'A', 'hash', 'hash.html')", []).unwrap();
    db.connection().execute("INSERT INTO welding_sessions (id, bom_file_id, status) VALUES ('session-1', 'bom-1', 'active')", []).unwrap();
    db.connection().execute("INSERT INTO welding_progress (id, session_id, component_key, side, part_id, required_quantity, taken_quantity) VALUES ('progress-1', 'session-1', 'R1', 'top', ?1, 1, 0)", rusqlite::params![part_id]).unwrap();
    db.connection().execute("INSERT INTO inventory_movements (id, part_id, session_id, movement_type, quantity, reason) VALUES ('movement-1', ?1, 'session-1', 'consume', -1, 'take')", rusqlite::params![part_id]).unwrap();
    db.connection().execute("INSERT INTO inventory_movements (id, part_id, session_id, movement_type, quantity, reason, reverses_movement_id) VALUES ('movement-2', ?1, 'session-1', 'reverse', 1, 'undo', 'movement-1')", rusqlite::params![part_id]).unwrap();

    assert!(db
        .connection()
        .execute("DELETE FROM boxes WHERE id = 'box-1'", [])
        .is_err());
    assert!(db
        .connection()
        .execute("DELETE FROM bom_files WHERE id = 'bom-1'", [])
        .is_err());
    assert!(db
        .connection()
        .execute(
            "DELETE FROM inventory_movements WHERE id = 'movement-1'",
            []
        )
        .is_err());

    db.connection()
        .execute(
            "DELETE FROM parts WHERE id = ?1",
            rusqlite::params![part_id],
        )
        .unwrap();
    assert_eq!(
        db.connection()
            .query_row(
                "SELECT part_id FROM inventory_movements WHERE id = 'movement-1'",
                [],
                |row| row.get::<_, Option<String>>(0)
            )
            .unwrap(),
        None
    );
    assert_eq!(
        db.connection()
            .query_row(
                "SELECT part_id FROM welding_progress WHERE id = 'progress-1'",
                [],
                |row| row.get::<_, Option<String>>(0)
            )
            .unwrap(),
        None
    );

    db.connection()
        .execute("DELETE FROM welding_sessions WHERE id = 'session-1'", [])
        .unwrap();
    assert_eq!(
        db.connection()
            .query_row(
                "SELECT session_id FROM inventory_movements WHERE id = 'movement-1'",
                [],
                |row| row.get::<_, Option<String>>(0)
            )
            .unwrap(),
        None
    );
    assert_eq!(
        db.connection()
            .query_row(
                "SELECT COUNT(*) FROM welding_progress WHERE id = 'progress-1'",
                [],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
        0
    );
    assert_eq!(
        db.connection()
            .query_row(
                "SELECT COUNT(*) FROM inventory_movements WHERE id IN ('movement-1', 'movement-2')",
                [],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
        2
    );
    db.connection()
        .execute("DELETE FROM boxes WHERE id = 'box-1'", [])
        .unwrap();
}

#[test]
fn migration_adds_audit_order_and_nullable_legacy_metadata() {
    let db = test_database();
    for column in [
        "before_quantity",
        "after_quantity",
        "movement_sequence",
        "bom_quantity",
        "confirmation_designators",
    ] {
        assert_eq!(
            db.connection()
                .query_row(
                    "SELECT COUNT(*) FROM pragma_table_info('inventory_movements') WHERE name = ?1",
                    [column],
                    |row| row.get::<_, i64>(0),
                )
                .unwrap(),
            1,
            "missing movement metadata column {column}"
        );
    }
    db.connection()
        .execute(
            "INSERT INTO inventory_movements (id, movement_type, quantity, reason) VALUES ('legacy', 'adjust', 1, 'legacy')",
            [],
        )
        .unwrap();
    assert_eq!(
        db.connection()
            .query_row(
                "SELECT before_quantity, after_quantity, confirmation_designators FROM inventory_movements WHERE id = 'legacy'",
                [],
                |row| Ok((row.get::<_, Option<i64>>(0)?, row.get::<_, Option<i64>>(1)?, row.get::<_, Option<String>>(2)?)),
            )
            .unwrap(),
        (None, None, None)
    );
}

#[test]
fn migration_rejects_unknown_future_and_missing_versions() {
    let future_path = test_path("future-schema");
    {
        let connection = rusqlite::Connection::open(&future_path).unwrap();
        connection
            .execute_batch("CREATE TABLE schema_migrations (version INTEGER PRIMARY KEY, applied_at TEXT NOT NULL); INSERT INTO schema_migrations VALUES (99, 'now');")
            .unwrap();
    }
    assert!(Database::open(&future_path).is_err());

    let gap_path = test_path("gap-schema");
    {
        let connection = rusqlite::Connection::open(&gap_path).unwrap();
        connection
            .execute_batch("CREATE TABLE schema_migrations (version INTEGER PRIMARY KEY, applied_at TEXT NOT NULL); INSERT INTO schema_migrations VALUES (2, 'now');")
            .unwrap();
    }
    assert!(Database::open(&gap_path).is_err());
}

#[test]
fn migration_backfills_confirmed_designators_from_surviving_take_movements() {
    // Databases created before version 8 have no designator-level audit, so the
    // duplicate-take guard would be vacuous unless the take history is replayed.
    let path = test_path("legacy-confirmed-designators");
    let migrations = [
        Migration {
            version: 1,
            sql: include_str!("../migrations/0001_initial.sql"),
        },
        Migration {
            version: 2,
            sql: include_str!("../migrations/0002_welding_movement_metadata.sql"),
        },
        Migration {
            version: 3,
            sql: include_str!("../migrations/0003_movement_audit_and_active_session.sql"),
        },
        Migration {
            version: 4,
            sql: include_str!("../migrations/0004_lcsc_cache.sql"),
        },
        Migration {
            version: 5,
            sql: include_str!("../migrations/0005_allow_overconsumption.sql"),
        },
        Migration {
            version: 6,
            sql: include_str!("../migrations/0006_normalize_lcsc_codes.sql"),
        },
    ];
    let db = Database::open_with_migrations(&path, &migrations).expect("legacy database");
    insert_box(&db, "legacy-box");
    let part_id = uuid7_id();
    db.connection().execute("INSERT INTO parts (id, name, quantity, box_id, slot) VALUES (?1, 'Part', 3, 'legacy-box', 'A0')", rusqlite::params![part_id]).unwrap();
    db.connection().execute("INSERT INTO bom_files (id, original_name, display_name, sha256, cache_name) VALUES ('bom-1', 'a.html', 'A', 'hash', 'hash.html')", []).unwrap();
    db.connection().execute("INSERT INTO welding_sessions (id, bom_file_id, status) VALUES ('session-1', 'bom-1', 'active')", []).unwrap();
    db.connection().execute("INSERT INTO welding_progress (id, session_id, component_key, side, part_id, required_quantity, taken_quantity) VALUES ('progress-1', 'session-1', 'C1', 'top', ?1, 3, 3)", rusqlite::params![part_id]).unwrap();
    db.connection().execute("INSERT INTO welding_progress (id, session_id, component_key, side, part_id, required_quantity, taken_quantity) VALUES ('progress-2', 'session-1', 'C9', 'top', ?1, 1, 0)", rusqlite::params![part_id]).unwrap();
    db.connection().execute("INSERT INTO inventory_movements (id, part_id, session_id, movement_type, quantity, reason, component_key, side, confirmation_designators) VALUES ('take-1', ?1, 'session-1', 'consume', -2, 'take', 'C1', 'top', '[\"R1\",\"R2\"]')", rusqlite::params![part_id]).unwrap();
    db.connection().execute("INSERT INTO inventory_movements (id, part_id, session_id, movement_type, quantity, reason, component_key, side, confirmation_designators) VALUES ('take-2', ?1, 'session-1', 'consume', -1, 'take', 'C1', 'top', '[\"R3\"]')", rusqlite::params![part_id]).unwrap();
    db.connection().execute("INSERT INTO inventory_movements (id, part_id, session_id, movement_type, quantity, reason, reverses_movement_id, component_key, side) VALUES ('undo-1', ?1, 'session-1', 'reverse', 1, 'undo', 'take-2', 'C1', 'top')", rusqlite::params![part_id]).unwrap();
    drop(db);

    let upgraded = Database::open(&path).expect("upgrade to the confirmed designator schema");
    let confirmed = |component_key: &str| -> Vec<String> {
        let raw = upgraded
            .connection()
            .query_row(
                "SELECT confirmed_designators FROM welding_progress WHERE component_key = ?1",
                [component_key],
                |row| row.get::<_, String>(0),
            )
            .unwrap();
        let mut values: Vec<String> = serde_json::from_str(&raw).unwrap();
        values.sort();
        values
    };
    assert_eq!(confirmed("C1"), ["R1".to_owned(), "R2".to_owned()]);
    assert_eq!(confirmed("C9"), Vec::<String>::new());
}

#[test]
fn projects_migration_renames_the_bom_rows_without_losing_sessions_or_audit() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("upgrade.db");
    let migrations = partnest_desktop_lib::db::migrations();
    let db = Database::open_with_migrations(&path, &migrations[..10]).unwrap();
    db.connection().execute_batch(
        "INSERT INTO boxes (id, name, rows, cols) VALUES (1, 'Bench', 2, 2);
         INSERT INTO parts (id, name, quantity, box_id, slot) VALUES ('p', 'Resistor', 8, 1, 'A0');
         INSERT INTO bom_files (id, original_name, display_name, sha256, cache_name, kind, normalized_json)
             VALUES ('bom-1', 'board.html', '主板', 'hash', 'hash.html', 'interactive', NULL);
         INSERT INTO welding_sessions (id, bom_file_id, status) VALUES ('s', 'bom-1', 'active');
         INSERT INTO inventory_movements (id, part_id, session_id, movement_type, quantity, reason)
             VALUES ('take', 'p', 's', 'consume', -2, '焊接取用');",
    )
    .unwrap();
    drop(db);

    let db = Database::open(&path).unwrap();
    let project: (String, String, String, String) = db
        .connection()
        .query_row(
            "SELECT p.id, p.name, p.original_name, s.status FROM projects p JOIN welding_sessions s ON s.project_id = p.id WHERE s.id = 's'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .unwrap();
    assert_eq!(
        project,
        (
            "bom-1".into(),
            "主板".into(),
            "board.html".into(),
            "active".into()
        ),
        "the imported BOM becomes the project its session already pointed at"
    );
    assert_eq!(
        db.connection()
            .query_row(
                "SELECT session_id FROM inventory_movements WHERE id = 'take'",
                [],
                |row| row.get::<_, String>(0),
            )
            .unwrap(),
        "s"
    );

    // The renamed table keeps the foreign key that protects the audit trail.
    assert!(db
        .connection()
        .execute("DELETE FROM projects WHERE id = 'bom-1'", [])
        .is_err());
    assert!(db
        .connection()
        .execute("INSERT INTO welding_sessions (id, project_id, status) VALUES ('s2', 'nope', 'completed')", [])
        .is_err());
}

#[test]
fn soft_delete_migration_preserves_v8_inventory_and_frees_only_archived_codes() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("upgrade.db");
    let migrations = partnest_desktop_lib::db::migrations();
    let db = Database::open_with_migrations(&path, &migrations[..8]).unwrap();
    db.connection().execute_batch("INSERT INTO boxes (id, name, rows, cols) VALUES (1, 'Bench', 2, 2);
        INSERT INTO parts (id, name, lcsc_code, quantity, box_id, slot) VALUES ('old', 'Resistor', 'C123', 7, 1, 'A0');
        INSERT INTO inventory_movements (id, part_id, movement_type, quantity, reason) VALUES ('history', 'old', 'in', 7, 'initial');").unwrap();
    drop(db);
    let db = Database::open(&path).unwrap();
    let original: (String, i64, Option<String>) = db
        .connection()
        .query_row(
            "SELECT name, quantity, deleted_at FROM parts WHERE id = 'old'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    assert_eq!(original, ("Resistor".into(), 7, None));
    assert!(db.connection().execute("INSERT INTO parts (id, name, lcsc_code, quantity) VALUES ('duplicate', 'Duplicate', 'c123', 0)", []).is_err());
    partnest_desktop_lib::commands::parts::delete_part_service(&db, "old").unwrap();
    db.connection().execute("INSERT INTO parts (id, name, lcsc_code, quantity, box_id, slot) VALUES ('replacement', 'New', 'C123', 3, 1, 'A0')", []).unwrap();
    assert!(db.connection().execute("INSERT INTO parts (id, name, lcsc_code, quantity) VALUES ('duplicate', 'Duplicate', 'c123', 0)", []).is_err());
    assert_eq!(
        db.connection()
            .query_row(
                "SELECT part_id FROM inventory_movements WHERE id = 'history'",
                [],
                |row| row.get::<_, String>(0)
            )
            .unwrap(),
        "old"
    );
    let parts = partnest_desktop_lib::commands::parts::list_parts_service(&db, None).unwrap();
    assert_eq!(parts.len(), 1);
    assert_eq!(parts[0].id, "replacement");
    drop(db);
    let reopened = Database::open(&path).unwrap();
    assert_eq!(
        partnest_desktop_lib::commands::parts::list_parts_service(&reopened, None).unwrap(),
        parts
    );
}
