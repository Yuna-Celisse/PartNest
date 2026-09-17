use std::{
    collections::{BTreeMap, BTreeSet},
    fmt, fs,
    path::Path,
};

use calamine::{open_workbook_auto, Data, Reader};
use csv::{ReaderBuilder, StringRecord};
use encoding_rs::UTF_16LE;

use super::types::{
    BomGroupDto, BomPlacementDto, BomSide, FieldMapping, ImportPreview, NormalizedBomDto,
};

const NORMALIZED_FIELDS: [&str; 9] = [
    "quantity",
    "designators",
    "package",
    "value",
    "name",
    "mpn",
    "manufacturer",
    "lcsc_code",
    "side",
];
#[derive(Debug)]
pub enum TabularError {
    Io(std::io::Error),
    Csv(csv::Error),
    Workbook(String),
    UnsupportedEncoding,
    MalformedCsv {
        delimiter: char,
        error: String,
    },
    InvalidQuantity {
        row: usize,
        value: String,
    },
    QuantityDesignatorMismatch {
        row: usize,
        quantity: i64,
        designators: usize,
    },
    InvalidSide {
        row: usize,
        value: String,
    },
    InvalidRecord {
        row: usize,
        reason: String,
    },
}

impl fmt::Display for TabularError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(f, "I/O error: {error}"),
            Self::Csv(error) => write!(f, "CSV error: {error}"),
            Self::Workbook(error) => write!(f, "workbook error: {error}"),
            Self::UnsupportedEncoding => write!(f, "unsupported text encoding"),
            Self::MalformedCsv { delimiter, error } => {
                write!(f, "malformed CSV using {delimiter:?} delimiter: {error}")
            }
            Self::InvalidQuantity { row, value } => {
                write!(f, "invalid quantity at row {row}: {value:?}")
            }
            Self::QuantityDesignatorMismatch {
                row,
                quantity,
                designators,
            } => write!(
                f,
                "quantity/designator mismatch at row {row}: quantity {quantity}, designators {designators}"
            ),
            Self::InvalidSide { row, value } => {
                write!(f, "unsupported board side at row {row}: {value:?}")
            }
            Self::InvalidRecord { row, reason } => write!(f, "invalid BOM row {row}: {reason}"),
        }
    }
}

impl std::error::Error for TabularError {}
impl From<std::io::Error> for TabularError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}
impl From<csv::Error> for TabularError {
    fn from(error: csv::Error) -> Self {
        Self::Csv(error)
    }
}

#[derive(Debug, Clone)]
struct Table {
    headers: Vec<String>,
    records: Vec<Vec<String>>,
}

/// Inspect a CSV or XLSX without mutating the source file. `mapping` is a
/// per-import override keyed by normalized field name.
pub fn inspect_tabular_bom(
    path: impl AsRef<Path>,
    mapping: Option<&FieldMapping>,
) -> Result<ImportPreview, TabularError> {
    let path = path.as_ref();
    let table = read_table(path)?;
    let (resolved, ambiguous) = resolve_mapping(&table.headers, mapping);
    if ambiguous || !has_required_mapping(&resolved) {
        return Ok(ImportPreview::NeedsMapping {
            headers: table.headers,
            suggestions: resolved,
        });
    }
    let source_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default()
        .to_owned();
    Ok(ImportPreview::Ready(normalize_table(
        source_name,
        table,
        &resolved,
    )?))
}

/// Parse a tabular BOM when its headers can be resolved without user input.
/// Interactive-BOM companion files use this strict path because they must be
/// validated against the selected HTML before a cache session is created.
pub fn parse_tabular_bom(
    path: impl AsRef<Path>,
    mapping: Option<&FieldMapping>,
) -> Result<NormalizedBomDto, TabularError> {
    let path = path.as_ref();
    let table = read_table(path)?;
    let (resolved, ambiguous) = resolve_mapping(&table.headers, mapping);
    if ambiguous || !has_required_mapping(&resolved) {
        return Err(TabularError::InvalidRecord {
            row: 1,
            reason: "field mapping is required".into(),
        });
    }
    let source_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default()
        .to_owned();
    normalize_table(source_name, table, &resolved)
}

