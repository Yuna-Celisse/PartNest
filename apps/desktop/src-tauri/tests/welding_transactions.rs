use partnest_desktop_lib::bom::cache::InteractiveBomRuntime;
use partnest_desktop_lib::bom::types::BomSide;
use partnest_desktop_lib::commands::boxes::{create_box_service, BoxInput};
use partnest_desktop_lib::commands::movements::list_movements_service;
use partnest_desktop_lib::commands::parts::{create_part_service, delete_part_service, PartInput};
use partnest_desktop_lib::commands::welding::{
    confirm_take_authorized_service, confirm_take_service, end_welding_session_service,
    get_welding_progress_service, reverse_take_service, ConfirmTakeInput,
};
use partnest_desktop_lib::commands::CommandError;
use partnest_desktop_lib::db::{new_id, Database, Migration};

fn database() -> Database {
    let path = std::env::temp_dir().join(format!("partnest-welding-test-{}", new_id()));
    Database::open(path).expect("open test database")
}

fn fixture() -> (Database, String, String) {
    let db = database();
    let box_record = create_box_service(
        &db,
        BoxInput {
            name: "Test box".into(),
            rows: 2,
            cols: 2,
        },
    )
    .unwrap();
    let part = create_part_service(
        &db,
        PartInput {
            name: "10k resistor".into(),
            category: "resistor".into(),
            package: "0603".into(),
            manufacturer: "Acme".into(),
            mpn: "R-10K".into(),
            lcsc_code: "C123".into(),
            quantity: 10,
            box_id: Some(box_record.id),
            slot: Some("A0".into()),
            note: String::new(),
        },
    )
    .unwrap();
    let bom_id = new_id();
    let session_id = new_id();
    db.connection()
        .execute(
            "INSERT INTO projects (id, name, original_name, sha256, cache_name) VALUES (?1, 'BOM', 'bom.html', 'hash', 'hash.html')",
            [&bom_id],
        )
        .unwrap();
    db.connection()
        .execute(
            "INSERT INTO welding_sessions (id, project_id, status) VALUES (?1, ?2, 'active')",
            rusqlite::params![session_id, bom_id],
        )
        .unwrap();
    (db, session_id, part.id)
}

fn take(session_id: &str, part_id: &str, side: BomSide, quantity: i64) -> ConfirmTakeInput {
    take_designating(session_id, part_id, side, quantity, &["R1"])
}

fn take_designating(
    session_id: &str,
    part_id: &str,
    side: BomSide,
    quantity: i64,
    designators: &[&str],
) -> ConfirmTakeInput {
    ConfirmTakeInput {
        session_id: session_id.into(),
        component_key: "C123".into(),
        side,
        designators: designators
            .iter()
            .map(|value| (*value).to_owned())
            .collect(),
        bom_quantity: 3,
        take_quantity: quantity,
        part_id: part_id.into(),
        expected_part_version: 1,
    }
}

