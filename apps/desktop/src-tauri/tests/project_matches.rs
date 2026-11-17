use partnest_desktop_lib::{
    bom::imports,
    commands::projects::{remove_project_match, save_project_match},
    db::Database,
};
use std::path::Path;
use tempfile::tempdir;

#[test]
fn manual_matches_survive_reopening_without_changing_stock_and_reject_invalid_references() {
    let root = tempdir().unwrap();
    let path = root.path().join("inventory.db");
    let db = Database::open(&path).unwrap();
    db.connection()
        .execute(
            "INSERT INTO parts(id,name,quantity) VALUES ('stock','Chosen part',7)",
            [],
        )
        .unwrap();
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../fixtures/bom/comma-utf8.csv");
    let imported = imports::commit_import(
        &db,
        imports::prepare_import(&source, None, None).unwrap(),
        None,
    )
    .unwrap();
    let key = &imported.normalized.groups[0].component_key;
    assert!(save_project_match(&db, &imported.project_id, "missing", "stock", true).is_err());
    assert!(save_project_match(&db, &imported.project_id, key, "missing", true).is_err());
    save_project_match(&db, &imported.project_id, key, "stock", true).unwrap();
    drop(db);
    let db = Database::open(&path).unwrap();
    let saved: (String, bool) = db
        .connection()
        .query_row(
            "SELECT part_id,manual FROM project_matches WHERE project_id=?1",
            [&imported.project_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(saved, ("stock".into(), true));
    let stock: i64 = db
        .connection()
        .query_row("SELECT quantity FROM parts WHERE id='stock'", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(stock, 7);
    for manual in [false, true] {
        save_project_match(&db, &imported.project_id, key, "stock", manual).unwrap();
        remove_project_match(&db, &imported.project_id, key).unwrap();
        remove_project_match(&db, &imported.project_id, key).unwrap();
        let remaining: i64 = db
            .connection()
            .query_row(
                "SELECT COUNT(*) FROM project_matches WHERE project_id=?1",
                [&imported.project_id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(remaining, 0);
        let stock: i64 = db
            .connection()
            .query_row("SELECT quantity FROM parts WHERE id='stock'", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(stock, 7);
    }
    db.connection()
        .execute(
            "UPDATE parts SET deleted_at='2026-11-03' WHERE id='stock'",
            [],
        )
        .unwrap();
    assert!(save_project_match(&db, &imported.project_id, key, "stock", true).is_err());
}
