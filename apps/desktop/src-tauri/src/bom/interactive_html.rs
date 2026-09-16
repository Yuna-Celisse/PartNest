//! Parser for EasyEDA interactive BOM exports.

use super::types::{BomGroupDto, BomPlacementDto, BomSide, NormalizedBomDto};
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet},
    fmt, fs,
    path::Path,
};

#[derive(Debug)]
pub enum InteractiveHtmlError {
    Io(std::io::Error),
    InvalidJson(String),
    UnsupportedInteractiveBom(String),
    TooLarge(usize),
}

/// Upper bound for a single interactive BOM export. Real EasyEDA exports embed
/// the whole PCB drawing in one HTML file, so the parser must not be handed an
/// arbitrarily large buffer while the database lock is held by the caller.
pub const MAX_INTERACTIVE_BOM_BYTES: usize = 64 * 1024 * 1024;

impl fmt::Display for InteractiveHtmlError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(f, "I/O error: {error}"),
            Self::InvalidJson(error) => write!(f, "invalid interactive BOM JSON: {error}"),
            Self::UnsupportedInteractiveBom(reason) => {
                write!(f, "unsupported interactive BOM: {reason}")
            }
            Self::TooLarge(size) => write!(
                f,
                "interactive BOM is too large: {size} bytes (limit {MAX_INTERACTIVE_BOM_BYTES})"
            ),
        }
    }
}
impl std::error::Error for InteractiveHtmlError {}

impl InteractiveHtmlError {
    /// Reader-facing text for the UI; `Display` keeps the English form for logs
    /// and test assertions.
    pub fn user_message(&self) -> String {
        match self {
            Self::Io(error) => format!("读取 BOM 文件失败：{error}"),
            Self::InvalidJson(error) => format!("交互式 BOM 数据解析失败：{error}"),
            Self::UnsupportedInteractiveBom(reason) => format!("不支持的交互式 BOM：{reason}"),
            Self::TooLarge(_) => format!(
                "交互式 BOM 文件过大，最大支持 {} MiB",
                MAX_INTERACTIVE_BOM_BYTES / (1024 * 1024)
            ),
        }
    }
}
impl From<std::io::Error> for InteractiveHtmlError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}

/// Parse an interactive HTML file without modifying it.
pub fn parse_interactive_html(
    path: impl AsRef<Path>,
) -> Result<NormalizedBomDto, InteractiveHtmlError> {
    let path = path.as_ref();
    let source = fs::read_to_string(path)?;
    let source_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default()
        .to_owned();
    parse_interactive_html_text(&source, source_name)
}

/// Parse a source string. This is public so callers can inspect a copied input
/// without creating a temporary source file.
pub fn parse_interactive_html_text(
    source: &str,
    source_name: impl Into<String>,
) -> Result<NormalizedBomDto, InteractiveHtmlError> {
    parse_interactive_html_text_with_companion(source, source_name, None)
}

