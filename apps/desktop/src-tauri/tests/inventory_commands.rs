use partnest_desktop_lib::commands::boxes::{
    create_box_service, delete_box_service, list_boxes_service, resize_box_service,
    update_box_service, BoxInput,
};
use partnest_desktop_lib::commands::parts::{
    adjust_stock_service, create_part_service, delete_part_service, format_lcsc_display_name,
    list_parts_service, lookup_lcsc_with_fetcher, update_part_service, LcscPartInfo, PartInput,
};
use partnest_desktop_lib::commands::CommandError;
use partnest_desktop_lib::db::{new_id, Database};

fn database() -> Database {
    let path = std::env::temp_dir().join(format!("partnest-inventory-test-{}", new_id()));
    Database::open(path).expect("open test database")
}

fn box_input(rows: i64, cols: i64) -> BoxInput {
    BoxInput {
        name: "Test box".into(),
        rows,
        cols,
    }
}

fn part_input(box_id: impl ToString, slot: &str, quantity: i64) -> PartInput {
    PartInput {
        name: "10k resistor".into(),
        category: "resistor".into(),
        package: "0603".into(),
        manufacturer: "Acme".into(),
        mpn: "R-10K".into(),
        lcsc_code: "C123".into(),
        quantity,
        box_id: Some(box_id.to_string().parse().expect("integer box id")),
        slot: Some(slot.into()),
        note: "test".into(),
    }
}

#[test]
fn create_part_normalizes_slot_and_rejects_out_of_range_positions() {
    let db = database();
    let box_record = create_box_service(&db, box_input(2, 100)).unwrap();

    let part = create_part_service(&db, part_input(box_record.id, "a0", 2)).unwrap();
    assert_eq!(part.slot.as_deref(), Some("A0"));
    let mut multi_input = part_input(box_record.id, "a10", 2);
    multi_input.lcsc_code = "C124".into();
    let multi_digit = create_part_service(&db, multi_input).unwrap();
    assert_eq!(multi_digit.slot.as_deref(), Some("A10"));

    assert!(create_part_service(&db, part_input(box_record.id, "C0", 1)).is_err());
    assert!(create_part_service(&db, part_input(box_record.id, "A100", 1)).is_err());
    for invalid in ["A01", "A+1", "A-0", "A１", "Ａ1", "é1", "盒1"] {
        assert!(
            create_part_service(&db, part_input(box_record.id, invalid, 1)).is_err(),
            "{invalid}"
        );
    }
}

#[test]
fn create_part_accepts_a_trimmed_box_identifier() {
    let db = database();
    let box_record = create_box_service(
        &db,
        BoxInput {
            name: "Trim box".into(),
            rows: 2,
            cols: 2,
        },
    )
    .unwrap();
    let part = create_part_service(&db, part_input(box_record.id, "A0", 1)).unwrap();
    assert_eq!(part.box_id, Some(box_record.id));
}

#[test]
fn create_and_update_parts_store_canonical_lcsc_codes() {
    let db = database();
    let box_record = create_box_service(&db, box_input(2, 2)).unwrap();
    let mut input = part_input(box_record.id, "A0", 1);
    input.lcsc_code = " c123 ".into();
    let part = create_part_service(&db, input.clone()).unwrap();
    assert_eq!(part.lcsc_code.as_deref(), Some("C123"));

    input.lcsc_code = " c124 ".into();
    let updated = update_part_service(&db, &part.id, part.version, input).unwrap();
    assert_eq!(updated.lcsc_code.as_deref(), Some("C124"));
}

#[test]
fn canonical_lcsc_codes_are_unique_case_insensitively() {
    let db = database();
    let box_record = create_box_service(&db, box_input(2, 2)).unwrap();
    let mut first = part_input(box_record.id, "A0", 1);
    first.lcsc_code = "C123".into();
    create_part_service(&db, first).unwrap();
    let mut duplicate = part_input(box_record.id, "A1", 1);
    duplicate.lcsc_code = " c123 ".into();

    assert!(matches!(
        create_part_service(&db, duplicate),
        Err(CommandError::Constraint(_))
    ));
}