#[test]
fn top_and_bottom_takes_are_independent_and_additional_take_is_a_new_movement() {
    let (db, session_id, part_id) = fixture();
    let top = confirm_take_service(&db, take(&session_id, &part_id, BomSide::Top, 2)).unwrap();
    let mut bottom_input = take_designating(&session_id, &part_id, BomSide::Bottom, 1, &["R2"]);
    bottom_input.expected_part_version = top.part_version;
    let bottom = confirm_take_service(&db, bottom_input).unwrap();

    let mut additional = take_designating(&session_id, &part_id, BomSide::Top, 1, &["R3"]);
    additional.expected_part_version = bottom.part_version;
    let extra = confirm_take_service(&db, additional).unwrap();
    assert_ne!(top.movement_id, extra.movement_id);
    assert_eq!(extra.consumed_quantity, 3);

    let progress = get_welding_progress_service(&db, &session_id).unwrap();
    assert_eq!(progress.len(), 2);
    assert_eq!(
        progress
            .iter()
            .find(|p| p.side == BomSide::Top)
            .unwrap()
            .consumed_quantity,
        3
    );
    assert_eq!(
        progress
            .iter()
            .find(|p| p.side == BomSide::Bottom)
            .unwrap()
            .consumed_quantity,
        1
    );
    assert_eq!(
        db.connection()
            .query_row(
                "SELECT quantity FROM parts WHERE id = ?1",
                [&part_id],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
        6
    );
    let audit = db
        .connection()
        .prepare(
            "SELECT quantity, before_quantity, after_quantity, bom_quantity, confirmation_designators, movement_sequence FROM inventory_movements WHERE id = ?1",
        )
        .unwrap()
        .query_row([top.movement_id], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, i64>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, i64>(5)?,
            ))
        })
        .unwrap();
    assert_eq!(audit.0, -2);
    assert_eq!((audit.1, audit.2), (10, 8));
    assert_eq!(audit.3, 3);
    assert_eq!(
        serde_json::from_str::<Vec<String>>(&audit.4).unwrap(),
        ["R1"]
    );
    assert!(audit.5 > 0);
}

#[test]
fn additional_take_keeps_the_bom_requirement_and_records_overconsumption() {
    let (db, session_id, part_id) = fixture();
    let first = confirm_take_service(&db, take(&session_id, &part_id, BomSide::Top, 2)).unwrap();
    let mut extra = take_designating(&session_id, &part_id, BomSide::Top, 2, &["R2"]);
    extra.expected_part_version = first.part_version;

    let result = confirm_take_service(&db, extra).unwrap();

    assert_eq!(result.required_quantity, 3);
    assert_eq!(result.consumed_quantity, 4);
    assert_eq!(result.status, "taken");
    let progress = get_welding_progress_service(&db, &session_id).unwrap();
    let top = progress
        .iter()
        .find(|item| item.side == BomSide::Top)
        .unwrap();
    assert_eq!((top.required_quantity, top.consumed_quantity), (3, 4));
}

#[test]
fn the_same_designator_is_only_charged_once_until_its_take_is_reversed() {
    let (db, session_id, part_id) = fixture();
    let first = confirm_take_service(&db, take(&session_id, &part_id, BomSide::Top, 1)).unwrap();

    let mut repeat = take(&session_id, &part_id, BomSide::Top, 1);
    repeat.expected_part_version = first.part_version;
    let error =
        confirm_take_service(&db, repeat).expect_err("same designator must not be charged twice");
    assert!(matches!(error, CommandError::Validation(message) if message.contains("均已取用")));

    // The rejected retry must not have left any trace behind.
    assert_eq!(
        db.connection()
            .query_row(
                "SELECT COUNT(*) FROM inventory_movements WHERE session_id = ?1",
                [&session_id],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
        1
    );

    reverse_take_service(&db, &first.movement_id).unwrap();
    let mut retaken = take(&session_id, &part_id, BomSide::Top, 1);
    retaken.expected_part_version = first.part_version + 1;
    assert!(confirm_take_service(&db, retaken).is_ok());
}

#[test]
fn a_wider_selection_raises_the_requirement_but_never_lowers_it() {
    let (db, session_id, part_id) = fixture();
    let narrow_input = take_designating(&session_id, &part_id, BomSide::Bottom, 1, &["R9"]);
    let narrow = confirm_take_service(&db, narrow_input).unwrap();
    assert_eq!(narrow.required_quantity, 3);

    let mut wider = take_designating(&session_id, &part_id, BomSide::Bottom, 1, &["R8"]);
    wider.expected_part_version = narrow.part_version;
    wider.bom_quantity = 7;
    let wider = confirm_take_service(&db, wider).unwrap();
    assert_eq!(wider.required_quantity, 7);

    let mut narrower = take_designating(&session_id, &part_id, BomSide::Bottom, 1, &["R7"]);
    narrower.expected_part_version = wider.part_version;
    narrower.bom_quantity = 2;
    let narrower = confirm_take_service(&db, narrower).unwrap();
    assert_eq!(
        narrower.required_quantity, 7,
        "a later narrow selection must not understate the requirement"
    );
}

#[test]
fn insufficient_stock_and_stale_version_are_atomic() {
    let (db, session_id, part_id) = fixture();
    let mut input = take(&session_id, &part_id, BomSide::Top, 11);
    input.bom_quantity = 11;
    assert!(confirm_take_service(&db, input).is_err());
    assert_eq!(
        db.connection()
            .query_row(
                "SELECT quantity, version FROM parts WHERE id = ?1",
                [&part_id],
                |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?))
            )
            .unwrap(),
        (10, 1)
    );
    let first = confirm_take_service(&db, take(&session_id, &part_id, BomSide::Top, 1)).unwrap();
    let stale = confirm_take_service(&db, take(&session_id, &part_id, BomSide::Bottom, 1));
    assert!(stale.is_err());
    assert_eq!(
        db.connection()
            .query_row(
                "SELECT COUNT(*) FROM inventory_movements WHERE session_id = ?1",
                [&session_id],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
        1
    );
    assert_eq!(first.part_version, 2);
}