/// Parse an interactive HTML file with optional metadata from its paired
/// tabular BOM. The paired BOM supplies identities for placements whose HTML
/// entry has an empty component key.
pub fn parse_interactive_html_text_with_companion(
    source: &str,
    source_name: impl Into<String>,
    companion: Option<&NormalizedBomDto>,
) -> Result<NormalizedBomDto, InteractiveHtmlError> {
    if source.len() > MAX_INTERACTIVE_BOM_BYTES {
        return Err(InteractiveHtmlError::TooLarge(source.len()));
    }
    let files = select_window_files_object(source)?;
    let merge = nested_json(files.get("bom_merge"), "bom_merge")?;
    let data = nested_json(merge.get("data"), "bom_merge.data")?;
    let comp_info = data
        .get("comp_info")
        .and_then(Value::as_object)
        .ok_or_else(|| unsupported("comp_info must be an object"))?;
    let designator_info = data
        .get("designator_info")
        .ok_or_else(|| unsupported("designator_info is missing"))?;
    if comp_info.is_empty() && companion.is_none() {
        return Err(unsupported("comp_info is empty"));
    }
    let companion_by_designator = companion.map(companion_designator_index).transpose()?;

    let mut groups: BTreeMap<String, BomGroupDto> = BTreeMap::new();
    let mut seen = BTreeSet::new();
    let mut placements = 0usize;
    for (side, entry) in designator_entries(designator_info)? {
        let list = entry
            .as_array()
            .ok_or_else(|| unsupported("designator_info side must be an array"))?;
        for item in list {
            let item = item
                .as_object()
                .ok_or_else(|| unsupported("designator entry must be an object"))?;
            let designator = first_text(item, &["des", "designator", "reference", "ref"])
                .ok_or_else(|| unsupported("designator entry has no designator"))?;
            if !seen.insert(designator.clone()) {
                return Err(unsupported("duplicate designator"));
            }
            let explicit_component_key = first_text(item, &["lc_code", "component_key", "bom_key"]);
            let companion_group = if explicit_component_key.is_none() {
                companion_by_designator
                    .as_ref()
                    .and_then(|index| index.get(&designator))
            } else {
                None
            };
            let component_key = explicit_component_key.unwrap_or_else(|| {
                companion_group
                    .map(|group| group.component_key.clone())
                    .unwrap_or_default()
            });
            let component_key = if component_key.is_empty() {
                resolve_component_key(item, comp_info)
            } else {
                component_key
            };
            let metadata = if companion_group.is_none() {
                comp_info.get(&component_key).and_then(Value::as_object)
            } else {
                None
            };
            let group =
                groups
                    .entry(component_key.clone())
                    .or_insert_with(|| match companion_group {
                        Some(group) => group_from_companion(component_key.clone(), group),
                        None => match metadata {
                            Some(metadata) => group_from_metadata(component_key.clone(), metadata),
                            None => group_from_designator_entry(component_key.clone(), item),
                        },
                    });
            group.quantity += 1;
            group.designators.push(designator.clone());
            group.placements.push(BomPlacementDto {
                designator,
                side: Some(side.clone()),
                component_key,
            });
            placements += 1;
        }
    }
    if placements == 0 || groups.is_empty() {
        return Err(unsupported("designator_info has no placements"));
    }
    if let Some(companion_by_designator) = companion_by_designator {
        let companion_designators = companion_by_designator.into_keys().collect::<BTreeSet<_>>();
        if seen != companion_designators {
            return Err(unsupported(
                "companion CSV designators do not match interactive BOM",
            ));
        }
    }
    Ok(NormalizedBomDto {
        source_name: source_name.into(),
        groups: groups.into_values().collect(),
    })
}

fn companion_designator_index(
    companion: &NormalizedBomDto,
) -> Result<BTreeMap<String, BomGroupDto>, InteractiveHtmlError> {
    let mut result = BTreeMap::new();
    for group in &companion.groups {
        for designator in &group.designators {
            if result.insert(designator.clone(), group.clone()).is_some() {
                return Err(unsupported("companion CSV has duplicate designator"));
            }
        }
    }
    if result.is_empty() {
        return Err(unsupported("companion CSV has no designators"));
    }
    Ok(result)
}

fn group_from_companion(component_key: String, companion: &BomGroupDto) -> BomGroupDto {
    BomGroupDto {
        component_key,
        name: companion.name.clone(),
        value: companion.value.clone(),
        package: companion.package.clone(),
        manufacturer: companion.manufacturer.clone(),
        mpn: companion.mpn.clone(),
        lcsc_code: companion.lcsc_code.clone(),
        quantity: 0,
        designators: Vec::new(),
        placements: Vec::new(),
        extra_fields: companion.extra_fields.clone(),
    }
}

