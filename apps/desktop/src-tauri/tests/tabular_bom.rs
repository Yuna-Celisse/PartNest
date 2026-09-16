use std::{
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicUsize, Ordering},
};

use partnest_desktop_lib::bom::tabular::inspect_tabular_bom;
use partnest_desktop_lib::bom::tabular::TabularError;
use partnest_desktop_lib::bom::types::ImportPreview;

static NEXT_TEMP: AtomicUsize = AtomicUsize::new(0);

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../fixtures/bom")
        .join(name)
}

fn parse_fixture(name: &str) -> partnest_desktop_lib::bom::types::NormalizedBomDto {
    match inspect_tabular_bom(fixture(name), None).expect("parse fixture") {
        ImportPreview::Ready(bom) => bom,
        ImportPreview::NeedsMapping { headers, .. } => {
            panic!("fixture unexpectedly needs mapping: {headers:?}")
        }
    }
}

#[test]
fn comma_and_tab_files_normalize_identically() {
    let comma = parse_fixture("comma-utf8.csv");
    let tab = parse_fixture("tab-utf16le.csv");
    assert_eq!(comma.groups, tab.groups);
    assert_eq!(comma.groups[0].designators, ["C1", "C2"]);
}

#[test]
fn explicit_quantity_must_equal_designator_count_without_side() {
    let path = temp_csv("Quantity,Designator,Footprint,Value\n1,\"C1,C2\",0603,100nF\n");
    let result = inspect_tabular_bom(&path, None);
    assert!(matches!(
        result,
        Err(TabularError::QuantityDesignatorMismatch {
            quantity: 1,
            designators: 2,
            ..
        })
    ));
    fs::remove_file(path).unwrap();
}

#[test]
fn explicit_quantity_must_equal_designator_count_with_side() {
    let path = temp_csv("Quantity,Designator,Footprint,Value,Side\n1,\"C1,C2\",0603,100nF,Top\n");
    let result = inspect_tabular_bom(&path, None);
    assert!(matches!(
        result,
        Err(TabularError::QuantityDesignatorMismatch {
            quantity: 1,
            designators: 2,
            ..
        })
    ));
    fs::remove_file(path).unwrap();
}

#[test]
fn invalid_explicit_quantity_never_falls_back_to_designators() {
    for quantity in ["0", "-1", "not-a-number"] {
        let path = temp_csv(&format!(
            "Quantity,Designator,Footprint,Value\n{quantity},C1,0603,100nF\n"
        ));
        let result = inspect_tabular_bom(&path, None);
        assert!(
            matches!(result, Err(TabularError::InvalidQuantity { .. })),
            "{quantity}"
        );
        fs::remove_file(path).unwrap();
    }
}

#[test]
fn absent_quantity_uses_designator_count() {
    let path = temp_csv("Designator,Footprint,Value\n\"C1,C2\",0603,100nF\n");
    let result = inspect_tabular_bom(&path, None).expect("parse designator count");
    let ImportPreview::Ready(bom) = result else {
        panic!("expected ready")
    };
    assert_eq!(bom.groups[0].quantity, 2);
    assert_eq!(bom.groups[0].designators, ["C1", "C2"]);
    fs::remove_file(path).unwrap();
}

#[test]
fn quoted_delimiters_and_unknown_fields_are_preserved() {
    let bom = parse_fixture("fields.xlsx");
    assert_eq!(bom.groups[0].quantity, 2);
    assert!(bom.groups[0].placements.is_empty());
    assert_eq!(bom.groups[0].extra_fields["Unknown Field"], "keep-me");
    assert_eq!(bom.groups[0].lcsc_code, "C100");
}

