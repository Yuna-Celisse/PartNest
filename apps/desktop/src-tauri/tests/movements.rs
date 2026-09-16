use partnest_desktop_lib::commands::boxes::{create_box_service, BoxInput};
use partnest_desktop_lib::commands::movements::list_movements_service;
use partnest_desktop_lib::commands::parts::{create_part_service, delete_part_service, PartInput};
use partnest_desktop_lib::db::{new_id, Database};
use rusqlite::params;
use tempfile::tempdir;

fn part_input(box_id: impl ToString) -> PartInput {
    PartInput {
        name: "10k resistor".into(),
        category: "resistor".into(),
        package: "0603".into(),
        manufacturer: "Acme".into(),
        mpn: "R-10K".into(),
        lcsc_code: "C123".into(),
        quantity: 13,
        box_id: Some(box_id.to_string().parse().expect("integer box id")),
        slot: Some("A0".into()),
        note: "test".into(),
    }
}

#[test]
fn movements_are_newest_first_with_stock_before_after_and_reversal_visibility() {
    let root = tempdir().unwrap();
    let db = Database::open(root.path().join("partnest.db")).unwrap();
    let box_record = create_box_service(
        &db,
        BoxInput {
            name: "Bench".into(),
            rows: 2,
            cols: 2,
        },
    )
    .unwrap();
    let part = create_part_service(&db, part_input(box_record.id)).unwrap();
    // This fixture supplies its own historical movement timeline.
    db.connection()
        .execute(
            "DELETE FROM inventory_movements WHERE part_id = ?1",
            params![part.id],
        )
        .unwrap();
    db.connection()
        .execute(
            "INSERT INTO bom_files (id, original_name, display_name, sha256, cache_name) VALUES ('bom-1', 'board.html', 'Board note', 'hash', 'hash.html')",
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
            "INSERT INTO welding_progress (id, session_id, component_key, side, part_id, required_quantity, taken_quantity) VALUES ('progress-1', 'session-1', 'R1', 'top', ?1, 2, 2)",
            params![part.id],
        )
        .unwrap();
    db.connection().execute("INSERT INTO inventory_movements (id, part_id, session_id, movement_type, quantity, reason, component_key, side, created_at) VALUES ('m-old', ?1, 'session-1', 'adjust', 5, 'restock', 'R1', 'top', '2026-01-01T00:00:00.000Z')", params![part.id]).unwrap();
    db.connection().execute("INSERT INTO inventory_movements (id, part_id, session_id, movement_type, quantity, reason, component_key, side, created_at) VALUES ('m-take', ?1, 'session-1', 'consume', -2, 'welding take', 'R1', 'top', '2026-01-02T00:00:00.000Z')", params![part.id]).unwrap();

    let movements = list_movements_service(&db).unwrap();
    assert_eq!(movements.len(), 2);
    assert_eq!(movements[0].id, "m-take");
    assert_eq!(movements[0].before_quantity, Some(15));
    assert_eq!(movements[0].after_quantity, Some(13));
    assert_eq!(movements[0].part_id.as_deref(), Some(part.id.as_str()));
    assert_eq!(movements[0].part_name.as_deref(), Some("10k resistor"));
    assert_eq!(movements[0].component_key.as_deref(), Some("R1"));
    assert_eq!(movements[0].movement_type, "consume");
    assert_eq!(movements[0].delta, -2);
    assert_eq!(movements[0].quantity, -2);
    assert_eq!(movements[0].reason, "welding take");
    assert_eq!(movements[0].bom_display_name.as_deref(), Some("Board note"));
    assert_eq!(movements[0].session_id.as_deref(), Some("session-1"));
    assert_eq!(movements[0].side.as_deref(), Some("top"));
    assert_eq!(movements[0].created_at, "2026-01-02T00:00:00.000Z");
    assert!(movements[0].reverses_movement_id.is_none());
    assert!(movements[0].reversible);

    db.connection().execute("INSERT INTO inventory_movements (id, part_id, session_id, movement_type, quantity, reason, reverses_movement_id, component_key, created_at) VALUES ('m-reverse', ?1, 'session-1', 'reverse', 2, 'welding take reversal', 'm-take', 'R1', '2026-01-03T00:00:00.000Z')", params![part.id]).unwrap();
    let movements = list_movements_service(&db).unwrap();
    assert_eq!(movements[1].id, "m-take");
    assert!(!movements[1].reversible);

    db.connection()
        .execute(
            "INSERT INTO welding_sessions (id, bom_file_id, status) VALUES ('session-2', 'bom-1', 'completed')",
            [],
        )
        .unwrap();
    db.connection().execute("INSERT INTO inventory_movements (id, part_id, session_id, movement_type, quantity, reason, component_key, side, created_at) VALUES ('m-inactive', ?1, 'session-2', 'consume', -1, 'legacy take', 'R1', 'top', '2026-01-04T00:00:00.000Z')", params![part.id]).unwrap();
    db.connection().execute("INSERT INTO inventory_movements (id, part_id, session_id, movement_type, quantity, reason, created_at) VALUES ('m-malformed', ?1, 'session-1', 'consume', -1, 'legacy malformed', '2026-01-05T00:00:00.000Z')", params![part.id]).unwrap();

    let movements = list_movements_service(&db).unwrap();
    assert!(
        !movements
            .iter()
            .find(|movement| movement.id == "m-inactive")
            .unwrap()
            .reversible
    );
    assert!(
        !movements
            .iter()
            .find(|movement| movement.id == "m-malformed")
            .unwrap()
            .reversible
    );
}

