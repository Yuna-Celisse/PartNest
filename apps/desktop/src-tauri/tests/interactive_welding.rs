use partnest_desktop_lib::bom::bridge::BridgeError;
use partnest_desktop_lib::bom::cache::{
    preview_interactive_bom, CacheError, CachedBomSession, InteractiveBomCache,
    InteractiveBomRuntime, SelectionError,
};
use partnest_desktop_lib::bom::projects::ProjectKind;
use partnest_desktop_lib::bom::types::BomSide;
use partnest_desktop_lib::db::Database;
use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};
use tempfile::tempdir;

fn fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../fixtures/bom/interactive-minimal.html")
}

fn cache_entries(dir: &Path) -> usize {
    fs::read_dir(dir)
        .map(|entries| entries.count())
        .unwrap_or(0)
}

/// Interactive sessions always carry a bridged cache copy.
fn cached_path(session: &CachedBomSession) -> PathBuf {
    session
        .cache_path
        .clone()
        .expect("interactive sessions have a cache file")
}

/// Import a project and open the welding workspace for it, which is the only
/// way an interactive canvas becomes usable.
fn open_project(
    cache: &InteractiveBomCache,
    source: &Path,
    name: Option<&str>,
) -> CachedBomSession {
    let imported = cache.import_project(source, name, None, None).unwrap();
    cache.open_project_welding(&imported.project_id).unwrap()
}

fn active_sessions(db: &Database) -> i64 {
    db.connection()
        .query_row(
            "SELECT COUNT(*) FROM welding_sessions WHERE status = 'active'",
            [],
            |row| row.get(0),
        )
        .unwrap()
}

#[test]
fn importing_a_project_caches_its_canvas_without_touching_the_session() {
    let root = tempdir().unwrap();
    let db = Database::open(root.path().join("partnest.db")).unwrap();
    let cache_dir = root.path().join("cache");

    let imported = InteractiveBomCache::new(&db, &cache_dir)
        .import_project(&fixture(), None, None, None)
        .unwrap();

    assert_eq!(imported.kind, ProjectKind::Interactive);
    assert_eq!(
        imported.name, "interactive-minimal",
        "no name falls back to the file stem"
    );
    assert_eq!(
        imported.normalized.groups[0].designators,
        ["R1", "R2", "R3"]
    );
    assert_eq!(
        cache_entries(&cache_dir),
        1,
        "the project keeps its own bridged canvas"
    );
    assert_eq!(
        active_sessions(&db),
        0,
        "importing must not cancel an in-progress session"
    );
}

#[test]
fn previewing_an_interactive_bom_caches_nothing_and_keeps_the_active_session() {
    let root = tempdir().unwrap();
    let db = Database::open(root.path().join("partnest.db")).unwrap();
    let cache_dir = root.path().join("cache");
    let cache = InteractiveBomCache::new(&db, &cache_dir);
    let active = open_project(&cache, &fixture(), None);
    let before = cache_entries(&cache_dir);

    let bom = preview_interactive_bom(&fixture(), None).unwrap();

    assert_eq!(bom.groups[0].designators, ["R1", "R2", "R3"]);
    assert_eq!(
        cache_entries(&cache_dir),
        before,
        "previewing must not write another cached copy"
    );
    assert_eq!(active_sessions(&db), 1);
    let restored = InteractiveBomCache::new(&db, &cache_dir)
        .restore_active_session()
        .unwrap()
        .expect("the opened session stays active");
    assert_eq!(restored.session_id, active.session_id);
    assert_eq!(restored.project_id, active.project_id);
}

#[test]
fn restoring_a_deleted_cache_file_asks_the_operator_to_import_the_project_again() {
    let root = tempdir().unwrap();
    let db = Database::open(root.path().join("partnest.db")).unwrap();
    let cache_dir = root.path().join("cache");
    let cache = InteractiveBomCache::new(&db, &cache_dir);
    let session = open_project(&cache, &fixture(), None);
    fs::remove_file(cached_path(&session)).unwrap();

    let error = InteractiveBomCache::new(&db, &cache_dir)
        .restore_active_session()
        .expect_err("a deleted cache copy must be reported, not silently ignored");
    assert!(matches!(error, CacheError::MissingCache(_)));
    assert!(error.user_message().contains("重新导入"));
}