#[test]
fn missing_optional_columns_are_blank_and_designators_supply_quantity() {
    let path = std::env::temp_dir().join(format!("partnest-tabular-{}.csv", std::process::id()));
    fs::write(&path, "Designator,Footprint,Value\nC1,C0603,100nF\n").unwrap();
    let result = inspect_tabular_bom(&path, None).expect("parse optional fixture");
    let ImportPreview::Ready(bom) = result else {
        panic!("optional columns should not need mapping")
    };
    assert_eq!(bom.groups[0].quantity, 1);
    assert_eq!(bom.groups[0].manufacturer, "");
    assert_eq!(
        bom.groups[0].placements,
        Vec::<partnest_desktop_lib::bom::types::BomPlacementDto>::new()
    );
    fs::remove_file(path).unwrap();
}

#[test]
fn duplicate_field_headers_require_mapping_instead_of_guessing() {
    let path = std::env::temp_dir().join(format!("partnest-ambiguous-{}.csv", std::process::id()));
    fs::write(
        &path,
        "Quantity,Quantity,Designator,Footprint,Value\n1,1,C1,0603,10k\n",
    )
    .unwrap();
    let result = inspect_tabular_bom(&path, None).expect("inspect ambiguous fixture");
    assert!(matches!(result, ImportPreview::NeedsMapping { .. }));
    fs::remove_file(path).unwrap();
}

#[test]
fn supplied_mapping_resolves_custom_headers() {
    let path = std::env::temp_dir().join(format!("partnest-mapping-{}.csv", std::process::id()));
    fs::write(&path, "Count,Refs,Case,Param\n1,C1,C0603,10k\n").unwrap();
    let mapping = [
        ("quantity".to_owned(), "Count".to_owned()),
        ("designators".to_owned(), "Refs".to_owned()),
        ("package".to_owned(), "Case".to_owned()),
        ("value".to_owned(), "Param".to_owned()),
    ]
    .into_iter()
    .collect();
    let result = inspect_tabular_bom(&path, Some(&mapping)).expect("parse mapped fixture");
    let ImportPreview::Ready(bom) = result else {
        panic!("custom mapping should be ready")
    };
    assert_eq!(bom.groups[0].quantity, 1);
    fs::remove_file(path).unwrap();
}

#[test]
fn custom_mapping_collision_requires_mapping_review() {
    let path = temp_csv("Count,Refs,Case,Param\n2,C1,C0603,10k\n");
    let mapping = [
        ("quantity".to_owned(), "Count".to_owned()),
        ("designators".to_owned(), "Count".to_owned()),
        ("package".to_owned(), "Case".to_owned()),
        ("value".to_owned(), "Param".to_owned()),
    ]
    .into_iter()
    .collect();
    let result = inspect_tabular_bom(&path, Some(&mapping)).expect("inspect collision");
    assert!(matches!(result, ImportPreview::NeedsMapping { .. }));
    fs::remove_file(path).unwrap();
}

#[test]
fn utf8_bom_is_decoded() {
    let path = std::env::temp_dir().join(format!("partnest-utf8-bom-{}.csv", std::process::id()));
    let mut bytes = vec![0xef, 0xbb, 0xbf];
    bytes.extend_from_slice(b"Quantity,Designator,Footprint,Value\n1,C1,0603,10k\n");
    fs::write(&path, bytes).unwrap();
    assert!(matches!(
        inspect_tabular_bom(&path, None),
        Ok(ImportPreview::Ready(_))
    ));
    fs::remove_file(path).unwrap();
}

#[test]
fn invalid_utf8_and_malformed_utf16le_are_rejected() {
    let invalid_utf8 =
        std::env::temp_dir().join(format!("partnest-invalid-utf8-{}.csv", std::process::id()));
    fs::write(&invalid_utf8, [0xff, 0xfe, 0x00]).unwrap();
    assert!(matches!(
        inspect_tabular_bom(&invalid_utf8, None),
        Err(TabularError::UnsupportedEncoding)
    ));
    fs::remove_file(invalid_utf8).unwrap();

    let invalid_utf8 = std::env::temp_dir().join(format!(
        "partnest-invalid-utf8-raw-{}.csv",
        std::process::id()
    ));
    fs::write(&invalid_utf8, [0xff, 0x00, 0x61]).unwrap();
    assert!(matches!(
        inspect_tabular_bom(&invalid_utf8, None),
        Err(TabularError::UnsupportedEncoding)
    ));
    fs::remove_file(invalid_utf8).unwrap();

    let malformed_utf16 =
        std::env::temp_dir().join(format!("partnest-odd-utf16-{}.csv", std::process::id()));
    fs::write(&malformed_utf16, [0xff, 0xfe, b'Q']).unwrap();
    assert!(matches!(
        inspect_tabular_bom(&malformed_utf16, None),
        Err(TabularError::UnsupportedEncoding)
    ));
    fs::remove_file(malformed_utf16).unwrap();
}