#[test]
fn box_rejects_more_than_one_hundred_columns() {
    let db = database();
    assert!(create_box_service(&db, box_input(2, 101)).is_err());
}

#[test]
fn resize_rejects_shrinking_away_an_occupied_slot() {
    let db = database();
    let box_record = create_box_service(&db, box_input(10, 10)).unwrap();
    create_part_service(&db, part_input(box_record.id, "A9", 1)).unwrap();

    let error = resize_box_service(&db, box_record.id, 2, 2).unwrap_err();
    assert_eq!(error.to_string(), "目标规格包含不了已占用盒位 A9");
}

#[test]
fn duplicate_slot_is_rejected() {
    let db = database();
    let box_record = create_box_service(&db, box_input(2, 2)).unwrap();
    create_part_service(&db, part_input(box_record.id, "A0", 1)).unwrap();
    assert!(create_part_service(&db, part_input(box_record.id, "a0", 1)).is_err());
}

#[test]
fn stale_version_update_returns_conflict() {
    let db = database();
    let box_record = create_box_service(&db, box_input(2, 2)).unwrap();
    let part = create_part_service(&db, part_input(box_record.id, "A0", 1)).unwrap();

    let mut changed = part_input(box_record.id, "A0", 99);
    changed.name = "updated".into();
    let updated = update_part_service(&db, &part.id, part.version, changed.clone()).unwrap();
    assert_eq!(updated.version, part.version + 1);
    assert_eq!(updated.quantity, 1);
    assert!(matches!(
        update_part_service(&db, &part.id, part.version, changed),
        Err(CommandError::Conflict)
    ));
}

#[test]
fn editing_zero_stock_can_add_initial_quantity_and_audits_restock() {
    let db = database();
    let box_record = create_box_service(&db, box_input(2, 2)).unwrap();
    let part = create_part_service(&db, part_input(box_record.id, "A0", 0)).unwrap();
    let mut input = part_input(box_record.id, "A1", 4);
    input.name = "restocked resistor".into();
    let updated = update_part_service(&db, &part.id, part.version, input).unwrap();
    assert_eq!(updated.quantity, 4);
    assert_eq!(updated.slot.as_deref(), Some("A1"));
    assert_eq!(db.connection().query_row(
        "SELECT movement_type, quantity, before_quantity, after_quantity, reason FROM inventory_movements WHERE part_id = ?1",
        [&part.id], |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?, row.get::<_, i64>(2)?, row.get::<_, i64>(3)?, row.get::<_, String>(4)?))
    ).unwrap(), ("in".into(), 4, 0, 4, "restock during edit".into()));
}

#[test]
fn editing_positive_stock_keeps_quantity_on_metadata_update() {
    let db = database();
    let box_record = create_box_service(&db, box_input(2, 2)).unwrap();
    let part = create_part_service(&db, part_input(box_record.id, "A0", 3)).unwrap();
    let updated = update_part_service(
        &db,
        &part.id,
        part.version,
        part_input(box_record.id, "A0", 99),
    )
    .unwrap();
    assert_eq!(updated.quantity, 3);
    assert_eq!(
        db.connection()
            .query_row(
                "SELECT COUNT(*) FROM inventory_movements WHERE part_id = ?1",
                [&part.id],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
        1
    );
}

#[test]
fn list_parts_filters_by_name_and_mpn() {
    let db = database();
    let box_record = create_box_service(&db, box_input(2, 2)).unwrap();
    create_part_service(&db, part_input(box_record.id, "A0", 1)).unwrap();
    let mut other = part_input(box_record.id, "A1", 1);
    other.name = "capacitor".into();
    other.mpn = "C-100".into();
    other.lcsc_code = "C125".into();
    create_part_service(&db, other).unwrap();

    let results = list_parts_service(&db, Some("c-100")).unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].name, "capacitor");
}