/// Pick the reader from the file name: spreadsheets go to calamine, which
/// detects xls versus xlsx by content once it is handed the file.
fn read_table(path: &Path) -> Result<Table, TabularError> {
    match path
        .extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.to_ascii_lowercase())
        .as_deref()
    {
        Some("xls") | Some("xlsx") => read_xlsx(path),
        _ => read_csv(path),
    }
}

fn read_csv(path: &Path) -> Result<Table, TabularError> {
    let bytes = fs::read(path)?;
    let text = decode_text(&bytes)?;
    let mut candidates = Vec::new();
    let mut malformed = Vec::new();
    for delimiter in *b",\t" {
        let mut reader = ReaderBuilder::new()
            .has_headers(false)
            .delimiter(delimiter)
            .from_reader(text.as_bytes());
        match reader.records().collect::<Result<Vec<_>, _>>() {
            Ok(rows) => match table_from_records(rows) {
                Ok(Some(table)) => {
                    let recognized = table
                        .headers
                        .iter()
                        .filter(|header| canonical_header(header).is_some())
                        .count();
                    let width = table.headers.len();
                    candidates.push((recognized, width, delimiter, table));
                }
                Ok(None) => {}
                Err((reason, recognized)) => {
                    malformed.push((delimiter as char, reason, recognized));
                }
            },
            Err(error) => {
                let recognized = ReaderBuilder::new()
                    .has_headers(false)
                    .delimiter(delimiter)
                    .from_reader(text.as_bytes())
                    .records()
                    .next()
                    .and_then(Result::ok)
                    .map(|header| {
                        header
                            .iter()
                            .filter(|value| canonical_header(value).is_some())
                            .count()
                    })
                    .unwrap_or(0);
                malformed.push((delimiter as char, error.to_string(), recognized));
            }
        }
    }
    candidates.sort_by_key(|candidate| (candidate.0, candidate.1, candidate.2 == b','));
    if let Some((recognized, _, _delimiter, table)) = candidates.pop() {
        if malformed
            .iter()
            .any(|(_, _, malformed_recognized)| *malformed_recognized > recognized)
        {
            let (delimiter, error, _) = malformed
                .into_iter()
                .max_by_key(|(_, _, recognized)| *recognized)
                .expect("malformed candidate exists");
            return Err(TabularError::MalformedCsv { delimiter, error });
        }
        return Ok(table);
    }
    if let Some((delimiter, error, _)) = malformed.into_iter().next() {
        return Err(TabularError::MalformedCsv { delimiter, error });
    }
    Err(TabularError::InvalidRecord {
        row: 1,
        reason: "missing header".into(),
    })
}

fn read_xlsx(path: &Path) -> Result<Table, TabularError> {
    let mut workbook =
        open_workbook_auto(path).map_err(|error| TabularError::Workbook(error.to_string()))?;
    let sheet_names = workbook.sheet_names().to_owned();
    for sheet_name in sheet_names {
        let range = workbook
            .worksheet_range(&sheet_name)
            .map_err(|error| TabularError::Workbook(error.to_string()))?;
        let rows: Vec<Vec<String>> = range
            .rows()
            .map(|row| row.iter().map(data_to_string).collect())
            .collect();
        if let Ok(Some(table)) = table_from_rows(rows) {
            return Ok(table);
        }
    }
    Err(TabularError::InvalidRecord {
        row: 1,
        reason: "workbook has no non-empty worksheet".into(),
    })
}

fn data_to_string(value: &Data) -> String {
    match value {
        Data::Empty => String::new(),
        _ => value.to_string(),
    }
}

fn decode_text(bytes: &[u8]) -> Result<String, TabularError> {
    if bytes.starts_with(&[0xff, 0xfe]) {
        if !(bytes.len() - 2).is_multiple_of(2) {
            return Err(TabularError::UnsupportedEncoding);
        }
        let (text, _, had_errors) = UTF_16LE.decode(&bytes[2..]);
        if had_errors {
            return Err(TabularError::UnsupportedEncoding);
        }
        return Ok(text.into_owned());
    }
    if bytes.starts_with(&[0xef, 0xbb, 0xbf]) {
        return String::from_utf8(bytes[3..].to_vec())
            .map_err(|_| TabularError::UnsupportedEncoding);
    }
    String::from_utf8(bytes.to_vec()).map_err(|_| TabularError::UnsupportedEncoding)
}

