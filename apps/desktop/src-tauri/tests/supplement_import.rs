use partnest_desktop_lib::bom::cache::InteractiveBomCache;
use partnest_desktop_lib::bom::projects::{project_record, ProjectKind};
use partnest_desktop_lib::db::{new_id, Database};
use std::path::{Path, PathBuf};
use tempfile::tempdir;

/// One board described both ways: the interactive export and a table that covers
/// exactly the same designators, which is what a companion merge requires.
fn sources(root: &Path) -> (PathBuf, PathBuf) {
    let html = root.join("board.html");
    std::fs::write(
        &html,
        r#"<script>window.files = {"bom_merge":{"data":{"comp_info":{"C1":{"Name":"R"}},"designator_info":{"top":[{"des":"R1","lc_code":""}],"bottom":[]}}}};</script>"#,
    )
    .unwrap();
    let csv = root.join("board.csv");
    std::fs::write(
        &csv,
        "Quantity,Comment,Designator,Footprint,Value,Manufacturer Part,Manufacturer,Supplier Part\n1,10k,R1,0603,10k,R-10K,Acme,C999\n",
    )
    .unwrap();
    (html, csv)
}

fn project_ids(db: &Database) -> Vec<String> {
    let mut statement = db
        .connection()
        .prepare("SELECT id FROM projects ORDER BY created_at")
        .unwrap();
    statement
        .query_map([], |row| row.get::<_, String>(0))
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap()
}

#[test]
fn an_interactive_project_can_receive_its_table_afterwards() {
    let root = tempdir().unwrap();
    let db = Database::open(root.path().join("partnest.db")).unwrap();
    let cache = InteractiveBomCache::new(&db, root.path().join("cache"));
    let (html, csv) = sources(root.path());
    let imported = cache.import_project(&html, None, None, None).unwrap();
    assert!(!imported.has_table, "the export alone has no parts table");
    assert_eq!(imported.normalized.groups[0].lcsc_code, "");

    let supplemented = cache
        .supplement_project(&imported.project_id, &csv, None)
        .unwrap();

    assert!(supplemented.has_table);
    assert_eq!(supplemented.kind, ProjectKind::Interactive);
    assert_eq!(
        supplemented.normalized.groups[0].lcsc_code, "C999",
        "the table's part data is merged into the analysis"
    );
    let record = project_record(&db, &imported.project_id).unwrap();
    assert_eq!(record.missing_source(), None);

    // The canvas still opens, and its cached copy now carries the merged table.
    let session = cache.open_project_welding(&imported.project_id).unwrap();
    assert!(session
        .cache_path
        .as_ref()
        .is_some_and(|path| path.is_file()));
    assert_eq!(session.normalized.groups[0].lcsc_code, "C999");
}

#[test]
fn a_table_project_can_gain_a_canvas_later_and_keeps_its_analysis() {
    let root = tempdir().unwrap();
    let db = Database::open(root.path().join("partnest.db")).unwrap();
    let cache_dir = root.path().join("cache");
    let cache = InteractiveBomCache::new(&db, &cache_dir);
    let (html, csv) = sources(root.path());
    let imported = cache
        .import_project(&csv, Some("主板"), None, None)
        .unwrap();
    assert_eq!(imported.kind, ProjectKind::Tabular);
    assert!(imported.has_table, "a table is its own parts source");
    assert_eq!(
        std::fs::read_dir(&cache_dir)
            .map(|entries| entries.count())
            .unwrap_or(0),
        0,
        "a table import writes no canvas"
    );

    let supplemented = cache
        .supplement_project(&imported.project_id, &html, None)
        .unwrap();

    assert_eq!(supplemented.kind, ProjectKind::Interactive);
    assert_eq!(supplemented.name, "主板", "the project keeps its own name");
    let session = cache.open_project_welding(&imported.project_id).unwrap();
    assert!(
        session
            .cache_path
            .as_ref()
            .is_some_and(|path| path.is_file()),
        "the supplemented canvas is cached"
    );
    assert_eq!(session.project_name, "主板");
    assert_eq!(
        session.normalized.groups[0].lcsc_code, "C999",
        "the stored table still supplies the part data"
    );
    assert_eq!(session.normalized.groups[0].designators, ["R1"]);
}

#[test]
fn supplementing_refuses_a_complete_project_and_the_wrong_source() {
    let root = tempdir().unwrap();
    let db = Database::open(root.path().join("partnest.db")).unwrap();
    let cache = InteractiveBomCache::new(&db, root.path().join("cache"));
    let (html, csv) = sources(root.path());
    let interactive = cache.import_project(&html, None, None, None).unwrap();
    cache
        .supplement_project(&interactive.project_id, &csv, None)
        .unwrap();

    let complete = cache
        .supplement_project(&interactive.project_id, &csv, None)
        .expect_err("both sources are already present");
    assert_eq!(
        complete.user_message(),
        "该项目的交互式与表格都已导入，无需补充导入"
    );

    let table_only = cache.import_project(&csv, None, None, None).unwrap();
    assert_eq!(project_ids(&db).len(), 2);
    let wrong = cache
        .supplement_project(&table_only.project_id, &csv, None)
        .expect_err("a table cannot supplement a table project");
    assert!(wrong.user_message().contains("需要可交互式 BOM"));
}

#[test]
fn a_project_that_is_welding_or_has_takes_cannot_be_supplemented() {
    let root = tempdir().unwrap();
    let db = Database::open(root.path().join("partnest.db")).unwrap();
    let cache = InteractiveBomCache::new(&db, root.path().join("cache"));
    let (html, csv) = sources(root.path());
    let imported = cache.import_project(&html, None, None, None).unwrap();

    let session = cache.open_project_welding(&imported.project_id).unwrap();
    let active = cache
        .supplement_project(&imported.project_id, &csv, None)
        .expect_err("the session being welded is frozen");
    assert_eq!(
        active.user_message(),
        "该项目正在焊接中，请先退出当前焊接再补充导入"
    );

    db.connection().execute(
        "INSERT INTO welding_progress (id, session_id, component_key, side, required_quantity, taken_quantity) VALUES (?1, ?2, 'part:R', 'top', 1, 1)",
        rusqlite::params![new_id(), session.session_id],
    )
    .unwrap();
    db.connection()
        .execute("UPDATE welding_sessions SET status = 'completed'", [])
        .unwrap();
    let started = cache
        .supplement_project(&imported.project_id, &csv, None)
        .expect_err("re-merging would move designators away from recorded takes");
    assert_eq!(
        started.user_message(),
        "该项目已有取用记录，补充导入会改变器件归属，无法进行"
    );
}

#[test]
fn a_supplemented_canvas_that_belongs_to_another_project_is_refused() {
    let root = tempdir().unwrap();
    let db = Database::open(root.path().join("partnest.db")).unwrap();
    let cache = InteractiveBomCache::new(&db, root.path().join("cache"));
    let (html, csv) = sources(root.path());
    let table = cache.import_project(&csv, None, None, None).unwrap();
    // Another project already owns this exact export.
    let other = cache.import_project(&html, None, None, None).unwrap();
    let before = project_record(&db, &other.project_id).unwrap();

    let clash = cache
        .supplement_project(&table.project_id, &html, None)
        .expect_err("two projects cannot share one canvas hash");
    assert_eq!(
        clash.user_message(),
        "这个文件已经是另一个项目了，请在项目列表里直接打开它"
    );
    assert_eq!(
        project_record(&db, &table.project_id).unwrap().kind,
        ProjectKind::Tabular
    );
    assert_eq!(project_record(&db, &other.project_id).unwrap(), before);
}
