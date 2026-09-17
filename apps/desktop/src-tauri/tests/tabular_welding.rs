use partnest_desktop_lib::bom::cache::InteractiveBomRuntime;
use partnest_desktop_lib::bom::projects::ProjectKind;
use partnest_desktop_lib::bom::types::BomSide;
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

fn cache_entries(dir: &Path) -> usize {
    fs::read_dir(dir)
        .map(|entries| entries.count())
        .unwrap_or(0)
}

/// Import a table as a project and open the welding workspace for it.
fn open_table_project(
    runtime: &InteractiveBomRuntime,
    db: &Database,
    source: &Path,
) -> partnest_desktop_lib::bom::cache::CachedBomSession {
    let imported = runtime
        .import_project(db, source, None, None, None)
        .unwrap();
    assert_eq!(imported.kind, ProjectKind::Tabular);
    runtime
        .open_project_welding(db, &imported.project_id)
        .unwrap()
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
fn a_side_less_tabular_project_drives_the_welding_workspace_from_its_snapshot() {
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
    let session = open_table_project(&runtime, &db, &source);
    assert!(session.cache_path.is_none(), "no canvas is cached");
    assert_eq!(cache_entries(&cache_dir), 0, "no cache copy is written");
    let project_id = session.project_id.clone();

    // A restart reopens the session straight from the stored snapshot.
    let restarted = InteractiveBomRuntime::new(&cache_dir);
    let restored = restarted
        .restore_active_session(&db)
        .unwrap()
        .expect("the opened session survives a restart");
    assert_eq!(restored.session_id, session.session_id);
    assert_eq!(restored.project_id, project_id);
    assert_eq!(restored.kind, ProjectKind::Tabular);
    assert_eq!(restored.normalized.groups.len(), 1);

    // Tray selections carry no side; the operator's side tab decides.
    let resolved = restarted
        .resolve_bom_selection(&db, &restored.token, &["R1".into(), "R2".into()])
        .unwrap();
    assert_eq!(resolved.side, None);
    let component_key = restored.normalized.groups[0].component_key.clone();
    assert_eq!(resolved.component_key, component_key);

    let part = stocked_part(&db);
    let input = ConfirmTakeInput {
        session_id: restored.session_id.clone(),
        component_key,
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
fn a_tabular_project_with_a_side_column_keeps_enforcing_its_recorded_side() {
    let root = tempdir().unwrap();
    let db = Database::open(root.path().join("partnest.db")).unwrap();
    let runtime = InteractiveBomRuntime::new(root.path().join("cache"));
    let source = write_csv(
        root.path(),
        "sided.csv",
        "Designator,Footprint,Value,Manufacturer Part,Side\nR1,0603,10k,R-10K,top\n",
    );
    let session = open_table_project(&runtime, &db, &source);

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
            component_key: session.normalized.groups[0].component_key.clone(),
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
fn opening_a_table_project_needs_no_source_file() {
    let root = tempdir().unwrap();
    let db = Database::open(root.path().join("partnest.db")).unwrap();
    let runtime = InteractiveBomRuntime::new(root.path().join("cache"));
    let source = write_csv(
        root.path(),
        "board.csv",
        "Designator,Footprint,Value\nR1,0603,10k\n",
    );
    let imported = runtime
        .import_project(&db, &source, Some("主板"), None, None)
        .unwrap();
    fs::remove_file(&source).unwrap();

    let session = runtime
        .open_project_welding(&db, &imported.project_id)
        .unwrap();
    assert_eq!(session.kind, ProjectKind::Tabular);
    assert_eq!(session.project_name, "主板");
    assert_eq!(session.normalized.groups, imported.normalized.groups);
    assert!(runtime
        .restore_active_session(&db)
        .unwrap()
        .is_some_and(|restored| restored.session_id == session.session_id));
}

#[test]
fn opening_an_unknown_project_reports_a_readable_error() {
    let root = tempdir().unwrap();
    let db = Database::open(root.path().join("partnest.db")).unwrap();
    let runtime = InteractiveBomRuntime::new(root.path().join("cache"));
    let missing = runtime
        .open_project_welding(&db, &new_id())
        .expect_err("an unknown id must fail");
    assert!(missing.to_string().contains("is not available"));
    assert!(missing.user_message().contains("找不到该项目"));
    assert_eq!(cache_entries(&root.path().join("cache")), 0);
}

#[test]
fn a_table_that_still_needs_mapping_is_not_imported() {
    let root = tempdir().unwrap();
    let db = Database::open(root.path().join("partnest.db")).unwrap();
    let runtime = InteractiveBomRuntime::new(root.path().join("cache"));
    let source = write_csv(
        root.path(),
        "odd.csv",
        "Designator stuff,Pin count\nR1 R2,2\n",
    );

    let error = runtime
        .import_project(&db, &source, None, None, None)
        .expect_err("an unmappable table must stop before any write");
    assert!(error.user_message().contains("字段映射"));
    assert_eq!(
        db.connection()
            .query_row("SELECT COUNT(*) FROM projects", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
}
