use partnest_desktop_lib::bom::cache::{preview_interactive_bom, InteractiveBomCache};
use partnest_desktop_lib::bom::projects::{
    load_snapshot, project_record, record_project, remove_project, rename_project, ProjectKind,
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
        .prepare("SELECT id, original_name, name, kind, normalized_json FROM projects ORDER BY created_at")
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

fn import_table(directory: &Path) -> (PathBuf, NormalizedBomDto) {
    let source = directory.join("board.csv");
    std::fs::copy(fixture("comma-utf8.csv"), &source).unwrap();
    let parsed = ready(inspect_tabular_bom(&source, None).unwrap());
    (source, parsed)
}

#[test]
fn a_tabular_project_is_stored_as_a_database_snapshot() {
    let root = tempdir().unwrap();
    let db = Database::open(root.path().join("partnest.db")).unwrap();
    let (source, parsed) = import_table(root.path());
    let bytes = std::fs::read(&source).unwrap();

    let record = record_project(
        &db,
        &source,
        &bytes,
        None,
        ProjectKind::Tabular,
        false,
        &parsed,
    )
    .unwrap();

    assert_eq!(
        record.name, "board",
        "a new project is named after its file"
    );
    assert_eq!(record.kind, ProjectKind::Tabular);
    assert!(
        record.normalized_json.is_some(),
        "the parsed BOM is stored in the database"
    );

    // Reopening must not depend on the original file still being there.
    std::fs::remove_file(&source).unwrap();
    let reopened = load_snapshot(&db, &record.id).unwrap();
    assert_eq!(reopened.groups, parsed.groups);
}

#[test]
fn reimporting_the_same_content_refreshes_the_project_and_can_rename_it() {
    let root = tempdir().unwrap();
    let db = Database::open(root.path().join("partnest.db")).unwrap();
    let (source, parsed) = import_table(root.path());
    let bytes = std::fs::read(&source).unwrap();

    let first = record_project(
        &db,
        &source,
        &bytes,
        Some("  "),
        ProjectKind::Tabular,
        false,
        &parsed,
    )
    .unwrap();
    let second = record_project(
        &db,
        &source,
        &bytes,
        Some("主板"),
        ProjectKind::Tabular,
        false,
        &parsed,
    )
    .unwrap();

    assert_eq!(first.id, second.id, "same content is one project");
    assert_eq!(stored_rows(&db).len(), 1);
    assert_eq!(second.name, "主板", "a blank name keeps the file name");

    let renamed = rename_project(&db, &second.id, "  副板  ").unwrap();
    assert_eq!(renamed.name, "副板");
    assert!(rename_project(&db, &second.id, "   ").is_err());
    assert!(rename_project(&db, "missing", "x")
        .expect_err("a missing project cannot be renamed")
        .to_string()
        .contains("not available"));
}

#[test]
fn an_imported_interactive_project_keeps_its_name_and_snapshot() {
    let root = tempdir().unwrap();
    let db = Database::open(root.path().join("partnest.db")).unwrap();
    let source = fixture("interactive-minimal.html");
    let imported = InteractiveBomCache::new(&db, root.path().join("cache"))
        .import_project(&source, Some("板子 A"), None, None)
        .unwrap();
    let parsed = preview_interactive_bom(&source, None).unwrap();

    let rows = stored_rows(&db);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].2, "板子 A");
    assert_eq!(rows[0].3, "interactive");
    assert_eq!(
        rows[0].0, imported.project_id,
        "the cache layer stores the row it just read"
    );
    assert_eq!(
        load_snapshot(&db, &rows[0].0).unwrap().groups,
        parsed.groups
    );
}

#[test]
fn removing_a_project_keeps_session_rows_and_deletes_the_rest() {
    let root = tempdir().unwrap();
    let db = Database::open(root.path().join("partnest.db")).unwrap();
    let cache_dir = root.path().join("cache");
    let source = fixture("interactive-minimal.html");
    let cache = InteractiveBomCache::new(&db, &cache_dir);
    let imported = cache.import_project(&source, None, None, None).unwrap();
    let session = cache.open_project_welding(&imported.project_id).unwrap();
    let canvas = cache_dir.join(&session.cache_name);
    assert!(canvas.is_file());

    let refused = remove_project(&db, &cache_dir, &imported.project_id)
        .expect_err("a project used by a welding session must survive");
    assert!(refused.to_string().contains("welding session"));
    assert_eq!(refused.user_message(), "该项目已有焊接会话记录，无法删除");
    assert_eq!(stored_rows(&db).len(), 1);
    assert!(canvas.is_file());

    db.connection()
        .execute(
            "DELETE FROM welding_sessions WHERE project_id = ?1",
            [&session.project_id],
        )
        .unwrap();
    remove_project(&db, &cache_dir, &imported.project_id).unwrap();
    assert!(stored_rows(&db).is_empty());
    assert!(!canvas.exists());
    assert!(
        remove_project(&db, &cache_dir, &imported.project_id).is_err(),
        "removing a missing project reports an error"
    );
}

#[test]
fn reopening_unknown_or_unsnapshotted_rows_reports_a_readable_error() {
    let root = tempdir().unwrap();
    let db = Database::open(root.path().join("partnest.db")).unwrap();
    let missing = load_snapshot(&db, "missing-id").expect_err("unknown id must fail");
    assert!(missing.to_string().contains("is not available"));

    db.connection()
        .execute(
            "INSERT INTO projects (id, name, original_name, sha256, cache_name, kind) VALUES ('legacy', 'Old', 'old.csv', 'legacy-hash', 'legacy-hash.csv', 'tabular')",
            [],
        )
        .unwrap();
    let legacy = load_snapshot(&db, "legacy").expect_err("rows without a snapshot must fail");
    assert!(legacy.to_string().contains("no analysis snapshot"));
    assert!(legacy.user_message().contains("重新导入"));
    assert!(project_record(&db, "legacy").is_ok());
}
