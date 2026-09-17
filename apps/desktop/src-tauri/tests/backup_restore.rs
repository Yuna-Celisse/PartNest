use partnest_desktop_lib::{
    backup::{
        backup_dir, create_backup, maybe_create_startup_backup, restore_database_file,
        restore_database_file_with_injected_failure,
        restore_database_file_with_injected_live_reopen_failure,
        restore_database_file_with_injected_post_open_failure,
        restore_database_file_with_injected_rollback_failure, validate_backup, BackupError,
        STARTUP_BACKUP_AGE,
    },
    db::{new_id, Database},
};
use rusqlite::{Connection, OpenFlags};
use std::fs::{self, File};
use std::time::{Duration, SystemTime};
use tempfile::tempdir;

fn seed_database(path: &std::path::Path) -> Database {
    let db = Database::open(path).expect("open database");
    db.connection()
        .execute(
            "INSERT INTO boxes (name, rows, cols) VALUES ('Bench', 2, 2)",
            [],
        )
        .unwrap();
    let box_id = db.connection().last_insert_rowid();
    db.connection()
        .execute(
            "INSERT INTO parts (id, name, quantity, box_id, slot) VALUES ('part-1', '10k', 4, ?1, 'A0')",
            [box_id],
        )
        .unwrap();
    db.connection()
        .execute(
            "INSERT INTO projects (id, name, original_name, sha256, cache_name) VALUES ('project-1', 'Board note', 'board.html', 'hash', 'hash.html')",
            [],
        )
        .unwrap();
    db.connection()
        .execute(
            "INSERT INTO welding_sessions (id, project_id, status) VALUES ('session-1', 'project-1', 'active')",
            [],
        )
        .unwrap();
    db.connection()
        .execute(
            "INSERT INTO welding_progress (id, session_id, component_key, side, required_quantity, taken_quantity) VALUES ('progress-1', 'session-1', 'R1', 'top', 1, 1)",
            [],
        )
        .unwrap();
    db.connection()
        .execute(
            "INSERT INTO inventory_movements (id, part_id, session_id, movement_type, quantity, reason) VALUES ('movement-1', 'part-1', 'session-1', 'consume', -1, 'welding take')",
            [],
        )
        .unwrap();
    db
}

fn sidecar(path: &std::path::Path, suffix: &str) -> std::path::PathBuf {
    let mut name = path.file_name().unwrap().to_os_string();
    name.push(suffix);
    path.with_file_name(name)
}

fn recovery_paths(root: &std::path::Path) -> Vec<std::path::PathBuf> {
    root.read_dir()
        .unwrap()
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.extension().is_some_and(|extension| extension == "db")
                && path
                    .file_name()
                    .is_some_and(|name| name.to_string_lossy().starts_with(".partnest-recovery-"))
        })
        .collect()
}

fn count(path: &std::path::Path, table: &str) -> i64 {
    let connection = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
    connection
        .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
            row.get(0)
        })
        .unwrap()
}

#[test]
fn backup_includes_uncheckpointed_wal_rows_and_all_business_tables() {
    let root = tempdir().unwrap();
    let db_path = root.path().join("partnest.db");
    let db = seed_database(&db_path);
    assert_eq!(
        db.connection()
            .query_row("PRAGMA journal_mode", [], |row| row.get::<_, String>(0))
            .unwrap(),
        "wal"
    );
    let backup_path = create_backup(&db, root.path()).unwrap();
    drop(db);

    assert_eq!(count(&backup_path, "boxes"), 1);
    assert_eq!(count(&backup_path, "parts"), 1);
    assert_eq!(count(&backup_path, "inventory_movements"), 1);
    assert_eq!(count(&backup_path, "welding_progress"), 1);
    validate_backup(&backup_path).unwrap();
}