#[test]
fn movement_ids_are_not_required_to_be_uuid_values() {
    let root = tempdir().unwrap();
    let db = Database::open(root.path().join(format!("{}.db", new_id()))).unwrap();
    assert!(list_movements_service(&db).unwrap().is_empty());
}

#[test]
fn legacy_movement_reconstruction_rewinds_past_modern_audit_rows() {
    let root = tempdir().unwrap();
    let db = Database::open(root.path().join("partnest.db")).unwrap();
    let box_record = create_box_service(
        &db,
        BoxInput {
            name: "Bench".into(),
            rows: 2,
            cols: 2,
        },
    )
    .unwrap();
    let part = create_part_service(&db, part_input(box_record.id)).unwrap();
    db.connection()
        .execute(
            "DELETE FROM inventory_movements WHERE part_id = ?1",
            params![part.id],
        )
        .unwrap();
    db.connection().execute(
        "INSERT INTO inventory_movements (id, part_id, movement_type, quantity, reason, created_at) VALUES ('legacy-in', ?1, 'in', 5, 'legacy stock', '2026-01-01T00:00:00.000Z')",
        params![part.id],
    ).unwrap();
    db.connection().execute(
        "INSERT INTO inventory_movements (id, part_id, movement_type, quantity, reason, before_quantity, after_quantity, movement_sequence, created_at) VALUES ('modern-consume', ?1, 'consume', -2, 'welding take', 15, 13, 2, '2026-01-02T00:00:00.000Z')",
        params![part.id],
    ).unwrap();

    let movements = list_movements_service(&db).unwrap();
    let legacy = movements
        .iter()
        .find(|item| item.id == "legacy-in")
        .unwrap();
    assert_eq!(
        (legacy.before_quantity, legacy.after_quantity),
        (Some(10), Some(15))
    );
}

#[test]
fn archived_parts_keep_legacy_and_modern_movement_history_but_cannot_be_reversed() {
    let root = tempdir().unwrap();
    let db = Database::open(root.path().join("partnest.db")).unwrap();
    let b = create_box_service(
        &db,
        BoxInput {
            name: "Bench".into(),
            rows: 2,
            cols: 2,
        },
    )
    .unwrap();
    let part = create_part_service(&db, part_input(b.id)).unwrap();
    db.connection()
        .execute(
            "DELETE FROM inventory_movements WHERE part_id = ?1",
            [&part.id],
        )
        .unwrap();
    db.connection().execute_batch("INSERT INTO bom_files (id, original_name, display_name, sha256, cache_name) VALUES ('b', 'b.html', 'Board', 'h', 'h.html');
        INSERT INTO welding_sessions (id, bom_file_id, status) VALUES ('s', 'b', 'active');").unwrap();
    db.connection().execute("INSERT INTO welding_progress (id, session_id, component_key, side, part_id, required_quantity, taken_quantity) VALUES ('p', 's', 'C123', 'top', ?1, 3, 2)", [&part.id]).unwrap();
    db.connection().execute("INSERT INTO inventory_movements (id, part_id, movement_type, quantity, reason, created_at) VALUES ('old', ?1, 'in', 5, 'legacy stock', '2026-01-01')", [&part.id]).unwrap();
    db.connection().execute("INSERT INTO inventory_movements (id, part_id, session_id, component_key, side, movement_type, quantity, reason, created_at) VALUES ('take', ?1, 's', 'C123', 'top', 'consume', -2, 'take', '2026-01-02')", [&part.id]).unwrap();
    let before = list_movements_service(&db).unwrap();
    assert!(before[0].reversible);
    delete_part_service(&db, &part.id).unwrap();
    let after = list_movements_service(&db).unwrap();
    assert_eq!(after.len(), before.len());
    for (mut old, archived) in before.into_iter().zip(after) {
        old.reversible = false;
        assert_eq!(old, archived);
        assert_eq!(archived.part_name.as_deref(), Some("10k resistor"));
    }
    // A modern audit row must still rewind the same baseline for older rows.
    db.connection().execute("UPDATE inventory_movements SET before_quantity = 15, after_quantity = 13 WHERE id = 'take'", []).unwrap();
    let mixed = list_movements_service(&db).unwrap();
    assert_eq!(
        (mixed[1].before_quantity, mixed[1].after_quantity),
        (Some(10), Some(15))
    );
}
