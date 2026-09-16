use partnest_desktop_lib::bom::cache::InteractiveBomRuntime;
use partnest_desktop_lib::bom::history::{record_import, BomImportKind};
use partnest_desktop_lib::bom::tabular::inspect_tabular_bom;
use partnest_desktop_lib::bom::types::{BomSide, ImportPreview, NormalizedBomDto};
use partnest_desktop_lib::commands::boxes::{create_box_service, BoxInput};
use partnest_desktop_lib::commands::parts::{create_part_service, PartInput};
use partnest_desktop_lib::commands::welding::{confirm_take_authorized_service, ConfirmTakeInput};
use partnest_desktop_lib::commands::CommandError;
use partnest_desktop_lib::db::{new_id, Database};
use std::fs;
use std::path::{Path, PathBuf};
use tempfile::tempdir;

fn write_csv(dir: &Path, name: &str, contents: &str) -> PathBuf {
    let path = dir.join(name);
    fs::write(&path, contents).unwrap();
    path
}

fn normalized(preview: ImportPreview) -> NormalizedBomDto {
    match preview {
        ImportPreview::Ready(bom) => bom,
        ImportPreview::NeedsMapping { headers, .. } => {
            panic!("fixture unexpectedly needs mapping: {headers:?}")
        }
    }
}

fn cache_entries(dir: &Path) -> usize {
    fs::read_dir(dir)
        .map(|entries| entries.count())
        .unwrap_or(0)
}