#[test]
fn backup_rejects_a_file_that_only_has_schema_migrations() {
    let root = tempdir().unwrap();
    let path = root.path().join("schema-only.db");
    let connection = Connection::open(&path).unwrap();
    connection
        .execute_batch(
            "CREATE TABLE schema_migrations (
                version INTEGER PRIMARY KEY,
                applied_at TEXT NOT NULL
            );
            INSERT INTO schema_migrations (version, applied_at) VALUES (2, 'now');",
        )
        .unwrap();
    drop(connection);

    assert!(matches!(
        validate_backup(&path),
        Err(BackupError::Invalid(_))
    ));
}

#[test]
fn backup_reads_rows_left_in_a_wal_without_source_checkpointing() {
    let root = tempdir().unwrap();
    let db_path = root.path().join("wal-source.db");
    let db = Database::open(&db_path).unwrap();
    db.connection()
        .execute_batch("PRAGMA wal_autocheckpoint = 0;")
        .unwrap();
    db.connection()
        .execute(
            "INSERT INTO boxes (name, rows, cols) VALUES ('WAL', 1, 1)",
            [],
        )
        .unwrap();
    let wal_box_id = db.connection().last_insert_rowid();
    db.connection()
        .execute(
            "INSERT INTO parts (id, name, quantity, box_id, slot) VALUES ('wal-part', 'WAL part', 7, ?1, 'A0')",
            [wal_box_id],
        )
        .unwrap();
    let wal_path = sidecar(&db_path, "-wal");
    assert!(
        wal_path.is_file(),
        "the source must have an uncheckpointed WAL"
    );

    let backup_path = create_backup(&db, root.path().join("backups")).unwrap();
    assert_eq!(count(&backup_path, "parts"), 1);
    assert_eq!(
        Connection::open(&backup_path)
            .unwrap()
            .query_row(
                "SELECT quantity FROM parts WHERE id = 'wal-part'",
                [],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
        7
    );
    assert!(!sidecar(&backup_path, "-wal").exists());
    assert!(!sidecar(&backup_path, "-shm").exists());
}

#[test]
fn corrupt_and_newer_schema_backups_are_rejected() {
    let root = tempdir().unwrap();
    let db_path = root.path().join("partnest.db");
    let db = seed_database(&db_path);
    let backup_path = create_backup(&db, root.path()).unwrap();
    drop(db);

    let corrupt_path = root.path().join("corrupt.db");
    fs::write(&corrupt_path, b"not sqlite").unwrap();
    assert!(validate_backup(&corrupt_path).is_err());

    let newer_path = root.path().join("newer.db");
    fs::copy(&backup_path, &newer_path).unwrap();
    let newer = Connection::open(&newer_path).unwrap();
    newer
        .execute(
            "INSERT INTO schema_migrations (version, applied_at) VALUES (99, 'future')",
            [],
        )
        .unwrap();
    drop(newer);
    assert!(validate_backup(&newer_path).is_err());
}

#[test]
fn backup_rotation_keeps_newest_ten_valid_backups() {
    let root = tempdir().unwrap();
    let db = seed_database(&root.path().join("partnest.db"));
    for _ in 0..12 {
        create_backup(&db, root.path()).unwrap();
    }
    let valid = fs::read_dir(root.path())
        .unwrap()
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|extension| extension == "db"))
        .filter(|path| {
            path.file_name()
                .is_some_and(|name| name.to_string_lossy().starts_with("partnest-"))
        })
        .filter(|path| validate_backup(path).is_ok())
        .count();
    assert_eq!(valid, 10);
}

#[test]
fn failed_restore_preserves_the_existing_database() {
    let root = tempdir().unwrap();
    let target_path = root.path().join("target.db");
    let mut target = seed_database(&target_path);
    target
        .connection()
        .execute("DELETE FROM inventory_movements", [])
        .unwrap();

    let invalid_path = root.path().join(format!("invalid-{}.db", new_id()));
    fs::write(&invalid_path, b"invalid backup").unwrap();
    assert!(restore_database_file(&mut target, &invalid_path).is_err());
    assert_eq!(count(&target_path, "inventory_movements"), 0);
}