fn table_from_records(rows: Vec<StringRecord>) -> Result<Option<Table>, (String, usize)> {
    table_from_rows(
        rows.into_iter()
            .map(|row| row.iter().map(str::to_owned).collect())
            .collect(),
    )
}

fn table_from_rows(rows: Vec<Vec<String>>) -> Result<Option<Table>, (String, usize)> {
    let mut iter = rows
        .into_iter()
        .filter(|row| row.iter().any(|value| !value.trim().is_empty()));
    let Some(headers) = iter.next() else {
        return Ok(None);
    };
    let headers = headers
        .into_iter()
        .map(|value| value.trim().to_owned())
        .collect::<Vec<_>>();
    if headers.is_empty() || headers.iter().all(String::is_empty) {
        return Ok(None);
    }
    let records: Vec<Vec<String>> = iter.collect();
    if let Some((row_index, row)) = records
        .iter()
        .enumerate()
        .find(|(_, row)| row.len() != headers.len())
    {
        let recognized = headers
            .iter()
            .filter(|header| canonical_header(header).is_some())
            .count();
        return Err((
            format!(
                "inconsistent row width at row {}: expected {}, got {}",
                row_index + 2,
                headers.len(),
                row.len()
            ),
            recognized,
        ));
    }
    Ok(Some(Table { headers, records }))
}

fn resolve_mapping(headers: &[String], supplied: Option<&FieldMapping>) -> (FieldMapping, bool) {
    if let Some(mapping) = supplied {
        let mut result = FieldMapping::new();
        let mut used = BTreeSet::new();
        let mut ambiguous = false;
        for (field, source) in mapping {
            if !NORMALIZED_FIELDS.contains(&field.as_str()) {
                continue;
            }
            let Some(index) = headers
                .iter()
                .position(|header| header.eq_ignore_ascii_case(source.trim()))
            else {
                ambiguous = true;
                continue;
            };
            if !used.insert(index) {
                ambiguous = true;
            }
            result.insert(field.clone(), headers[index].clone());
        }
        return (result, ambiguous);
    }
    let mut result = FieldMapping::new();
    let mut used = BTreeSet::new();
    let mut ambiguous = false;
    for header in headers {
        if let Some(field) = canonical_header(header) {
            if result.contains_key(field) || !used.insert(header) {
                ambiguous = true;
            } else {
                result.insert(field.to_owned(), header.clone());
            }
        }
    }
    (result, ambiguous)
}

fn has_required_mapping(mapping: &FieldMapping) -> bool {
    let has_count = mapping.contains_key("quantity") || mapping.contains_key("designators");
    let has_identity = mapping.contains_key("lcsc_code")
        || mapping.contains_key("mpn")
        || (mapping.contains_key("package")
            && (mapping.contains_key("value") || mapping.contains_key("name")));
    has_count && has_identity
}

fn canonical_header(header: &str) -> Option<&'static str> {
    let normalized = header.trim().to_ascii_lowercase();
    match normalized.as_str() {
        "quantity" | "qty" | "count" | "数量" => Some("quantity"),
        "designator" | "designators" | "reference" | "references" | "refdes" | "位号" => {
            Some("designators")
        }
        "footprint" | "package" | "pcb footprint" | "封装" => Some("package"),
        "value" | "parameters" | "param" | "参数" => Some("value"),
        "comment" | "name" | "description" | "注释" | "名称" => Some("name"),
        "manufacturer part" | "manufacturer part number" | "mpn" | "mfr part" | "制造商型号" => {
            Some("mpn")
        }
        "manufacturer" | "mfr" | "maker" | "制造商" => Some("manufacturer"),
        "supplier part"
        | "supplier part number"
        | "lcsc"
        | "lcsc code"
        | "supplier code"
        | "立创商城编号"
        | "供应商料号" => Some("lcsc_code"),
        "side" | "layer" | "board side" | "板面" | "层" => Some("side"),
        _ => None,
    }
}