#[test]
fn caches_atomic_copy_with_hash_name_and_bridge_marker() {
    let root = tempdir().unwrap();
    let db = Database::open(root.path().join("partnest.db")).unwrap();
    let source = fixture();
    let before = hash(&source);
    let cache = InteractiveBomCache::new(&db, root.path().join("cache"));
    let session = open_project(&cache, &source, None);
    assert_eq!(hash(&source), before);
    assert_eq!(session.sha256, before);
    assert_eq!(session.cache_name, format!("{before}.html"));
    assert!(cached_path(&session).is_file());
    let cached = fs::read_to_string(cached_path(&session)).unwrap();
    assert!(cached.contains("bridge-v1"));
    assert!(cached.contains("MutationObserver"));
    assert!(cached.contains("data-designator"));
    assert_eq!(session.normalized.groups[0].designators, ["R1", "R2", "R3"]);
    assert_eq!(
        session.normalized.groups[0]
            .placements
            .iter()
            .filter(|p| p.side == Some(BomSide::Bottom))
            .count(),
        1
    );
}

#[test]
fn resolves_only_current_token_known_unique_same_group_designators() {
    let root = tempdir().unwrap();
    let db = Database::open(root.path().join("partnest.db")).unwrap();
    let cache = InteractiveBomCache::new(&db, root.path().join("cache"));
    let session = open_project(&cache, &fixture(), None);
    let resolved = cache
        .resolve_bom_selection(&session.token, &["R1".into(), "R2".into()])
        .unwrap();
    assert_eq!(resolved.session_id, session.session_id);
    assert_eq!(resolved.designators, ["R1", "R2"]);
    assert_eq!(resolved.side, Some(BomSide::Top));
    assert!(matches!(
        cache.resolve_bom_selection(&session.token, &["R1".into(), "R3".into()]),
        Err(SelectionError::MixedSideSelection)
    ));
    assert!(matches!(
        cache.resolve_bom_selection("forged", &["R1".into()]),
        Err(BridgeError::InvalidToken)
    ));
    assert!(matches!(
        cache.resolve_bom_selection(&session.token, &["R1".into(), "R1".into()]),
        Err(SelectionError::DuplicateDesignator)
    ));
    assert!(matches!(
        cache.resolve_bom_selection(&session.token, &["Q9".into()]),
        Err(SelectionError::UnknownDesignator)
    ));
    db.connection()
        .execute(
            "UPDATE welding_sessions SET status = 'completed' WHERE id = ?1",
            [&session.session_id],
        )
        .unwrap();
    assert!(matches!(
        cache.resolve_bom_selection(&session.token, &["R1".into()]),
        Err(BridgeError::InactiveSession)
    ));
}

#[test]
fn bootstraps_current_token_and_repairs_tampered_cache() {
    let root = tempdir().unwrap();
    let db = Database::open(root.path().join("partnest.db")).unwrap();
    let cache = InteractiveBomCache::new(&db, root.path().join("cache"));
    let first = open_project(&cache, &fixture(), None);
    let content = fs::read_to_string(cached_path(&first)).unwrap();
    assert!(content.contains("__PARTNEST_BOM_BRIDGE_V1__"));
    assert!(content.contains(&first.token));
    let config_start =
        content.find("__PARTNEST_BOM_BRIDGE_V1__=").unwrap() + "__PARTNEST_BOM_BRIDGE_V1__=".len();
    let config_end = content[config_start..].find(";</script>").unwrap() + config_start;
    let config = &content[config_start..config_end];
    assert!(serde_json::from_str::<serde_json::Value>(config).is_ok());
    assert!(!config.contains("</script>"));
    fs::write(cached_path(&first), "tampered").unwrap();
    let second = open_project(&cache, &fixture(), None);
    let repaired = fs::read_to_string(cached_path(&second)).unwrap();
    assert!(repaired.contains(&second.token));
    assert!(repaired.contains("bridge-v1"));
    assert_ne!(repaired, "tampered");
}

#[test]
fn restores_active_session_with_new_token_after_runtime_restart() {
    let root = tempdir().unwrap();
    let db = Database::open(root.path().join("partnest.db")).unwrap();
    let cache_dir = root.path().join("cache");
    let first = open_project(&InteractiveBomCache::new(&db, &cache_dir), &fixture(), None);
    let restarted = InteractiveBomCache::new(&db, &cache_dir);
    let restored = restarted.restore_active_session().unwrap().unwrap();
    assert_eq!(restored.session_id, first.session_id);
    assert_ne!(restored.token, first.token);
    assert!(matches!(
        restarted.resolve_bom_selection(&first.token, &["R1".into()]),
        Err(BridgeError::InvalidToken)
    ));
    assert_eq!(
        restarted
            .resolve_bom_selection(&restored.token, &["R1".into()])
            .unwrap()
            .side,
        Some(BomSide::Top)
    );
}

