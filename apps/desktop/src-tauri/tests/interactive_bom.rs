use partnest_desktop_lib::bom::bridge::{decode_selection_message, BridgeError};
use partnest_desktop_lib::bom::interactive_html::{parse_interactive_html, InteractiveHtmlError};
use partnest_desktop_lib::bom::types::{BomGroupDto, BomPlacementDto, BomSide, NormalizedBomDto};
use std::path::{Path, PathBuf};
use std::{collections::BTreeMap, fs};

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../fixtures/bom")
        .join(name)
}

#[test]
fn parses_top_and_bottom_designators_without_mutating_source() {
    let path = fixture("interactive-minimal.html");
    let before = sha256(&path);
    let bom = parse_interactive_html(&path).expect("parse fixture");
    assert_eq!(sha256(&path), before);
    assert_eq!(bom.groups.len(), 1);
    assert_eq!(bom.groups[0].designators, ["R1", "R2", "R3"]);
    assert_eq!(bom.groups[0].quantity, 3);
    assert_eq!(
        bom.groups[0]
            .placements
            .iter()
            .filter(|p| p.side == Some(BomSide::Top))
            .count(),
        2
    );
    assert_eq!(
        bom.groups[0]
            .placements
            .iter()
            .filter(|p| p.side == Some(BomSide::Bottom))
            .count(),
        1
    );
}

#[test]
fn rejects_missing_core_sections_without_partial_groups() {
    let path = std::env::temp_dir().join(format!(
        "partnest-invalid-interactive-{}",
        std::process::id()
    ));
    fs::write(
        &path,
        r#"<script>window.files = {"bom_merge":{"data":{"comp_info":{}}}};</script>"#,
    )
    .unwrap();
    assert!(matches!(
        parse_interactive_html(&path),
        Err(InteractiveHtmlError::UnsupportedInteractiveBom(_))
    ));
    fs::remove_file(path).unwrap();
}

#[test]
fn scanner_ignores_decoys_comments_and_all_javascript_string_forms() {
    let source = r#"
      const a = "window.files = {\"bad\":true}";
      const b = `window.files = {\"bad\":true}`;
      // window.files = {"bad":true}
      /* window.files = {"bad":true} */
      window.files /* comment */ = {
        "bom_merge": {"data": {"comp_info": {"C1": {"Name": "x" /* inline */}},
        "designator_info": {"top": [{"des": "R1", "lc_code": "C1"}], "bottom": []}}}
      };
    "#;
    let bom =
        partnest_desktop_lib::bom::interactive_html::parse_interactive_html_text(source, "scanner")
            .unwrap();
    assert_eq!(bom.groups[0].designators, ["R1"]);
}

#[test]
fn scanner_survives_markup_quotes_and_unterminated_attributes_before_the_data_script() {
    // Real exports surround the bundle with prose, comments, and markup. A quote
    // that has no partner must never abort the scan, because the assignment it
    // hides is the only way to read the BOM at all.
    const BODY: &str = r#""bom_merge": {"data": {"comp_info": {"C1": {"Name": "x"}}, "designator_info": {"top": [{"des": "R1", "lc_code": "C1"}], "bottom": []}}}"#;
    for markup in [
        r#"<html><body><p>It's a board</p>"#,
        r#"<html><body><p>It's a "board", don't touch</p>"#,
        r#"<html><!-- don't --><body>"#,
        r#"<html><body><div title='unclosed>"#,
        r#"<html><body><script>var re = /a"b/g;</script>"#,
    ] {
        let source = format!("{markup}<script>window.files = {{{BODY}}};</script></body></html>");
        let bom = partnest_desktop_lib::bom::interactive_html::parse_interactive_html_text(
            &source, "markup",
        )
        .unwrap_or_else(|error| panic!("{markup} must still parse: {error}"));
        assert_eq!(bom.groups[0].designators, ["R1"], "markup: {markup}");
    }
}

#[test]
fn scanner_keeps_comment_and_brace_like_text_inside_string_values() {
    // Real exports put URLs, comment-looking prose and braces inside field
    // values. Stripping them as syntax would corrupt the document instead of
    // cleaning it, so string content must survive both the scan and the strip.
    let source = r#"
      window.files = {
        "bom_merge": {"data": {
          "comp_info": {"C1": {"Name": "10k", "note": "https://example.com/a//b /* x */ {y} don't"}},
          "designator_info": {"top": [{"des": "R1", "lc_code": "C1"}], "bottom": []}
        }}
      };
    "#;
    let bom = partnest_desktop_lib::bom::interactive_html::parse_interactive_html_text(
        source,
        "strings-in-values",
    )
    .unwrap_or_else(|error| panic!("string values must survive scanning: {error}"));
    assert_eq!(bom.groups[0].designators, ["R1"]);
}