fn normalize_table(
    source_name: String,
    table: Table,
    mapping: &FieldMapping,
) -> Result<NormalizedBomDto, TabularError> {
    let index: BTreeMap<&str, usize> = mapping
        .iter()
        .filter_map(|(field, source)| {
            table
                .headers
                .iter()
                .position(|header| header == source)
                .map(|idx| (field.as_str(), idx))
        })
        .collect();
    let mut groups: BTreeMap<String, BomGroupDto> = BTreeMap::new();
    for (offset, row) in table.records.iter().enumerate() {
        if row.iter().all(|value| value.trim().is_empty()) {
            continue;
        }
        let value = |field: &str| {
            index
                .get(field)
                .and_then(|idx| row.get(*idx))
                .map(|value| value.trim().to_owned())
                .unwrap_or_default()
        };
        let designators = value("designators");
        let designator_list = split_designators(&designators);
        let quantity = if index.contains_key("quantity") {
            let raw_quantity = value("quantity");
            let quantity =
                parse_quantity(&raw_quantity).ok_or_else(|| TabularError::InvalidQuantity {
                    row: offset + 2,
                    value: raw_quantity.clone(),
                })?;
            if index.contains_key("designators") && quantity as usize != designator_list.len() {
                return Err(TabularError::QuantityDesignatorMismatch {
                    row: offset + 2,
                    quantity,
                    designators: designator_list.len(),
                });
            }
            quantity
        } else if !designator_list.is_empty() {
            designator_list.len() as i64
        } else {
            return Err(TabularError::InvalidRecord {
                row: offset + 2,
                reason: "quantity or designators is required".into(),
            });
        };
        let name = value("name");
        let component_key = component_key(
            &value("lcsc_code"),
            &value("mpn"),
            &value("value"),
            &value("package"),
            &name,
        );
        if component_key.is_empty() {
            return Err(TabularError::InvalidRecord {
                row: offset + 2,
                reason: "missing component identity".into(),
            });
        }
        let raw_side = value("side");
        let side = if raw_side.is_empty() {
            None
        } else {
            Some(
                parse_side(&raw_side).ok_or_else(|| TabularError::InvalidSide {
                    row: offset + 2,
                    value: raw_side.clone(),
                })?,
            )
        };
        let group = groups
            .entry(component_key.clone())
            .or_insert_with(|| BomGroupDto {
                component_key: component_key.clone(),
                name: name.clone(),
                value: value("value"),
                package: value("package"),
                manufacturer: value("manufacturer"),
                mpn: value("mpn"),
                lcsc_code: value("lcsc_code"),
                quantity: 0,
                designators: Vec::new(),
                placements: Vec::new(),
                extra_fields: BTreeMap::new(),
            });
        group.quantity += quantity;
        group.designators.extend(designator_list.iter().cloned());
        if side.is_some() {
            group
                .placements
                .extend(
                    designator_list
                        .into_iter()
                        .map(|designator| BomPlacementDto {
                            designator,
                            side: side.clone(),
                            component_key: component_key.clone(),
                        }),
                );
        }
        for (idx, header) in table.headers.iter().enumerate() {
            if canonical_header(header).is_none() {
                let field_value = row.get(idx).map(String::as_str).unwrap_or_default().trim();
                if !field_value.is_empty() {
                    group
                        .extra_fields
                        .insert(header.clone(), field_value.to_owned());
                }
            }
        }
    }
    Ok(NormalizedBomDto {
        source_name,
        groups: groups.into_values().collect(),
    })
}

fn parse_quantity(raw: &str) -> Option<i64> {
    raw.trim()
        .parse::<i64>()
        .ok()
        .filter(|quantity| *quantity > 0)
}
fn split_designators(raw: &str) -> Vec<String> {
    raw.split(|character: char| character == ',' || character == ';' || character.is_whitespace())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .collect()
}
fn parse_side(raw: &str) -> Option<BomSide> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "top" | "topside" | "front" => Some(BomSide::Top),
        "bottom" | "bottomside" | "back" => Some(BomSide::Bottom),
        _ => None,
    }
}
fn component_key(lcsc: &str, mpn: &str, value: &str, package: &str, name: &str) -> String {
    if !lcsc.is_empty() {
        format!("lcsc:{lcsc}")
    } else if !mpn.is_empty() {
        format!("mpn:{mpn}")
    } else if !package.is_empty() && (!value.is_empty() || !name.is_empty()) {
        format!("value:{value}|package:{package}|name:{name}")
    } else {
        String::new()
    }
}