#[test]
fn does_not_restore_an_inactive_session_after_runtime_restart() {
    let root = tempdir().unwrap();
    let db = Database::open(root.path().join("partnest.db")).unwrap();
    let cache_dir = root.path().join("cache");
    let first = open_project(&InteractiveBomCache::new(&db, &cache_dir), &fixture(), None);
    db.connection()
        .execute(
            "UPDATE welding_sessions SET status = 'completed' WHERE id = ?1",
            [&first.session_id],
        )
        .unwrap();

    let restarted = InteractiveBomCache::new(&db, &cache_dir);
    assert!(restarted.restore_active_session().unwrap().is_none());
}

#[test]
fn invalidating_runtime_rejects_the_previous_bom_authorization() {
    let root = tempdir().unwrap();
    let db = Database::open(root.path().join("partnest.db")).unwrap();
    let runtime = InteractiveBomRuntime::new(root.path().join("cache"));
    let imported = runtime
        .import_project(&db, &fixture(), None, None, None)
        .unwrap();
    let session = runtime
        .open_project_welding(&db, &imported.project_id)
        .unwrap();

    runtime.invalidate_active_session().unwrap();

    assert!(matches!(
        runtime.resolve_bom_selection(&db, &session.token, &["R1".into()]),
        Err(BridgeError::InactiveSession)
    ));
}

#[test]
fn resolver_authoritatively_rejects_oversized_direct_inputs() {
    let root = tempdir().unwrap();
    let db = Database::open(root.path().join("partnest.db")).unwrap();
    let cache = InteractiveBomCache::new(&db, root.path().join("cache"));
    let session = open_project(&cache, &fixture(), None);
    assert!(matches!(
        cache.resolve_bom_selection(&"t".repeat(129), &["R1".into()]),
        Err(BridgeError::TokenTooLong)
    ));
    assert!(matches!(
        cache.resolve_bom_selection(&session.token, &vec!["R1".into(); 513]),
        Err(BridgeError::TooManyDesignators)
    ));
    assert!(matches!(
        cache.resolve_bom_selection(&session.token, &["".into()]),
        Err(BridgeError::EmptyDesignator)
    ));
}

#[test]
fn rejects_untrusted_cache_name_that_escapes_cache_directory() {
    let root = tempdir().unwrap();
    let db = Database::open(root.path().join("partnest.db")).unwrap();
    let cache_dir = root.path().join("cache");
    let cache = InteractiveBomCache::new(&db, &cache_dir);
    let session = open_project(&cache, &fixture(), None);
    db.connection()
        .execute(
            "UPDATE projects SET cache_name = ?1 WHERE id = ?2",
            rusqlite::params!["..\\outside.html", session.project_id],
        )
        .unwrap();
    let restarted = InteractiveBomCache::new(&db, &cache_dir);
    assert!(matches!(
        restarted.restore_active_session(),
        Err(CacheError::TamperedCache(_))
    ));
}