#[test]
fn stock_adjustment_rejects_negative_inventory_and_writes_audit_row() {
    let db = database();
    let box_record = create_box_service(&db, box_input(2, 2)).unwrap();
    let part = create_part_service(&db, part_input(box_record.id, "A0", 1)).unwrap();

    assert!(adjust_stock_service(&db, &part.id, -2, "consume").is_err());
    let adjusted = adjust_stock_service(&db, &part.id, 3, "restock").unwrap();
    assert_eq!(adjusted.quantity, 4);
    assert_eq!(
        db.connection()
            .query_row(
                "SELECT COUNT(*) FROM inventory_movements WHERE part_id = ?1 AND quantity = 3 AND reason = 'restock'",
                [&part.id],
                |row| row.get::<_, i64>(0),
            )
            .unwrap(),
        1
    );
    assert_eq!(
        db.connection()
            .query_row(
                "SELECT before_quantity, after_quantity FROM inventory_movements WHERE part_id = ?1 AND reason = 'restock'",
                [&part.id],
                |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)),
            )
            .unwrap(),
        (1, 4)
    );
}

#[test]
fn create_part_audits_nonzero_initial_stock_atomically() {
    let db = database();
    let box_record = create_box_service(&db, box_input(2, 2)).unwrap();
    let part = create_part_service(&db, part_input(box_record.id, "A0", 7)).unwrap();
    assert_eq!(
        db.connection()
            .query_row(
                "SELECT movement_type, quantity, before_quantity, after_quantity FROM inventory_movements WHERE part_id = ?1",
                [&part.id],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?, row.get::<_, i64>(2)?, row.get::<_, i64>(3)?)),
            )
            .unwrap(),
        ("in".to_owned(), 7, 0, 7)
    );
}

#[test]
fn boxes_use_integer_ids_and_zero_stock_releases_its_location() {
    let db = database();
    let box_record = create_box_service(&db, box_input(2, 2)).unwrap();
    assert!(box_record.id > 0);
    let part = create_part_service(&db, part_input(box_record.id.to_string(), "A0", 1)).unwrap();

    let cleared = adjust_stock_service(&db, &part.id, -1, "消耗").unwrap();
    assert_eq!(cleared.quantity, 0);
    assert_eq!(cleared.box_id, None);
    assert_eq!(cleared.slot, None);
    assert!(list_parts_service(&db, None)
        .unwrap()
        .iter()
        .all(|item| item.slot.is_none()));
    let replenished = adjust_stock_service(&db, &part.id, 2, "补库").unwrap();
    assert_eq!(replenished.quantity, 2);
    assert_eq!(replenished.box_id, None);
    assert_eq!(replenished.slot, None);
    assert!(list_boxes_service(&db).unwrap()[0]
        .occupied_slots
        .is_empty());
}

#[test]
fn editing_depleted_part_restocks_with_location_and_audit_atomically() {
    let db = database();
    let box_record = create_box_service(&db, box_input(2, 2)).unwrap();
    let part = create_part_service(&db, part_input(box_record.id, "A0", 1)).unwrap();
    let cleared = adjust_stock_service(&db, &part.id, -1, "consume").unwrap();
    let mut input = part_input(box_record.id, "A1", 12);
    input.box_id = None;
    assert!(update_part_service(&db, &part.id, cleared.version, input.clone()).is_err());
    input.box_id = Some(box_record.id);
    input.quantity = -1;
    assert!(update_part_service(&db, &part.id, cleared.version, input.clone()).is_err());
    input.quantity = 12;
    assert!(matches!(
        update_part_service(&db, &part.id, part.version, input.clone()),
        Err(CommandError::Conflict)
    ));
    let mut occupied = part_input(box_record.id, "A1", 1);
    occupied.lcsc_code = "C999".into();
    let other = create_part_service(&db, occupied).unwrap();
    assert!(update_part_service(&db, &part.id, cleared.version, input.clone()).is_err());
    assert_eq!(
        list_parts_service(&db, None)
            .unwrap()
            .iter()
            .find(|p| p.id == part.id)
            .unwrap(),
        &cleared
    );
    adjust_stock_service(&db, &other.id, -1, "consume").unwrap();

    let updated = update_part_service(&db, &part.id, cleared.version, input).unwrap();
    assert_eq!(updated.quantity, 12);
    assert_eq!(updated.box_id, Some(box_record.id));
    assert_eq!(updated.slot.as_deref(), Some("A1"));
    assert_eq!(updated.version, cleared.version + 1);
    assert_eq!(
        list_boxes_service(&db).unwrap()[0].occupied_slots,
        vec!["A1"]
    );
    let movements: i64 = db
        .connection()
        .query_row(
            "SELECT COUNT(*) FROM inventory_movements WHERE part_id = ?1",
            [&part.id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(movements, 3);
    let movement = db.connection().query_row(
        "SELECT movement_type, quantity, before_quantity, after_quantity FROM inventory_movements WHERE part_id = ?1 ORDER BY movement_sequence DESC LIMIT 1",
        [&part.id],
        |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?, row.get::<_, i64>(2)?, row.get::<_, i64>(3)?)),
    ).unwrap();
    assert_eq!(movement, ("in".into(), 12, 0, 12));
}