#[test]
fn reversing_bottom_take_restores_stock_and_duplicate_reversal_is_rejected() {
    let (db, session_id, part_id) = fixture();
    let mut input = take(&session_id, &part_id, BomSide::Bottom, 1);
    input.expected_part_version = 1;
    let taken = confirm_take_service(&db, input).unwrap();
    let reversed = reverse_take_service(&db, &taken.movement_id).unwrap();
    assert_eq!(reversed.side, BomSide::Bottom);
    assert_eq!(reversed.consumed_quantity, 0);
    assert_eq!(reversed.status, "pending");
    assert_eq!(
        db.connection()
            .query_row(
                "SELECT quantity FROM parts WHERE id = ?1",
                [&part_id],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
        10
    );
    assert!(reverse_take_service(&db, &taken.movement_id).is_err());
    assert_eq!(
        db.connection()
            .query_row(
                "SELECT COUNT(*) FROM inventory_movements WHERE session_id = ?1",
                [&session_id],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
        2
    );
}

#[test]
fn complete_sequence_keeps_side_status_and_quantities_independent() {
    let (db, session_id, part_id) = fixture();
    let mut top_input = take(&session_id, &part_id, BomSide::Top, 2);
    top_input.expected_part_version = 1;
    let top = confirm_take_service(&db, top_input).unwrap();
    assert_eq!(top.status, "partial");

    let mut bottom_input = take_designating(&session_id, &part_id, BomSide::Bottom, 1, &["R2"]);
    bottom_input.expected_part_version = top.part_version;
    let bottom = confirm_take_service(&db, bottom_input).unwrap();
    assert_eq!(bottom.status, "partial");

    let mut additional = take_designating(&session_id, &part_id, BomSide::Top, 1, &["R3"]);
    additional.expected_part_version = bottom.part_version;
    let top_done = confirm_take_service(&db, additional).unwrap();
    assert_eq!(top_done.status, "taken");

    let bottom_reversed = reverse_take_service(&db, &bottom.movement_id).unwrap();
    assert_eq!(bottom_reversed.status, "pending");
    let progress = get_welding_progress_service(&db, &session_id).unwrap();
    let top_progress = progress.iter().find(|p| p.side == BomSide::Top).unwrap();
    let bottom_progress = progress.iter().find(|p| p.side == BomSide::Bottom).unwrap();
    assert_eq!(
        (top_progress.consumed_quantity, top_progress.status.as_str()),
        (3, "taken")
    );
    assert_eq!(
        (
            bottom_progress.consumed_quantity,
            bottom_progress.status.as_str()
        ),
        (0, "pending")
    );
    assert!(reverse_take_service(&db, &bottom.movement_id).is_err());
    assert_eq!(
        db.connection()
            .query_row(
                "SELECT COUNT(*) FROM inventory_movements WHERE session_id = ?1",
                [&session_id],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
        4
    );
}

#[test]
fn authorized_command_accepts_designator_subsets_and_uses_authoritative_quantity() {
    let root = tempfile::tempdir().unwrap();
    let db = Database::open(root.path().join("partnest.db")).unwrap();
    let runtime = InteractiveBomRuntime::new(root.path().join("cache"));
    let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../fixtures/bom/interactive-minimal.html");
    let imported = runtime
        .import_project(&db, &source, None, None, None)
        .unwrap();
    let session = runtime
        .open_project_welding(&db, &imported.project_id)
        .unwrap();
    let box_record = create_box_service(
        &db,
        BoxInput {
            name: "Test box".into(),
            rows: 2,
            cols: 2,
        },
    )
    .unwrap();
    let part = create_part_service(
        &db,
        PartInput {
            name: "10k resistor".into(),
            category: "resistor".into(),
            package: "0603".into(),
            manufacturer: "Acme".into(),
            mpn: "ACME-10K".into(),
            lcsc_code: "C100".into(),
            quantity: 10,
            box_id: Some(box_record.id),
            slot: Some("A0".into()),
            note: String::new(),
        },
    )
    .unwrap();
    let mut valid = ConfirmTakeInput {
        session_id: session.session_id.clone(),
        component_key: "C100".into(),
        side: BomSide::Top,
        designators: vec!["R1".into()],
        bom_quantity: 999,
        take_quantity: 1,
        part_id: part.id.clone(),
        expected_part_version: 1,
    };
    let taken = confirm_take_authorized_service(&db, &runtime, valid.clone()).unwrap();
    assert_eq!(taken.required_quantity, 1);
    assert_eq!(
        db.connection()
            .query_row(
                "SELECT bom_quantity FROM inventory_movements WHERE id = ?1",
                [&taken.movement_id],
                |row| row.get::<_, i64>(0),
            )
            .unwrap(),
        1
    );

    for designators in [
        vec!["R1".into(), "R2".into(), "R9".into()],
        vec!["R1".into(), "R1".into()],
        vec!["R1".into(), "R3".into()],
    ] {
        valid.designators = designators;
        valid.expected_part_version = 2;
        assert!(confirm_take_authorized_service(&db, &runtime, valid.clone()).is_err());
    }
    db.connection()
        .execute(
            "UPDATE welding_sessions SET status = 'completed' WHERE id = ?1",
            [&session.session_id],
        )
        .unwrap();
    valid.designators = vec!["R1".into(), "R2".into()];
    assert!(confirm_take_authorized_service(&db, &runtime, valid).is_err());
    assert_eq!(
        db.connection()
            .query_row(
                "SELECT quantity, version FROM parts WHERE id = ?1",
                [&part.id],
                |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?))
            )
            .unwrap(),
        (9, 2)
    );
    assert_eq!(
        db.connection()
            .query_row(
                "SELECT COUNT(*) FROM inventory_movements WHERE session_id = ?1",
                [&session.session_id],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
        1
    );
}

#[test]
fn later_movement_failure_rolls_back_stock_version_and_progress() {
    let (db, session_id, part_id) = fixture();
    db.connection().execute_batch("CREATE TRIGGER fail_welding_movement BEFORE INSERT ON inventory_movements WHEN NEW.reason = 'welding take' BEGIN SELECT RAISE(ABORT, 'injected movement failure'); END").unwrap();
    let result = confirm_take_service(&db, take(&session_id, &part_id, BomSide::Top, 1));
    assert!(result.is_err());
    assert_eq!(
        db.connection()
            .query_row(
                "SELECT quantity, version FROM parts WHERE id = ?1",
                [&part_id],
                |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?))
            )
            .unwrap(),
        (10, 1)
    );
    assert_eq!(
        db.connection()
            .query_row(
                "SELECT COUNT(*) FROM inventory_movements WHERE session_id = ?1",
                [&session_id],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
        0
    );
    assert_eq!(
        db.connection()
            .query_row(
                "SELECT COUNT(*) FROM welding_progress WHERE session_id = ?1",
                [&session_id],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
        0
    );
}

#[test]
fn migration_0002_upgrades_existing_welding_databases() {
    let path = std::env::temp_dir().join(format!("partnest-welding-migration-{}", new_id()));
    let initial = include_str!("../migrations/0001_initial.sql");
    Database::open_with_migrations(
        &path,
        &[Migration {
            version: 1,
            sql: initial,
        }],
    )
    .unwrap();
    let upgraded = Database::open(&path).unwrap();
    let columns = upgraded
        .connection()
        .prepare("PRAGMA table_info(inventory_movements)")
        .unwrap()
        .query_map([], |row| row.get::<_, String>(1))
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap();
    assert!(columns.iter().any(|column| column == "component_key"));
    assert!(columns.iter().any(|column| column == "side"));
    assert!(columns.iter().any(|column| column == "before_quantity"));
    assert!(columns.iter().any(|column| column == "movement_sequence"));
}

#[test]
fn deleting_active_stock_preserves_progress_and_allows_replacement_for_fresh_placements() {
    let (db, session, old_part) = fixture();
    let first = confirm_take_service(&db, take(&session, &old_part, BomSide::Top, 1)).unwrap();
    let before = get_welding_progress_service(&db, &session).unwrap();
    delete_part_service(&db, &old_part).unwrap();
    let archived_progress = get_welding_progress_service(&db, &session).unwrap();
    assert_eq!(archived_progress[0].part_id, None);
    assert_eq!(
        archived_progress[0].confirmed_designators,
        before[0].confirmed_designators
    );
    assert_eq!(archived_progress[0].consumed_quantity, 1);
    assert_eq!(archived_progress[0].required_quantity, 3);
    assert!(matches!(
        confirm_take_service(&db, take(&session, &old_part, BomSide::Top, 1)),
        Err(CommandError::NotFound(_))
    ));
    assert!(matches!(
        reverse_take_service(&db, &first.movement_id),
        Err(CommandError::NotFound(_))
    ));

    let box_id = db
        .connection()
        .query_row("SELECT id FROM boxes LIMIT 1", [], |row| {
            row.get::<_, i64>(0)
        })
        .unwrap();
    let replacement = create_part_service(
        &db,
        PartInput {
            name: "Replacement resistor".into(),
            category: "resistor".into(),
            package: "0603".into(),
            manufacturer: "Acme".into(),
            mpn: "R-10K".into(),
            lcsc_code: "C123".into(),
            quantity: 5,
            box_id: Some(box_id),
            slot: Some("A0".into()),
            note: String::new(),
        },
    )
    .unwrap();
    // R1 was consumed before deletion and must never be charged to new stock.
    assert!(matches!(
        confirm_take_service(&db, take(&session, &replacement.id, BomSide::Top, 1)),
        Err(CommandError::Validation(_))
    ));
    let second = confirm_take_service(
        &db,
        take_designating(&session, &replacement.id, BomSide::Top, 1, &["R2"]),
    )
    .unwrap();
    assert_eq!(second.consumed_quantity, 2);
    let progress = get_welding_progress_service(&db, &session).unwrap();
    assert_eq!(
        progress[0].part_id.as_deref(),
        Some(replacement.id.as_str())
    );
    assert_eq!(progress[0].confirmed_designators, vec!["R1", "R2"]);
    assert!(matches!(
        reverse_take_service(&db, &first.movement_id),
        Err(CommandError::NotFound(_))
    ));
    let reversed = reverse_take_service(&db, &second.movement_id).unwrap();
    assert_eq!(reversed.consumed_quantity, 1);
    assert_eq!(
        get_welding_progress_service(&db, &session).unwrap()[0].confirmed_designators,
        vec!["R1"]
    );
    let stocks: (i64, i64) = db.connection().query_row(
        "SELECT (SELECT quantity FROM parts WHERE id = ?1), (SELECT quantity FROM parts WHERE id = ?2)",
        rusqlite::params![old_part, replacement.id], |row| Ok((row.get(0)?, row.get(1)?)),
    ).unwrap();
    assert_eq!(stocks, (9, 5));
    let movements = partnest_desktop_lib::commands::movements::list_movements_service(&db).unwrap();
    let archived_take = movements
        .iter()
        .find(|m| m.id == first.movement_id)
        .unwrap();
    assert_eq!(archived_take.part_name.as_deref(), Some("10k resistor"));
    assert!(!archived_take.reversible);
}

#[test]
fn deleting_stock_keeps_completed_session_part_associations() {
    let (db, session, part) = fixture();
    confirm_take_service(&db, take(&session, &part, BomSide::Top, 1)).unwrap();
    db.connection()
        .execute(
            "UPDATE welding_sessions SET status = 'completed' WHERE id = ?1",
            [&session],
        )
        .unwrap();
    let before = get_welding_progress_service(&db, &session).unwrap();
    delete_part_service(&db, &part).unwrap();
    assert_eq!(get_welding_progress_service(&db, &session).unwrap(), before);
}

#[test]
fn failed_progress_detachment_rolls_back_the_entire_deletion() {
    let (db, session, part) = fixture();
    confirm_take_service(&db, take(&session, &part, BomSide::Top, 1)).unwrap();
    let before = partnest_desktop_lib::commands::parts::list_parts_service(&db, None).unwrap();
    let progress = get_welding_progress_service(&db, &session).unwrap();
    db.connection()
        .execute_batch(
            "CREATE TRIGGER fail_detachment BEFORE UPDATE OF part_id ON welding_progress
        BEGIN SELECT RAISE(ABORT, 'injected deletion failure'); END;",
        )
        .unwrap();
    assert!(delete_part_service(&db, &part).is_err());
    assert_eq!(
        partnest_desktop_lib::commands::parts::list_parts_service(&db, None).unwrap(),
        before
    );
    assert_eq!(
        get_welding_progress_service(&db, &session).unwrap(),
        progress
    );
    assert_eq!(
        db.connection()
            .query_row(
                "SELECT deleted_at FROM parts WHERE id = ?1",
                [&part],
                |row| row.get::<_, Option<String>>(0)
            )
            .unwrap(),
        None
    );
}

fn stock_of(db: &Database, part_id: &str) -> i64 {
    db.connection()
        .query_row(
            "SELECT quantity FROM parts WHERE id = ?1",
            [part_id],
            |row| row.get::<_, i64>(0),
        )
        .unwrap()
}

#[test]
fn leaving_a_session_keeps_taken_stock_deducted_and_still_allows_corrections() {
    let (db, session_id, part_id) = fixture();
    let taken = confirm_take_service(&db, take(&session_id, &part_id, BomSide::Top, 2)).unwrap();
    assert_eq!(stock_of(&db, &part_id), 8);

    end_welding_session_service(&db, &session_id).unwrap();

    // 退出会话不退回已取用的库存。
    assert_eq!(stock_of(&db, &part_id), 8);

    // 结束后不能再产生新的取用。
    let blocked = confirm_take_service(
        &db,
        take_designating(&session_id, &part_id, BomSide::Top, 1, &["R2"]),
    )
    .expect_err("an ended session cannot take parts");
    assert!(matches!(blocked, CommandError::NotFound(_)));

    // 已经发生的取用仍可撤销，用于纠正错拿。
    let movement = list_movements_service(&db)
        .unwrap()
        .into_iter()
        .find(|item| item.id == taken.movement_id)
        .expect("the take movement stays listed");
    assert!(movement.reversible, "a take stays reversible");
    let reversed = reverse_take_service(&db, &taken.movement_id).unwrap();
    assert_eq!(reversed.status, "pending");
    assert_eq!(stock_of(&db, &part_id), 10);

    assert!(end_welding_session_service(&db, &session_id).is_err());
}