#[test]
fn scanner_rejects_candidates_without_bom_merge_and_documents_a_missing_assignment() {
    let decoy =
        r#"<html><body><script>var decoy = window.files = {"bad": true};</script></body></html>"#;
    let error = partnest_desktop_lib::bom::interactive_html::parse_interactive_html_text(
        decoy,
        "decoy-only",
    )
    .expect_err("an object without bom_merge is not a BOM");
    assert!(error.to_string().contains("no bom_merge section"));

    let empty = "<html><body><p>nothing here</p></body></html>";
    let error = partnest_desktop_lib::bom::interactive_html::parse_interactive_html_text(
        empty,
        "no-assignment",
    )
    .expect_err("a document without the assignment cannot be read");
    assert!(error.to_string().contains("window.files object is missing"));
}

#[test]
fn derives_component_key_from_a_unique_component_name_when_lc_code_is_absent() {
    let source = r#"
      window.files = {
        "bom_merge": {"data": {
          "comp_info": {"manual-m3": {"Name": "M3", "value": "", "Supplier Footprint": "Hole"}},
          "designator_info": {"top": [{"des": "H1", "cm": "M3"}], "bottom": []}
        }}
      };
    "#;
    let bom = partnest_desktop_lib::bom::interactive_html::parse_interactive_html_text(
        source,
        "no-lc-code",
    )
    .expect("derive component key");
    assert_eq!(bom.groups.len(), 1);
    assert_eq!(bom.groups[0].component_key, "manual-m3");
    assert_eq!(bom.groups[0].designators, ["H1"]);
}

#[test]
fn ambiguous_component_name_falls_back_to_the_designator_identity() {
    let source = r#"
      window.files = {
        "bom_merge": {"data": {
          "comp_info": {
            "r0603": {"Name": "10k", "value": "10k"},
            "r0805": {"Name": "10k", "value": "10k"}
          },
          "designator_info": {"top": [{"des": "R1", "cm": "10k", "ft_name": "R0603"}], "bottom": []}
        }}
      };
    "#;
    let bom = partnest_desktop_lib::bom::interactive_html::parse_interactive_html_text(
        source,
        "ambiguous-no-lc-code",
    )
    .expect("an ambiguous name must not block the import");
    assert_eq!(bom.groups.len(), 1);
    assert_eq!(bom.groups[0].component_key, "part:10k+R0603");
    assert_eq!(bom.groups[0].package, "R0603");
    assert_eq!(bom.groups[0].designators, ["R1"]);
}

#[test]
fn parts_without_a_supplier_code_keep_their_own_groups() {
    // EasyEDA leaves `lc_code` empty for parts that have no LCSC entry and keys
    // comp_info by the composite "C<code>,<value>" string otherwise.
    let source = r#"
      window.files = {
        "bom_merge": {"data": {
          "comp_info": {
            "C1525,100nF": {"Name": "100nF", "Supplier Part": "C1525"},
            "": {"Name": "ZX-XH2.54-8PZZ", "Supplier Part": "C7429638"}
          },
          "designator_info": {"top": [
            {"des": "C2", "cm": "100nF", "lc_code": "C1525,100nF", "ft_name": "0402"},
            {"des": "H2", "cm": "HDR-M_2.54_1x8P", "lc_code": "", "ft_name": "HDR-TH_8P-P2.54-V-M"},
            {"des": "hole1", "cm": "M3", "lc_code": "", "ft_name": "m3 125x300"}
          ], "bottom": []}
        }}
      };
    "#;
    let bom = partnest_desktop_lib::bom::interactive_html::parse_interactive_html_text(
        source,
        "no-supplier-code",
    )
    .expect("parts without a supplier code must import");
    assert_eq!(
        bom.groups.len(),
        3,
        "each distinct part keeps its own group"
    );
    let header = bom
        .groups
        .iter()
        .find(|group| group.name == "HDR-M_2.54_1x8P")
        .expect("header group");
    assert_eq!(
        header.component_key,
        "part:HDR-M_2.54_1x8P+HDR-TH_8P-P2.54-V-M"
    );
    assert_eq!(header.package, "HDR-TH_8P-P2.54-V-M");
    assert!(header.lcsc_code.is_empty());
    assert_eq!(header.designators, ["H2"]);
}