fn group_from_metadata(
    component_key: String,
    metadata: &serde_json::Map<String, Value>,
) -> BomGroupDto {
    BomGroupDto {
        component_key,
        name: metadata_text(metadata, &["Name", "name", "Description", "description"]),
        value: metadata_text(metadata, &["value", "Value", "Parameters", "parameters"]),
        package: metadata_text(
            metadata,
            &[
                "Supplier Footprint",
                "supplier_footprint",
                "Footprint",
                "footprint",
                "lc_pkg",
            ],
        ),
        manufacturer: metadata_text(metadata, &["Manufacturer", "manufacturer"]),
        mpn: metadata_text(
            metadata,
            &[
                "Manufacturer Part",
                "manufacturer_part",
                "MPN",
                "mpn",
                "lc_model",
            ],
        ),
        lcsc_code: metadata_text(
            metadata,
            &["Supplier Part", "supplier_part", "LCSC", "lcsc_code"],
        ),
        quantity: 0,
        designators: Vec::new(),
        placements: Vec::new(),
        extra_fields: BTreeMap::new(),
    }
}

/// Pick the real `window.files` assignment out of an HTML document.
///
/// The document is untrusted markup, so every `window.files` occurrence is
/// treated as a candidate and only accepted once it parses and carries the
/// `bom_merge` section. Locating candidates never skips over quotes, because a
/// stray apostrophe in markup text (`<p>It's a board</p>`, `<!-- don't -->`, an
/// unterminated attribute) must not hide the assignment that follows it.
fn select_window_files_object(source: &str) -> Result<Value, InteractiveHtmlError> {
    let candidates = window_files_objects(source);
    let mut last_json_error = None;
    let mut saw_object = false;
    for object in candidates {
        match serde_json::from_str(&strip_js_comments(object)) {
            Ok(files @ Value::Object(_)) => {
                saw_object = true;
                if files.get("bom_merge").is_some() {
                    return Ok(files);
                }
            }
            Ok(_) => {}
            Err(error) => last_json_error = Some(error),
        }
    }
    if let Some(error) = last_json_error {
        return Err(json_error(error));
    }
    if saw_object {
        return Err(unsupported("window.files object has no bom_merge section"));
    }
    Err(unsupported("window.files object is missing"))
}

fn window_files_objects(source: &str) -> Vec<&str> {
    let bytes = source.as_bytes();
    let marker = b"window.files";
    let mut result = Vec::new();
    let mut index = 0;
    while let Some(offset) = find_bytes(&bytes[index..], marker) {
        let at = index + offset;
        let before_ok = at == 0 || !is_identifier(bytes[at - 1]);
        let after_ok = !bytes
            .get(at + marker.len())
            .is_some_and(|byte| is_identifier(*byte));
        index = at + marker.len();
        if !before_ok || !after_ok {
            continue;
        }
        // A malformed tail (unterminated string or unbalanced braces) only
        // discards this candidate; later occurrences are still inspected.
        let Some(cursor) = skip_space_and_comments(bytes, at + marker.len()) else {
            continue;
        };
        if bytes.get(cursor) != Some(&b'=') {
            continue;
        }
        let Some(cursor) = skip_space_and_comments(bytes, cursor + 1) else {
            continue;
        };
        if bytes.get(cursor) != Some(&b'{') {
            continue;
        }
        if let Some(end) = scan_balanced_object(bytes, cursor) {
            result.push(&source[cursor..=end]);
        }
    }
    result
}