#[test]
fn updating_part_moves_between_boxes_and_rejects_an_occupied_destination() {
    let db = database();
    let first_box = create_box_service(&db, box_input(2, 2)).unwrap();
    let second_box = create_box_service(&db, box_input(2, 2)).unwrap();
    let first = create_part_service(&db, part_input(first_box.id, "A0", 2)).unwrap();
    let mut second_input = part_input(second_box.id, "A0", 2);
    second_input.lcsc_code = "C997".into();
    let _second = create_part_service(&db, second_input).unwrap();

    let mut moved_input = part_input(second_box.id, "B1", 2);
    moved_input.name = first.name.clone();
    moved_input.lcsc_code = "C999".into();
    let moved = update_part_service(&db, &first.id, first.version, moved_input).unwrap();
    assert_eq!(moved.box_id, Some(second_box.id));
    assert_eq!(moved.slot.as_deref(), Some("B1"));

    let mut conflict_input = part_input(second_box.id, "A0", 2);
    conflict_input.name = moved.name;
    conflict_input.lcsc_code = "C998".into();
    assert!(matches!(
        update_part_service(&db, &moved.id, moved.version, conflict_input),
        Err(CommandError::Constraint(_))
    ));
}

#[test]
fn deletion_archives_audited_parts_and_releases_their_box() {
    let db = database();
    let box_record = create_box_service(&db, box_input(2, 2)).unwrap();
    let part = create_part_service(&db, part_input(box_record.id, "A0", 1)).unwrap();
    assert!(matches!(
        delete_box_service(&db, box_record.id),
        Err(CommandError::Constraint(_))
    ));
    delete_part_service(&db, &part.id).unwrap();
    assert!(list_parts_service(&db, None).unwrap().is_empty());
    assert!(list_boxes_service(&db).unwrap()[0]
        .occupied_slots
        .is_empty());
    let archived: (String, i64, Option<i64>, Option<String>, i64, bool) = db.connection().query_row(
        "SELECT name, quantity, box_id, slot, version, deleted_at IS NOT NULL FROM parts WHERE id = ?1",
        [&part.id],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?, row.get(5)?)),
    ).unwrap();
    assert_eq!(archived, (part.name, 1, None, None, part.version + 1, true));
    assert_eq!(
        db.connection()
            .query_row(
                "SELECT COUNT(*) FROM inventory_movements WHERE part_id = ?1",
                [&part.id],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
        1
    );
    delete_box_service(&db, box_record.id).unwrap();
    let empty_box = create_box_service(&db, box_input(1, 1)).unwrap();
    let renamed = update_box_service(
        &db,
        empty_box.id,
        BoxInput {
            name: "Renamed".into(),
            rows: 1,
            cols: 1,
        },
    )
    .unwrap();
    assert_eq!(renamed.name, "Renamed");
    delete_box_service(&db, empty_box.id).unwrap();
}