#[test]
fn post_swap_open_failure_restores_the_original_file_and_connection() {
    let root = tempdir().unwrap();
    let source = seed_database(&root.path().join("source.db"));
    let backup_path = create_backup(&source, root.path().join("backups")).unwrap();
    drop(source);

    let target_path = root.path().join("target.db");
    let mut target = seed_database(&target_path);
    target
        .connection()
        .execute("DELETE FROM inventory_movements", [])
        .unwrap();
    let result = restore_database_file_with_injected_failure(&mut target, &backup_path);
    assert!(
        matches!(result, Err(BackupError::Invalid(message)) if message.contains("替换后打开失败"))
    );
    assert_eq!(count(&target_path, "inventory_movements"), 0);
    assert_eq!(
        target
            .connection()
            .query_row("SELECT COUNT(*) FROM inventory_movements", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert!(!root
        .path()
        .read_dir()
        .unwrap()
        .filter_map(Result::ok)
        .any(|entry| entry
            .file_name()
            .to_string_lossy()
            .starts_with(".partnest-previous-")));
    assert!(recovery_paths(root.path()).is_empty());
}

#[test]
fn post_open_failure_restores_the_original_file_and_shared_database_connection() {
    let root = tempdir().unwrap();
    let source = seed_database(&root.path().join("source.db"));
    let backup_path = create_backup(&source, root.path().join("backups")).unwrap();
    drop(source);

    let target_path = root.path().join("target.db");
    let mut target = seed_database(&target_path);
    target
        .connection()
        .execute("DELETE FROM inventory_movements", [])
        .unwrap();
    let result = restore_database_file_with_injected_post_open_failure(&mut target, &backup_path);
    assert!(matches!(
        result,
        Err(BackupError::Invalid(message)) if message.contains("打开后")
    ));
    assert_eq!(target.path(), target_path.as_path());
    assert_eq!(
        target
            .connection()
            .query_row("SELECT COUNT(*) FROM inventory_movements", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert!(!root
        .path()
        .read_dir()
        .unwrap()
        .filter_map(Result::ok)
        .any(|entry| entry
            .file_name()
            .to_string_lossy()
            .starts_with(".partnest-previous-")));
    assert!(recovery_paths(root.path()).is_empty());
}

#[test]
fn rollback_failure_keeps_a_preserved_recovery_database_queryable() {
    let root = tempdir().unwrap();
    let source = seed_database(&root.path().join("source.db"));
    let backup_path = create_backup(&source, root.path().join("backups")).unwrap();
    drop(source);

    let target_path = root.path().join("target.db");
    let mut target = seed_database(&target_path);
    target
        .connection()
        .execute("DELETE FROM inventory_movements", [])
        .unwrap();
    let result = restore_database_file_with_injected_rollback_failure(&mut target, &backup_path);
    let recovery_path = match result {
        Err(BackupError::FatalRecovery { recovery_path, .. }) => {
            std::path::PathBuf::from(recovery_path)
        }
        other => panic!("expected fatal recovery, got {other:?}"),
    };
    assert!(recovery_path.is_file());
    validate_backup(&recovery_path).unwrap();
    assert_eq!(target.path(), recovery_path.as_path());
    assert_eq!(count(&recovery_path, "inventory_movements"), 0);
    assert_eq!(
        target
            .connection()
            .query_row("SELECT COUNT(*) FROM inventory_movements", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert!(!sidecar(&target_path, "-wal").exists());
    assert!(!sidecar(&target_path, "-shm").exists());
    assert_eq!(recovery_paths(root.path()), vec![recovery_path]);
}

#[test]
fn live_reopen_failure_keeps_the_verified_recovery_snapshot_queryable() {
    let root = tempdir().unwrap();
    let source = seed_database(&root.path().join("source.db"));
    let backup_path = create_backup(&source, root.path().join("backups")).unwrap();
    drop(source);

    let target_path = root.path().join("target.db");
    let mut target = seed_database(&target_path);
    target
        .connection()
        .execute("DELETE FROM inventory_movements", [])
        .unwrap();
    let result = restore_database_file_with_injected_live_reopen_failure(&mut target, &backup_path);
    let recovery_path = match result {
        Err(BackupError::FatalRecovery { recovery_path, .. }) => {
            std::path::PathBuf::from(recovery_path)
        }
        other => panic!("expected fatal recovery, got {other:?}"),
    };
    assert!(recovery_path.is_file());
    validate_backup(&recovery_path).unwrap();
    assert_eq!(target.path(), recovery_path.as_path());
    assert_eq!(count(&recovery_path, "inventory_movements"), 0);
    assert_eq!(
        target
            .connection()
            .query_row("SELECT COUNT(*) FROM inventory_movements", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert_eq!(recovery_paths(root.path()), vec![recovery_path]);
}

#[test]
fn restore_replaces_live_database_from_a_valid_backup() {
    let root = tempdir().unwrap();
    let source_path = root.path().join("source.db");
    let source = seed_database(&source_path);
    let backup_path = create_backup(&source, root.path().join("backups")).unwrap();
    drop(source);

    let target_path = root.path().join("target.db");
    let mut target = seed_database(&target_path);
    target
        .connection()
        .execute("DELETE FROM inventory_movements", [])
        .unwrap();
    restore_database_file(&mut target, &backup_path).unwrap();
    assert_eq!(count(&target_path, "inventory_movements"), 1);
    assert_eq!(count(&target_path, "welding_progress"), 1);
    assert!(recovery_paths(root.path()).is_empty());
}

#[test]
fn successful_restore_removes_previous_database_sidecars() {
    let root = tempdir().unwrap();
    let source = seed_database(&root.path().join("source.db"));
    let backup_path = create_backup(&source, root.path().join("backups")).unwrap();
    drop(source);

    let target_path = root.path().join("target.db");
    let mut target = seed_database(&target_path);
    let live_wal = sidecar(&target_path, "-wal");
    let live_shm = sidecar(&target_path, "-shm");
    assert!(live_wal.is_file());
    assert!(live_shm.is_file());
    restore_database_file(&mut target, &backup_path).unwrap();

    assert!(!root
        .path()
        .read_dir()
        .unwrap()
        .filter_map(Result::ok)
        .any(|entry| entry
            .file_name()
            .to_string_lossy()
            .starts_with(".partnest-previous-")));
}

#[test]
fn startup_backup_is_reused_until_it_is_at_least_24_hours_old() {
    let root = tempdir().unwrap();
    let db = seed_database(&root.path().join("partnest.db"));
    let first = maybe_create_startup_backup(&db, root.path())
        .unwrap()
        .unwrap();
    assert!(maybe_create_startup_backup(&db, root.path())
        .unwrap()
        .is_none());

    let file = File::options().write(true).open(&first).unwrap();
    file.set_modified(SystemTime::now() - STARTUP_BACKUP_AGE - Duration::from_secs(1))
        .unwrap();
    assert!(maybe_create_startup_backup(&db, root.path())
        .unwrap()
        .is_some());
    let valid_count = fs::read_dir(backup_dir(root.path()))
        .unwrap()
        .filter_map(Result::ok)
        .filter(|entry| {
            entry
                .path()
                .extension()
                .is_some_and(|extension| extension == "db")
                && entry.file_name().to_string_lossy().starts_with("partnest-")
        })
        .count();
    assert_eq!(
        valid_count, 2,
        "an old valid backup should trigger exactly one new startup backup"
    );
}

#[test]
fn rotation_keeps_invalid_and_unrelated_files() {
    let root = tempdir().unwrap();
    let backup_directory = root.path().join("backups");
    fs::create_dir_all(&backup_directory).unwrap();
    let invalid = backup_directory.join("partnest-invalid.db");
    let unrelated = backup_directory.join("notes.db");
    fs::write(&invalid, b"not a backup").unwrap();
    fs::write(&unrelated, b"leave me alone").unwrap();

    let db = seed_database(&root.path().join("partnest.db"));
    for _ in 0..12 {
        create_backup(&db, &backup_directory).unwrap();
    }
    assert!(invalid.is_file());
    assert_eq!(fs::read(&unrelated).unwrap(), b"leave me alone");
}

#[test]
fn restore_rejects_a_hard_link_to_the_live_database() {
    let root = tempdir().unwrap();
    let target_path = root.path().join("target.db");
    let mut target = seed_database(&target_path);
    let hard_link = root.path().join("target-alias.db");
    fs::hard_link(&target_path, &hard_link).unwrap();

    assert!(matches!(
        restore_database_file(&mut target, &hard_link),
        Err(BackupError::Invalid(message)) if message.contains("硬链接")
    ));
}

#[test]
fn backup_restores_archived_parts_and_reused_lcsc_without_resurrecting_stock() {
    use partnest_desktop_lib::commands::movements::list_movements_service;
    use partnest_desktop_lib::commands::parts::{delete_part_service, list_parts_service};
    let root = tempdir().unwrap();
    let mut db = seed_database(&root.path().join("partnest.db"));
    db.connection()
        .execute(
            "UPDATE parts SET lcsc_code = 'C123' WHERE id = 'part-1'",
            [],
        )
        .unwrap();
    delete_part_service(&db, "part-1").unwrap();
    db.connection().execute("INSERT INTO parts (id, name, lcsc_code, quantity) VALUES ('part-2', 'Second', 'C123', 0)", []).unwrap();
    delete_part_service(&db, "part-2").unwrap();
    db.connection().execute("INSERT INTO parts (id, name, lcsc_code, quantity) VALUES ('part-3', 'Current', 'C123', 0)", []).unwrap();
    let expected_history = list_movements_service(&db).unwrap();
    let backup = create_backup(&db, root.path()).unwrap();
    validate_backup(&backup).unwrap();
    delete_part_service(&db, "part-3").unwrap();
    restore_database_file(&mut db, &backup).unwrap();
    let live = list_parts_service(&db, None).unwrap();
    assert_eq!(live.len(), 1);
    assert_eq!(live[0].id, "part-3");
    assert_eq!(list_movements_service(&db).unwrap(), expected_history);
    assert_eq!(
        db.connection()
            .query_row(
                "SELECT COUNT(*) FROM parts WHERE deleted_at IS NOT NULL",
                [],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
        2
    );
    assert_eq!(
        db.connection()
            .query_row(
                "SELECT quantity FROM parts WHERE id = 'part-1'",
                [],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
        4
    );
}

#[test]
fn backup_validation_rejects_missing_archive_column_or_unscoped_lcsc_index() {
    let root = tempdir().unwrap();
    let db = seed_database(&root.path().join("partnest.db"));
    let backup = create_backup(&db, root.path()).unwrap();
    let conn = Connection::open(&backup).unwrap();
    conn.execute_batch(
        "DROP INDEX parts_lcsc_code_unique;
        CREATE UNIQUE INDEX parts_lcsc_code_unique ON parts (lcsc_code COLLATE NOCASE)
        WHERE lcsc_code IS NOT NULL AND trim(lcsc_code) <> '';",
    )
    .unwrap();
    drop(conn);
    assert!(matches!(
        validate_backup(&backup),
        Err(BackupError::Invalid(_))
    ));
    let conn = Connection::open(&backup).unwrap();
    conn.execute_batch("ALTER TABLE parts DROP COLUMN deleted_at;")
        .unwrap();
    drop(conn);
    assert!(matches!(
        validate_backup(&backup),
        Err(BackupError::Invalid(_))
    ));
}