fn find_bytes(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

fn scan_balanced_object(bytes: &[u8], start: usize) -> Option<usize> {
    let mut depth = 0usize;
    let mut index = start;
    while index < bytes.len() {
        match bytes[index] {
            b'"' | b'\'' | b'`' => index = skip_js_string(bytes, index)?,
            b'/' if bytes.get(index + 1) == Some(&b'/') => index = skip_line_comment(bytes, index),
            b'/' if bytes.get(index + 1) == Some(&b'*') => {
                index = skip_block_comment(bytes, index)?
            }
            b'{' => {
                depth += 1;
                index += 1;
            }
            b'}' => {
                depth = depth.checked_sub(1)?;
                if depth == 0 {
                    return Some(index);
                }
                index += 1;
            }
            _ => index += 1,
        }
    }
    None
}

fn skip_js_string(bytes: &[u8], start: usize) -> Option<usize> {
    let quote = *bytes.get(start)?;
    let mut index = start + 1;
    while index < bytes.len() {
        if bytes[index] == b'\\' {
            index += 2;
            continue;
        }
        if bytes[index] == quote {
            return Some(index + 1);
        }
        index += 1;
    }
    None
}

fn skip_line_comment(bytes: &[u8], start: usize) -> usize {
    bytes[start..]
        .iter()
        .position(|byte| *byte == b'\n')
        .map(|offset| start + offset + 1)
        .unwrap_or(bytes.len())
}

fn skip_block_comment(bytes: &[u8], start: usize) -> Option<usize> {
    let end = bytes[start + 2..]
        .windows(2)
        .position(|pair| pair == b"*/")?;
    Some(start + 2 + end + 2)
}

fn skip_space_and_comments(bytes: &[u8], mut index: usize) -> Option<usize> {
    loop {
        while bytes.get(index).is_some_and(u8::is_ascii_whitespace) {
            index += 1;
        }
        if bytes.get(index) == Some(&b'/') && bytes.get(index + 1) == Some(&b'/') {
            index = skip_line_comment(bytes, index);
        } else if bytes.get(index) == Some(&b'/') && bytes.get(index + 1) == Some(&b'*') {
            index = skip_block_comment(bytes, index)?;
        } else {
            return Some(index);
        }
    }
}

fn is_identifier(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'$'
}

fn strip_js_comments(source: &str) -> String {
    let bytes = source.as_bytes();
    let mut output = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'"' | b'\'' | b'`' => {
                let end = skip_js_string(bytes, index).unwrap_or(bytes.len());
                output.extend_from_slice(&bytes[index..end]);
                index = end;
            }
            b'/' if bytes.get(index + 1) == Some(&b'/') => {
                index = skip_line_comment(bytes, index);
                output.push(b'\n');
            }
            b'/' if bytes.get(index + 1) == Some(&b'*') => {
                index = skip_block_comment(bytes, index).unwrap_or(bytes.len());
                output.push(b' ');
            }
            byte => {
                output.push(byte);
                index += 1;
            }
        }
    }
    String::from_utf8(output).unwrap_or_default()
}

/// A JSON object that is either borrowed from the parsed document or freshly
/// decoded from a nested JSON string. `bom_merge` and `bom_merge.data` are
/// normally strings inside the export, so this avoids cloning the whole
/// component tree twice per import.
enum JsonRef<'a> {
    Borrowed(&'a Value),
    Owned(Value),
}

impl JsonRef<'_> {
    fn get(&self, key: &str) -> Option<&Value> {
        match self {
            Self::Borrowed(value) => value.get(key),
            Self::Owned(value) => value.get(key),
        }
    }
}

fn nested_json<'a>(
    value: Option<&'a Value>,
    name: &str,
) -> Result<JsonRef<'a>, InteractiveHtmlError> {
    let value = value.ok_or_else(|| unsupported(format!("{name} is missing")))?;
    match value {
        Value::String(text) => serde_json::from_str(text)
            .map(JsonRef::Owned)
            .map_err(json_error),
        Value::Object(_) => Ok(JsonRef::Borrowed(value)),
        _ => Err(unsupported(format!(
            "{name} must be a JSON object or string"
        ))),
    }
}

