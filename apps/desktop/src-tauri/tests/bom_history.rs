use partnest_desktop_lib::bom::cache::{preview_interactive_bom, InteractiveBomCache};
use partnest_desktop_lib::bom::history::{
    load_import, record_import, remove_import, BomImportKind,
};
use partnest_desktop_lib::bom::tabular::inspect_tabular_bom;
use partnest_desktop_lib::bom::types::{ImportPreview, NormalizedBomDto};
use partnest_desktop_lib::db::Database;
use std::path::{Path, PathBuf};
use tempfile::tempdir;

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../fixtures/bom")
        .join(name)
}

fn ready(preview: ImportPreview) -> NormalizedBomDto {
    match preview {
        ImportPreview::Ready(bom) => bom,
        ImportPreview::NeedsMapping { headers, .. } => {
            panic!("fixture unexpectedly needs mapping: {headers:?}")
        }
    }
}

fn stored_rows(db: &Database) -> Vec<(String, String, String, String, Option<String>)> {
    let mut stmt = db
        .connection()
        .prepare("SELECT id, original_name, display_name, kind, normalized_json FROM bom_files ORDER BY created_at")
        .unwrap();
    stmt.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, String>(3)?,
            row.get::<_, Option<String>>(4)?,
        ))
    })
    .unwrap()
    .collect::<Result<Vec<_>, _>>()
    .unwrap()
}

#[test]
fn a_tabular_import_is_stored_as_a_database_snapshot() {
    let root = tempdir().unwrap();
    let db = Database::open(root.path().join("partnest.db")).unwrap();
    let source = root.path().join("board.csv");
    std::fs::copy(fixture("comma-utf8.csv"), &source).unwrap();
    let parsed = ready(inspect_tabular_bom(&source, None).unwrap());

    record_import(&db, &source, None, BomImportKind::Tabular, &parsed).unwrap();
    record_import(&db, &source, Some("  "), BomImportKind::Tabular, &parsed).unwrap();

    let rows = stored_rows(&db);
    assert_eq!(rows.len(), 1, "same content must deduplicate into one row");
    assert_eq!(rows[0].1, "board.csv");
    assert_eq!(
        rows[0].2, "board.csv",
        "blank remarks fall back to the file name"
    );
    assert_eq!(rows[0].3, "tabular");
    assert!(
        rows[0].4.is_some(),
        "the parsed BOM is stored in the database"
    );

    // Reopening must not depend on the original file still being there.
    std::fs::remove_file(&source).unwrap();
    let reopened = load_import(&db, &rows[0].0).unwrap();
    assert_eq!(reopened.groups, parsed.groups);
}

#[test]
fn a_late_import_remark_replaces_the_file_name_only_when_given() {
    let root = tempdir().unwrap();
    let db = Database::open(root.path().join("partnest.db")).unwrap();
    let source = fixture("comma-utf8.csv");
    let parsed = ready(inspect_tabular_bom(&source, None).unwrap());

    record_import(&db, &source, Some("主板"), BomImportKind::Tabular, &parsed).unwrap();
    record_import(&db, &source, None, BomImportKind::Tabular, &parsed).unwrap();
    assert_eq!(stored_rows(&db)[0].2, "主板");

    record_import(&db, &source, Some("副板"), BomImportKind::Tabular, &parsed).unwrap();
    assert_eq!(stored_rows(&db)[0].2, "副板");
}

#[test]
fn an_interactive_import_keeps_the_activated_remark_and_its_snapshot() {
    let root = tempdir().unwrap();
    let db = Database::open(root.path().join("partnest.db")).unwrap();
    let source = fixture("interactive-minimal.html");
    let activated = InteractiveBomCache::new(&db, root.path().join("cache"))
        .cache_interactive_bom(&source, "板子 A")
        .unwrap();
    let parsed = preview_interactive_bom(&source, None).unwrap();

    record_import(&db, &source, None, BomImportKind::Interactive, &parsed).unwrap();

    let rows = stored_rows(&db);
    assert_eq!(rows.len(), 1);
    assert_eq!(
        rows[0].0, activated.bom_file_id,
        "sha upsert reuses the row"
    );
    assert_eq!(
        rows[0].2, "板子 A",
        "an empty remark must not clobber the activation name"
    );
    assert_eq!(rows[0].3, "interactive");
    assert_eq!(load_import(&db, &rows[0].0).unwrap().groups, parsed.groups);
}

#[test]
fn removing_an_import_keeps_session_rows_and_deletes_the_rest() {
    let root = tempdir().unwrap();
    let db = Database::open(root.path().join("partnest.db")).unwrap();
    let cache_dir = root.path().join("cache");
    let source = fixture("interactive-minimal.html");
    let activated = InteractiveBomCache::new(&db, &cache_dir)
        .cache_interactive_bom(&source, "板子 A")
        .unwrap();
    let parsed = preview_interactive_bom(&source, None).unwrap();
    record_import(&db, &source, None, BomImportKind::Interactive, &parsed).unwrap();

    let refused = remove_import(&db, &cache_dir, &activated.bom_file_id)
        .expect_err("a record used by a welding session must survive");
    assert!(refused.to_string().contains("welding session"));
    assert_eq!(stored_rows(&db).len(), 1);
    assert!(cache_dir.join(&activated.cache_name).is_file());

    db.connection()
        .execute(
            "DELETE FROM welding_sessions WHERE bom_file_id = ?1",
            [&activated.bom_file_id],
        )
        .unwrap();
    remove_import(&db, &cache_dir, &activated.bom_file_id).unwrap();
    assert!(stored_rows(&db).is_empty());
    assert!(!cache_dir.join(&activated.cache_name).exists());
    assert!(
        remove_import(&db, &cache_dir, &activated.bom_file_id).is_err(),
        "removing a missing record reports an error"
    );
}

#[test]
fn reopening_unknown_or_unsnapshotted_rows_reports_a_readable_error() {
    let root = tempdir().unwrap();
    let db = Database::open(root.path().join("partnest.db")).unwrap();
    let missing = load_import(&db, "missing-id").expect_err("unknown id must fail");
    assert!(missing.to_string().contains("not available"));

    db.connection()
        .execute(
            "INSERT INTO bom_files (id, original_name, display_name, sha256, cache_name) VALUES ('legacy', 'old.csv', 'Old', 'legacy-hash', 'legacy-hash.csv')",
            [],
        )
        .unwrap();
    let legacy = load_import(&db, "legacy").expect_err("rows without a snapshot must fail");
    assert!(legacy.to_string().contains("no analysis snapshot"));
    assert!(legacy.user_message().contains("重新选择原始文件"));
}