#[test]
fn companion_csv_metadata_completes_html_entries_with_empty_lc_code() {
    let source = r#"
      window.files = {
        "bom_merge": {"data": {
          "comp_info": {"C1": {"Name": "10k", "value": "10k"}},
          "designator_info": {"top": [
            {"des": "R1", "lc_code": "C1"},
            {"des": "U1", "lc_code": "", "cm": "GD32F303CCT6"}
          ], "bottom": []}
        }}
      };
    "#;
    let companion = NormalizedBomDto {
        source_name: "board.csv".into(),
        groups: vec![
            companion_group("lcsc:C1", "R1", "10k", "", ""),
            companion_group("lcsc:C116151", "U1", "GD32F303CCT6", "LQFP-48", "C116151"),
        ],
    };
    let bom =
        partnest_desktop_lib::bom::interactive_html::parse_interactive_html_text_with_companion(
            source,
            "board.html",
            Some(&companion),
        )
        .expect("combine companion CSV");
    let group = bom
        .groups
        .iter()
        .find(|group| group.component_key == "lcsc:C116151")
        .expect("CSV-backed group");
    assert_eq!(group.name, "GD32F303CCT6");
    assert_eq!(group.package, "LQFP-48");
    assert_eq!(group.lcsc_code, "C116151");
    assert_eq!(group.placements[0].side, Some(BomSide::Top));
}

#[test]
fn bridge_rejects_oversized_messages_and_designators() {
    let many = (0..513)
        .map(|i| format!("\"R{i}\""))
        .collect::<Vec<_>>()
        .join(",");
    let too_many =
        format!(r#"{{"type":"partnest:bom-selection","token":"t","designators":[{many}]}}"#);
    assert!(matches!(
        decode_selection_message(&too_many),
        Err(BridgeError::TooManyDesignators)
    ));
    let too_long = format!(
        r#"{{"type":"partnest:bom-selection","token":"t","designators":["{}"]}}"#,
        "R".repeat(64)
    );
    assert!(decode_selection_message(&too_long).is_ok());
    let much_too_long = format!(
        r#"{{"type":"partnest:bom-selection","token":"t","designators":["{}"]}}"#,
        "R".repeat(65)
    );
    assert!(matches!(
        decode_selection_message(&much_too_long),
        Err(BridgeError::DesignatorTooLong)
    ));
    assert!(matches!(
        decode_selection_message(&"x".repeat(65 * 1024)),
        Err(BridgeError::MessageTooLarge)
    ));
    assert!(matches!(
        decode_selection_message(
            r#"{"type":"partnest:bom-selection","token":"","designators":["R1"]}"#
        ),
        Err(BridgeError::EmptyToken)
    ));
}

fn sha256(path: &Path) -> String {
    use sha2::{Digest, Sha256};
    hex::encode(Sha256::digest(fs::read(path).unwrap()))
}

fn companion_group(
    component_key: &str,
    designator: &str,
    name: &str,
    package: &str,
    lcsc_code: &str,
) -> BomGroupDto {
    BomGroupDto {
        component_key: component_key.into(),
        name: name.into(),
        value: String::new(),
        package: package.into(),
        manufacturer: String::new(),
        mpn: name.into(),
        lcsc_code: lcsc_code.into(),
        quantity: 1,
        designators: vec![designator.into()],
        placements: vec![BomPlacementDto {
            designator: designator.into(),
            side: None,
            component_key: component_key.into(),
        }],
        extra_fields: BTreeMap::new(),
    }
}

#[test]
fn a_part_without_a_supplier_code_still_carries_its_model_as_the_mpn() {
    // EasyEDA leaves lc_code empty for unlisted parts and shows `cm` in the
    // 器件型号 column, so that value has to stay matchable as an MPN.
    let source = r#"
      window.files = {
        "bom_merge": {"data": {
          "comp_info": {"": {"Name": "SOME-OTHER-PART"}},
          "designator_info": [{"top": [
            {"des": "H2", "cm": "HDR-M_2.54_1x8P", "lc_code": "", "ft_name": "HDR-TH_8P-P2.54-V-M"}
          ], "bottom": []}]
        }}
      };
    "#;
    let bom = partnest_desktop_lib::bom::interactive_html::parse_interactive_html_text(
        source,
        "no-supplier-code",
    )
    .expect("parses");
    assert_eq!(bom.groups.len(), 1);
    let group = &bom.groups[0];
    assert_eq!(
        group.component_key,
        "part:HDR-M_2.54_1x8P+HDR-TH_8P-P2.54-V-M"
    );
    assert_eq!(group.name, "HDR-M_2.54_1x8P");
    assert_eq!(group.mpn, "HDR-M_2.54_1x8P");
    assert_eq!(group.package, "HDR-TH_8P-P2.54-V-M");
    assert!(group.lcsc_code.is_empty(), "no supplier code was invented");
}
