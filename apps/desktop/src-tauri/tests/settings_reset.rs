use partnest_desktop_lib::{
    backup, bom::cache::InteractiveBomRuntime, commands::settings::reset_data_service, db::Database,
};

fn seed(db: &Database) {
    db.connection().execute_batch(
        "INSERT INTO boxes (id, name, rows, cols) VALUES (1, 'Bench', 2, 2);
         INSERT INTO parts (id, name, quantity, box_id, slot) VALUES ('p', 'Resistor', 2, 1, 'A0');
         INSERT INTO projects (id, name, original_name, sha256, cache_name) VALUES ('p', 'Board', 'b.html', 'hash', 'hash.html');
         INSERT INTO welding_sessions (id, project_id, status) VALUES ('s', 'p', 'active');
         INSERT INTO welding_progress (id, session_id, component_key, side, part_id, required_quantity, taken_quantity) VALUES ('w', 's', 'R', 'top', 'p', 1, 0);
         INSERT INTO inventory_movements (id, part_id, session_id, movement_type, quantity, reason) VALUES ('take', 'p', 's', 'consume', -1, 'take');
         INSERT INTO inventory_movements (id, part_id, session_id, movement_type, quantity, reason, reverses_movement_id) VALUES ('undo', 'p', 's', 'reverse', 1, 'undo', 'take');"
    ).unwrap();
}

#[test]
fn reset_clears_business_data_and_preserves_restorable_backup() {
    let root = tempfile::tempdir().unwrap();
    let mut db = Database::open(root.path().join("live.db")).unwrap();
    seed(&db);
    let runtime = InteractiveBomRuntime::new(root.path().join("cache"));
    let path = reset_data_service(&db, &runtime, &root.path().join("backups")).unwrap();
    for table in [
        "parts",
        "boxes",
        "inventory_movements",
        "projects",
        "welding_sessions",
        "welding_progress",
        "lcsc_cache",
    ] {
        assert_eq!(
            db.connection()
                .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r
                    .get::<_, i64>(0))
                .unwrap(),
            0
        );
    }
    backup::restore_database_file(&mut db, path).unwrap();
    assert_eq!(
        db.connection()
            .query_row("SELECT quantity FROM parts WHERE id = 'p'", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        2
    );
    assert_eq!(
        db.connection()
            .query_row("SELECT COUNT(*) FROM inventory_movements", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        2
    );
}

#[test]
fn failed_backup_or_delete_keeps_data() {
    let root = tempfile::tempdir().unwrap();
    let db = Database::open(root.path().join("live.db")).unwrap();
    seed(&db);
    let runtime = InteractiveBomRuntime::new(root.path().join("cache"));
    let blocked = root.path().join("blocked");
    std::fs::write(&blocked, "not a directory").unwrap();
    assert!(reset_data_service(&db, &runtime, &blocked).is_err());
    db.connection().execute_batch("CREATE TRIGGER refuse_reset BEFORE DELETE ON boxes BEGIN SELECT RAISE(ABORT, 'blocked'); END;").unwrap();
    assert!(reset_data_service(&db, &runtime, &root.path().join("backups")).is_err());
    assert_eq!(
        db.connection()
            .query_row("SELECT COUNT(*) FROM inventory_movements", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        2
    );
    assert_eq!(
        db.connection()
            .query_row("SELECT COUNT(*) FROM parts", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        1
    );
}