fn designator_entries(value: &Value) -> Result<Vec<(BomSide, &Value)>, InteractiveHtmlError> {
    let mut result = Vec::new();
    let roots: Vec<&Value> = value
        .as_array()
        .map(|items| items.iter().collect())
        .unwrap_or_else(|| vec![value]);
    for root in roots {
        let object = root
            .as_object()
            .ok_or_else(|| unsupported("designator_info must contain objects"))?;
        for (name, side) in [("top", BomSide::Top), ("bottom", BomSide::Bottom)] {
            if let Some(items) = object.get(name) {
                result.push((side, items));
            }
        }
    }
    if result.is_empty() {
        return Err(unsupported("designator_info has no top or bottom arrays"));
    }
    Ok(result)
}

fn first_text(object: &serde_json::Map<String, Value>, keys: &[&str]) -> Option<String> {
    keys.iter()
        .find_map(|key| object.get(*key).and_then(value_text))
}

fn resolve_component_key(
    entry: &serde_json::Map<String, Value>,
    comp_info: &serde_json::Map<String, Value>,
) -> String {
    // Callers only reach this path after the explicit keys came back empty, so
    // the remaining options are a unique name match or the entry's own identity.
    let component_name = first_text(entry, &["cm", "component_name", "name", "value"]);
    if let Some(component_name) = &component_name {
        let candidates = comp_info
            .iter()
            .filter_map(|(key, value)| {
                let metadata = value.as_object()?;
                let matches = ["Name", "name", "value", "Value"]
                    .iter()
                    .filter_map(|field| metadata.get(*field).and_then(value_text))
                    .any(|text| text.eq_ignore_ascii_case(component_name));
                matches.then(|| key.clone())
            })
            .collect::<Vec<_>>();
        if let [key] = candidates.as_slice() {
            return key.clone();
        }
    }
    // EasyEDA leaves `lc_code` empty for parts with no supplier code, and then
    // comp_info has no entry for them. Keying on name plus footprint keeps
    // distinct parts apart instead of merging them into one anonymous group.
    let package = first_text(entry, &["ft_name", "package", "footprint"]).unwrap_or_default();
    match component_name {
        Some(name) => format!("part:{name}+{package}"),
        None => format!(
            "part:{}",
            first_text(entry, &["deviceUuid", "uuid", "id"]).unwrap_or(package)
        ),
    }
}

/// Group metadata for a placement that comp_info does not describe.
fn group_from_designator_entry(
    component_key: String,
    entry: &serde_json::Map<String, Value>,
) -> BomGroupDto {
    // EasyEDA renders `cm` in the 器件型号 column for parts without a supplier
    // code, so it doubles as the manufacturer part number for matching.
    let component_name = metadata_text(entry, &["cm", "component_name", "name", "value"]);
    let mpn = metadata_text(
        entry,
        &["Manufacturer Part", "manufacturer_part", "MPN", "mpn"],
    );
    BomGroupDto {
        component_key,
        name: component_name.clone(),
        value: metadata_text(entry, &["value", "Value"]),
        package: metadata_text(entry, &["ft_name", "package", "footprint"]),
        manufacturer: metadata_text(entry, &["Manufacturer", "manufacturer"]),
        mpn: if mpn.is_empty() { component_name } else { mpn },
        lcsc_code: metadata_text(
            entry,
            &["Supplier Part", "supplier_part", "LCSC", "lcsc_code"],
        ),
        quantity: 0,
        designators: Vec::new(),
        placements: Vec::new(),
        extra_fields: BTreeMap::new(),
    }
}

fn metadata_text(object: &serde_json::Map<String, Value>, keys: &[&str]) -> String {
    first_text(object, keys).unwrap_or_default()
}
fn value_text(value: &Value) -> Option<String> {
    match value {
        Value::String(text) if !text.trim().is_empty() => Some(text.trim().to_owned()),
        Value::Number(number) => Some(number.to_string()),
        _ => None,
    }
}
fn unsupported(reason: impl Into<String>) -> InteractiveHtmlError {
    InteractiveHtmlError::UnsupportedInteractiveBom(reason.into())
}
fn json_error(error: serde_json::Error) -> InteractiveHtmlError {
    InteractiveHtmlError::InvalidJson(error.to_string())
}
