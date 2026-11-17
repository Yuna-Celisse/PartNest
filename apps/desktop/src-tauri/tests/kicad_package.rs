use partnest_desktop_lib::{
    bom::{kicad_package, projects::load_snapshot},
    db::Database,
};
use std::{fs, io::Write, path::Path};
use tempfile::tempdir;
use zip::{write::SimpleFileOptions, ZipWriter};

fn archive(path: &Path, entries: &[(&str, &[u8])]) {
    let mut zip = ZipWriter::new(fs::File::create(path).unwrap());
    for (name, bytes) in entries {
        zip.start_file(*name, SimpleFileOptions::default()).unwrap();
        zip.write_all(bytes).unwrap();
    }
    zip.finish().unwrap();
}

#[test]
fn persists_package_and_analysis_without_the_original_zip_and_deduplicates() {
    let root = tempdir().unwrap();
    let path = root.path().join("board.zip");
    archive(
        &path,
        &[
            ("board/board.kicad_pro", b"{}"),
            (
                "board/BOM.csv",
                b"designators,quantity,value,package,lcsc_code\nR1,1,10k,0603,C123\n",
            ),
            ("gerber/board-F_Cu.gbr", b"G04 gerber*"),
        ],
    );
    let db = Database::open(root.path().join("db.sqlite")).unwrap();
    let result =
        kicad_package::commit(&db, kicad_package::prepare(&path, None).unwrap(), None).unwrap();
    let second =
        kicad_package::commit(&db, kicad_package::prepare(&path, None).unwrap(), None).unwrap();
    assert_eq!(result.project_id, second.project_id);
    fs::remove_file(path).unwrap();
    drop(db);
    let db = Database::open(root.path().join("db.sqlite")).unwrap();
    let manifest = kicad_package::manifest(&db, &result.project_id)
        .unwrap()
        .unwrap();
    assert_eq!(manifest.files.len(), 3);
    assert!(manifest.files.iter().any(|f| f.kind == "Gerber"));
    assert_eq!(
        load_snapshot(&db, &result.project_id).unwrap().groups[0].lcsc_code,
        "C123"
    );
    let stored: i64 = db
        .connection()
        .query_row(
            "SELECT length(archive) FROM project_packages WHERE project_id=?1",
            [&result.project_id],
            |r| r.get(0),
        )
        .unwrap();
    assert!(stored > 0);
}

#[test]
fn root_project_takes_precedence_over_historical_copies() {
    let root = tempdir().unwrap();
    let path = root.path().join("board.zip");
    archive(
        &path,
        &[
            ("board.kicad_pro", b"{}"),
            (".history/board.kicad_pro", b"{}"),
            ("fabrication/source/board.kicad_pro", b"{}"),
            (
                "BOM.csv",
                b"designators,quantity,value,package\nR1,1,10k,0603\n",
            ),
        ],
    );
    let db = Database::open(root.path().join("db.sqlite")).unwrap();
    let result =
        kicad_package::commit(&db, kicad_package::prepare(&path, None).unwrap(), None).unwrap();
    let manifest = kicad_package::manifest(&db, &result.project_id)
        .unwrap()
        .unwrap();
    assert!(manifest
        .warnings
        .iter()
        .any(|w| w.contains("主工程 board.kicad_pro")));
    assert_eq!(manifest.bom_source, "BOM.csv");
}

#[test]
fn rejects_traversal_multiple_projects_and_gerber_only_archives() {
    let root = tempdir().unwrap();
    let path = root.path().join("unsafe.zip");
    archive(&path, &[("../escape.csv", b"bad")]);
    assert!(kicad_package::prepare(&path, None)
        .err()
        .unwrap()
        .contains("路径"));
    assert!(!root.path().join("escape.csv").exists());
    archive(&path, &[("one.kicad_pro", b"{}"), ("two.kicad_pro", b"{}")]);
    assert!(kicad_package::prepare(&path, None)
        .err()
        .unwrap()
        .contains("多个"));
    archive(&path, &[("board.gbr", b"G04*")]);
    assert!(kicad_package::prepare(&path, None)
        .err()
        .unwrap()
        .contains("未找到"));
}

#[test]
#[ignore = "Set PARTNEST_KICAD_TEST_ZIP to a real KiCad archive and install kicad-cli"]
fn exports_real_kicad_3d_model() {
    let bytes = fs::read(std::env::var("PARTNEST_KICAD_TEST_ZIP").unwrap()).unwrap();
    let model = kicad_package::export_model(&bytes).unwrap();
    assert_eq!(&model[..4], b"glTF");
    let json_size = u32::from_le_bytes(model[12..16].try_into().unwrap()) as usize;
    let json: serde_json::Value = serde_json::from_slice(&model[20..20 + json_size]).unwrap();
    assert!(json["meshes"].as_array().unwrap().len() > 1);
    println!(
        "GLB: {} bytes, {} meshes",
        model.len(),
        json["meshes"].as_array().unwrap().len()
    );
}

#[test]
#[ignore = "Set PARTNEST_KICAD_TEST_ZIP to a real KiCad archive and install kicad-cli"]
fn imports_real_kicad_archive_with_board_exports() {
    let path = std::env::var("PARTNEST_KICAD_TEST_ZIP").unwrap();
    let root = tempdir().unwrap();
    let db = Database::open(root.path().join("db.sqlite")).unwrap();
    let result = kicad_package::commit(
        &db,
        kicad_package::prepare(Path::new(&path), None).unwrap(),
        None,
    )
    .unwrap();
    let manifest = kicad_package::manifest(&db, &result.project_id)
        .unwrap()
        .unwrap();
    assert!(!result.normalized.groups.is_empty());
    assert!(
        manifest
            .front_svg
            .as_ref()
            .is_some_and(|s| s.contains("<svg")),
        "{:?}",
        manifest.warnings
    );
    assert!(manifest
        .back_svg
        .as_ref()
        .is_some_and(|s| s.contains("<svg")));
    assert!(!manifest.positions.is_empty(), "{:?}", manifest.warnings);
    let mut legacy = result.normalized.clone();
    for group in &mut legacy.groups {
        group.placements.clear();
    }
    db.connection()
        .execute(
            "UPDATE projects SET normalized_json=?1 WHERE id=?2",
            rusqlite::params![serde_json::to_string(&legacy).unwrap(), result.project_id],
        )
        .unwrap();
    let cache =
        partnest_desktop_lib::bom::cache::InteractiveBomCache::new(&db, root.path().join("cache"));
    let session = cache.open_project_welding(&result.project_id).unwrap();
    let expected = manifest
        .positions
        .iter()
        .filter(|p| {
            session
                .normalized
                .groups
                .iter()
                .any(|g| g.designators.contains(&p.reference))
        })
        .count();
    assert_eq!(
        session
            .normalized
            .groups
            .iter()
            .map(|g| g.placements.len())
            .sum::<usize>(),
        expected
    );
    if let Some(bottom) = manifest.positions.iter().find(|p| {
        p.side == "bottom"
            && session
                .normalized
                .groups
                .iter()
                .any(|g| g.designators.contains(&p.reference))
    }) {
        let resolved = cache
            .resolve_bom_selection(&session.token, std::slice::from_ref(&bottom.reference))
            .unwrap();
        assert_eq!(
            resolved.side,
            Some(partnest_desktop_lib::bom::types::BomSide::Bottom)
        );
    }

    assert!(manifest
        .positions
        .iter()
        .any(|p| p.reference.starts_with('R')));
}