#[test]
fn reimporting_the_same_content_updates_the_project_and_keeps_one_session() {
    let root = tempdir().unwrap();
    let db = Database::open(root.path().join("partnest.db")).unwrap();
    let cache_dir = root.path().join("cache");
    let cache = InteractiveBomCache::new(&db, &cache_dir);
    let first = open_project(&cache, &fixture(), Some("First name"));
    let second = open_project(&cache, &fixture(), Some("Second name"));
    assert_eq!(first.session_id, second.session_id);
    assert_eq!(first.project_id, second.project_id);
    assert_eq!(
        db.connection()
            .query_row(
                "SELECT name FROM projects WHERE id = ?1",
                [&second.project_id],
                |row| row.get::<_, String>(0),
            )
            .unwrap(),
        "Second name"
    );
    assert_eq!(
        db.connection()
            .query_row("SELECT COUNT(*) FROM projects", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        1,
        "the same BOM content stays one project"
    );

    let alternate = root.path().join("alternate.html");
    let mut source = fs::read_to_string(fixture()).unwrap();
    source.push_str("\n<!-- alternate source -->\n");
    fs::write(&alternate, source).unwrap();
    let third = open_project(&cache, &alternate, None);
    assert_ne!(third.session_id, second.session_id);
    assert_eq!(active_sessions(&db), 1);
    assert_eq!(
        db.connection()
            .query_row(
                "SELECT status FROM welding_sessions WHERE id = ?1",
                [&second.session_id],
                |row| row.get::<_, String>(0)
            )
            .unwrap(),
        "cancelled"
    );
}

#[test]
fn companion_metadata_is_cached_and_restored_without_schema_changes() {
    let root = tempdir().unwrap();
    let db = Database::open(root.path().join("partnest.db")).unwrap();
    let source = root.path().join("board.html");
    fs::write(
        &source,
        r#"<script>window.files = {"bom_merge":{"data":{"comp_info":{"C1":{"Name":"R"}},"designator_info":{"top":[{"des":"R1","lc_code":""}],"bottom":[]}}}};</script>"#,
    )
    .unwrap();
    let companion = root.path().join("board.csv");
    fs::write(
        &companion,
        "Quantity,Comment,Designator,Footprint,Value,Manufacturer Part,Manufacturer,Supplier Part\n1,10k,R1,0603,10k,R-10K,Acme,C999\n",
    )
    .unwrap();
    let cache_dir = root.path().join("cache");
    let cache = InteractiveBomCache::new(&db, &cache_dir);
    let imported = cache
        .import_project(&source, None, None, Some(&companion))
        .unwrap();
    let first = cache.open_project_welding(&imported.project_id).unwrap();
    assert_eq!(first.normalized.groups[0].lcsc_code, "C999");
    assert!(fs::read_to_string(cached_path(&first))
        .unwrap()
        .contains("data-partnest-companion=\"bom-v1\""));

    let restarted = InteractiveBomCache::new(&db, &cache_dir);
    let restored = restarted.restore_active_session().unwrap().unwrap();
    assert_eq!(restored.normalized.groups[0].lcsc_code, "C999");
}

#[test]
fn opening_a_project_reissues_the_cached_canvas_token() {
    let root = tempdir().unwrap();
    let db = Database::open(root.path().join("partnest.db")).unwrap();
    let cache_dir = root.path().join("cache");
    let runtime = InteractiveBomRuntime::new(&cache_dir);
    let imported = runtime
        .import_project(&db, &fixture(), None, None, None)
        .unwrap();
    let activated = runtime
        .open_project_welding(&db, &imported.project_id)
        .unwrap();

    let reopened = runtime
        .open_project_welding(&db, &activated.project_id)
        .unwrap();
    assert_eq!(reopened.kind, ProjectKind::Interactive);
    assert_eq!(reopened.session_id, activated.session_id, "resumed");
    assert_ne!(reopened.token, activated.token, "each open is fresh");
    assert!(
        fs::read_to_string(cached_path(&reopened))
            .unwrap()
            .contains(&reopened.token),
        "the canvas bootstrap must carry the re-issued token"
    );
    assert_eq!(active_sessions(&db), 1);
    let resolved = runtime
        .resolve_bom_selection(&db, &reopened.token, &["R1".into()])
        .unwrap();
    assert_eq!(resolved.side, Some(BomSide::Top));

    let stale = runtime
        .resolve_bom_selection(&db, &activated.token, &["R1".into()])
        .expect_err("the previous token must stop working");
    assert!(matches!(stale, BridgeError::InvalidToken));
}

#[test]
fn a_table_project_opens_from_its_snapshot_without_any_cache_copy() {
    let root = tempdir().unwrap();
    let db = Database::open(root.path().join("partnest.db")).unwrap();
    let cache_dir = root.path().join("cache");
    let source = root.path().join("board.csv");
    fs::copy(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../fixtures/bom/comma-utf8.csv"),
        &source,
    )
    .unwrap();
    let cache = InteractiveBomCache::new(&db, &cache_dir);
    let imported = cache.import_project(&source, None, None, None).unwrap();
    assert_eq!(imported.kind, ProjectKind::Tabular);
    assert_eq!(
        cache_entries(&cache_dir),
        0,
        "a table import never writes a canvas"
    );

    let session = cache.open_project_welding(&imported.project_id).unwrap();
    assert!(session.cache_path.is_none());
    assert!(!session.normalized.groups.is_empty());
}

#[test]
fn an_unsupported_extension_is_rejected_before_anything_is_stored() {
    let root = tempdir().unwrap();
    let db = Database::open(root.path().join("partnest.db")).unwrap();
    let source = root.path().join("board.txt");
    fs::write(&source, "Quantity,Designator\n1,R1\n").unwrap();
    let cache = InteractiveBomCache::new(&db, root.path().join("cache"));

    let error = cache
        .import_project(&source, None, None, None)
        .expect_err("a txt file is not a BOM format PartNest reads");
    assert!(matches!(error, CacheError::Project(_)));
    assert!(error.user_message().contains("无法导入该 BOM"));
    assert_eq!(active_sessions(&db), 0);
    assert_eq!(
        db.connection()
            .query_row("SELECT COUNT(*) FROM projects", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
}

fn hash(path: &Path) -> String {
    hex::encode(Sha256::digest(fs::read(path).unwrap()))
}