#[test]
fn deleting_legacy_positive_stock_preserves_its_historical_quantity() {
    let db = database();
    let box_record = create_box_service(&db, box_input(1, 2)).unwrap();
    db.connection()
        .execute(
            "INSERT INTO parts (id, name, quantity, box_id, slot, version) VALUES (?1, 'legacy', 5, ?2, 'A0', 1)",
            rusqlite::params![new_id(), box_record.id],
        )
        .unwrap();
    let legacy_id: String = db
        .connection()
        .query_row("SELECT id FROM parts WHERE name = 'legacy'", [], |row| {
            row.get(0)
        })
        .unwrap();
    delete_part_service(&db, &legacy_id).unwrap();
    assert!(list_parts_service(&db, None).unwrap().is_empty());
    assert_eq!(
        db.connection()
            .query_row(
                "SELECT quantity FROM parts WHERE id = ?1",
                [&legacy_id],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
        5
    );
    assert_eq!(
        db.connection()
            .query_row("SELECT COUNT(*) FROM inventory_movements", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
}

#[test]
fn lcsc_lookup_prefers_the_network_and_falls_back_to_its_cached_metadata() {
    let db = database();
    let fetched = lookup_lcsc_with_fetcher(&db, " c1549 ", |code| {
        assert_eq!(code, "C1549");
        Ok(LcscPartInfo {
            lcsc_code: code.into(),
            name: "0402CG180J500NT".into(),
            category: "Capacitors".into(),
            package: "0402".into(),
            manufacturer: "FH".into(),
            mpn: "0402CG180J500NT".into(),
        })
    })
    .expect("network lookup");
    assert_eq!(fetched.lcsc_code, "C1549");
    let cached =
        lookup_lcsc_with_fetcher(&db, "C1549", |_| Err("offline".into())).expect("cached lookup");
    assert_eq!(cached, fetched);
}

#[test]
fn lcsc_display_name_combines_resistance_and_package() {
    assert_eq!(
        format_lcsc_display_name("5.1kΩ ±1% 1/16W", "R0402"),
        "5.1kΩ R0402"
    );
}

#[test]
fn lcsc_display_name_keeps_an_unrecognized_model_unchanged() {
    assert_eq!(
        format_lcsc_display_name("RC0402FR-075K1L", "R0402"),
        "RC0402FR-075K1L"
    );
}

#[test]
fn deleted_parts_cannot_be_mutated_and_their_slot_and_lcsc_can_be_reused() {
    let db = database();
    let b = create_box_service(&db, box_input(2, 2)).unwrap();
    let input = part_input(b.id, "A0", 7);
    let original = create_part_service(&db, input.clone()).unwrap();
    delete_part_service(&db, &original.id).unwrap();
    assert!(list_parts_service(&db, Some("C123")).unwrap().is_empty());
    assert!(matches!(
        delete_part_service(&db, &original.id),
        Err(CommandError::NotFound(_))
    ));
    assert!(matches!(
        delete_part_service(&db, "missing"),
        Err(CommandError::NotFound(_))
    ));
    assert!(matches!(
        update_part_service(&db, &original.id, original.version, input.clone()),
        Err(CommandError::NotFound(_))
    ));
    assert!(matches!(
        adjust_stock_service(&db, &original.id, 1, "restock"),
        Err(CommandError::NotFound(_))
    ));
    let replacement = create_part_service(&db, input).unwrap();
    assert_ne!(replacement.id, original.id);
    assert_eq!(list_parts_service(&db, None).unwrap().len(), 1);
    assert_eq!(
        list_boxes_service(&db).unwrap()[0].occupied_slots,
        vec!["A0"]
    );
    let mut duplicate = part_input(b.id, "A1", 1);
    duplicate.lcsc_code = " c123 ".into();
    assert!(matches!(
        create_part_service(&db, duplicate),
        Err(CommandError::Constraint(_))
    ));
}

#[test]
fn zero_stock_parts_can_be_deleted_without_creating_movements() {
    let db = database();
    let b = create_box_service(&db, box_input(1, 1)).unwrap();
    let part = create_part_service(&db, part_input(b.id, "A0", 0)).unwrap();
    delete_part_service(&db, &part.id).unwrap();
    assert!(list_parts_service(&db, None).unwrap().is_empty());
    assert_eq!(
        db.connection()
            .query_row("SELECT COUNT(*) FROM inventory_movements", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
}