/// A stocked part has to live in a box slot, so both take tests need one.
fn stocked_part(db: &Database) -> partnest_desktop_lib::commands::parts::PartView {
    let box_record = create_box_service(
        db,
        BoxInput {
            name: "Test box".into(),
            rows: 2,
            cols: 2,
        },
    )
    .unwrap();
    create_part_service(
        db,
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
    .unwrap()
}

#[test]
fn a_side_less_tabular_bom_drives_the_welding_workspace_from_its_snapshot() {
    let root = tempdir().unwrap();
    let db = Database::open(root.path().join("partnest.db")).unwrap();
    let cache_dir = root.path().join("cache");
    let runtime = InteractiveBomRuntime::new(&cache_dir);
    // EasyEDA-style export without a board-side column.
    let source = write_csv(
        root.path(),
        "board.csv",
        "Designator,Footprint,Value,Manufacturer Part\n\"R1,R2\",0603,10k,R-10K\n",
    );
    let import = normalized(inspect_tabular_bom(&source, None).unwrap());
    record_import(&db, &source, None, BomImportKind::Tabular, &import).unwrap();

    let session = runtime
        .activate_imported_bom(&db, None, Some(&source), None)
        .unwrap();
    assert_eq!(session.kind, BomImportKind::Tabular);
    assert!(session.cache_path.is_none(), "no canvas is cached");
    assert_eq!(cache_entries(&cache_dir), 0, "no cache copy is written");

    // A restart reopens the session straight from the stored snapshot.
    let restarted = InteractiveBomRuntime::new(&cache_dir);
    let restored = restarted
        .restore_active_session(&db)
        .unwrap()
        .expect("the activated session survives a restart");
    assert_eq!(restored.session_id, session.session_id);
    assert_eq!(restored.kind, BomImportKind::Tabular);
    assert_eq!(restored.normalized.groups, import.groups);

    // Tray selections carry no side; the operator's side tab decides.
    let resolved = restarted
        .resolve_bom_selection(&db, &restored.token, &["R1".into(), "R2".into()])
        .unwrap();
    assert_eq!(resolved.side, None);
    assert_eq!(resolved.component_key, import.groups[0].component_key);

    let part = stocked_part(&db);
    let input = ConfirmTakeInput {
        session_id: restored.session_id.clone(),
        component_key: import.groups[0].component_key.clone(),
        side: BomSide::Top,
        designators: vec!["R1".into()],
        bom_quantity: 1,
        take_quantity: 1,
        part_id: part.id.clone(),
        expected_part_version: part.version,
    };
    let taken = confirm_take_authorized_service(&db, &restarted, input.clone()).unwrap();
    assert_eq!(taken.side, BomSide::Top);
    assert_eq!(taken.consumed_quantity, 1);

    // The other board side must not charge the same board position again.
    let mut repeated = input;
    repeated.side = BomSide::Bottom;
    repeated.expected_part_version = taken.part_version;
    let error = confirm_take_authorized_service(&db, &restarted, repeated)
        .expect_err("a designator is charged once per session");
    assert!(matches!(error, CommandError::Validation(message) if message.contains("均已取用")));
}

#[test]
fn a_tabular_bom_with_a_side_column_keeps_enforcing_its_recorded_side() {
    let root = tempdir().unwrap();
    let db = Database::open(root.path().join("partnest.db")).unwrap();
    let runtime = InteractiveBomRuntime::new(root.path().join("cache"));
    let source = write_csv(
        root.path(),
        "sided.csv",
        "Designator,Footprint,Value,Manufacturer Part,Side\nR1,0603,10k,R-10K,top\n",
    );
    let import = normalized(inspect_tabular_bom(&source, None).unwrap());
    record_import(&db, &source, None, BomImportKind::Tabular, &import).unwrap();
    let session = runtime
        .activate_imported_bom(&db, None, Some(&source), None)
        .unwrap();

    let resolved = runtime
        .resolve_bom_selection(&db, &session.token, &["R1".into()])
        .unwrap();
    assert_eq!(resolved.side, Some(BomSide::Top));

    let part = stocked_part(&db);
    let error = confirm_take_authorized_service(
        &db,
        &runtime,
        ConfirmTakeInput {
            session_id: session.session_id.clone(),
            component_key: import.groups[0].component_key.clone(),
            side: BomSide::Bottom,
            designators: vec!["R1".into()],
            bom_quantity: 1,
            take_quantity: 1,
            part_id: part.id,
            expected_part_version: part.version,
        },
    )
    .expect_err("a top placement cannot be charged on the bottom side");
    assert!(matches!(error, CommandError::Validation(message) if message.contains("不同板面")));
}

#[test]
fn activating_a_tabular_record_by_id_needs_no_source_file() {
    let root = tempdir().unwrap();
    let db = Database::open(root.path().join("partnest.db")).unwrap();
    let runtime = InteractiveBomRuntime::new(root.path().join("cache"));
    let source = write_csv(
        root.path(),
        "board.csv",
        "Designator,Footprint,Value\nR1,0603,10k\n",
    );
    let import = normalized(inspect_tabular_bom(&source, None).unwrap());
    record_import(&db, &source, None, BomImportKind::Tabular, &import).unwrap();
    let bom_file_id: String = db
        .connection()
        .query_row("SELECT id FROM bom_files", [], |row| row.get(0))
        .unwrap();
    fs::remove_file(&source).unwrap();

    let session = runtime
        .activate_imported_bom(&db, Some(&bom_file_id), None, None)
        .unwrap();
    assert_eq!(session.kind, BomImportKind::Tabular);
    assert_eq!(session.normalized.groups, import.groups);
    assert!(runtime
        .restore_active_session(&db)
        .unwrap()
        .is_some_and(|restored| restored.session_id == session.session_id));
}

#[test]
fn activating_an_unknown_import_reports_a_readable_error() {
    let root = tempdir().unwrap();
    let db = Database::open(root.path().join("partnest.db")).unwrap();
    let runtime = InteractiveBomRuntime::new(root.path().join("cache"));
    let missing = runtime
        .activate_imported_bom(&db, Some(&new_id()), None, None)
        .expect_err("an unknown id must fail");
    assert!(missing.to_string().contains("not available"));

    let unregistered = write_csv(root.path(), "loose.csv", "Designator\nR1\n");
    let error = runtime
        .activate_imported_bom(&db, None, Some(&unregistered), None)
        .expect_err("a file that was never analysed must fail");
    assert!(error.user_message().contains("找不到该 BOM 记录"));
}