#[test]
fn malformed_csv_is_rejected() {
    let path = temp_csv("Quantity,Designator,Footprint,Value\n1,\"C1,0603,10k\n");
    assert!(matches!(
        inspect_tabular_bom(&path, None),
        Err(TabularError::MalformedCsv { .. })
    ));
    fs::remove_file(path).unwrap();
}

#[test]
fn inconsistent_row_width_is_rejected_instead_of_selecting_a_wrong_delimiter() {
    let path = temp_csv("Quantity,Designator,Footprint,Value\n1,C1,0603,10k\n2,C2,0603\n");
    assert!(matches!(
        inspect_tabular_bom(&path, None),
        Err(TabularError::MalformedCsv { .. })
    ));
    fs::remove_file(path).unwrap();
}

#[test]
fn mapped_empty_designators_do_not_satisfy_an_explicit_quantity() {
    let path = temp_csv("Quantity,Designator,Footprint,Value\n2,,0603,10k\n");
    assert!(matches!(
        inspect_tabular_bom(&path, None),
        Err(TabularError::QuantityDesignatorMismatch {
            quantity: 2,
            designators: 0,
            ..
        })
    ));
    fs::remove_file(path).unwrap();
}

#[test]
fn delimiter_detection_scores_more_than_twenty_rows_and_quoted_commas() {
    let mut csv = String::from("Quantity,Designator,Footprint,Value\n");
    for row in 0..25 {
        csv.push_str(&format!("2,\"C{row},C{}\",0603,100nF\n", row + 100));
    }
    let path = temp_csv(&csv);
    let result = inspect_tabular_bom(&path, None).expect("parse long quoted csv");
    let ImportPreview::Ready(bom) = result else {
        panic!("expected ready")
    };
    assert_eq!(bom.groups[0].designators.len(), 50);
    fs::remove_file(path).unwrap();
}

#[test]
fn unsupported_nonblank_side_is_an_explicit_error() {
    let path = temp_csv("Quantity,Designator,Footprint,Value,Side\n1,C1,0603,10k,Middle\n");
    assert!(matches!(
        inspect_tabular_bom(&path, None),
        Err(TabularError::InvalidSide { .. })
    ));
    fs::remove_file(path).unwrap();
}

#[test]
fn side_column_creates_placements_without_inventing_side_when_absent() {
    let path = std::env::temp_dir().join(format!("partnest-side-{}.csv", std::process::id()));
    fs::write(&path, "数量,位号,封装,参数,板面\n1,C1,C0603,100nF,Bottom\n").unwrap();
    let result = inspect_tabular_bom(&path, None).expect("parse side fixture");
    let ImportPreview::Ready(bom) = result else {
        panic!("side fixture should be ready")
    };
    assert_eq!(bom.groups[0].placements.len(), 1);
    assert_eq!(
        bom.groups[0].placements[0].side,
        Some(partnest_desktop_lib::bom::types::BomSide::Bottom)
    );
    fs::remove_file(path).unwrap();
}

fn temp_csv(contents: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!(
        "partnest-tabular-{}-{}.csv",
        std::process::id(),
        NEXT_TEMP.fetch_add(1, Ordering::Relaxed)
    ));
    fs::write(&path, contents).unwrap();
    path
}
