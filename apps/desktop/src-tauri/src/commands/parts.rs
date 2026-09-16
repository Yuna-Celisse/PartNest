use super::{lock_error, normalize_slot, optional_text, validate_name, CommandError};
use crate::db::{new_id, utc_now, Database};
use rusqlite::{params, OptionalExtension, Transaction};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{sync::Mutex, time::Duration};
use tauri::State;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PartInput {
    pub name: String,
    pub category: String,
    pub package: String,
    pub manufacturer: String,
    pub mpn: String,
    pub lcsc_code: String,
    pub quantity: i64,
    pub box_id: Option<i64>,
    pub slot: Option<String>,
    pub note: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PartView {
    pub id: String,
    pub name: String,
    pub category: Option<String>,
    pub package: Option<String>,
    pub manufacturer: Option<String>,
    pub mpn: Option<String>,
    pub lcsc_code: Option<String>,
    pub quantity: i64,
    pub box_id: Option<i64>,
    pub slot: Option<String>,
    pub note: Option<String>,
    pub version: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LcscPartInfo {
    pub lcsc_code: String,
    pub name: String,
    pub category: String,
    pub package: String,
    pub manufacturer: String,
    pub mpn: String,
}

fn normalize_lcsc_code(value: &str) -> Result<String, CommandError> {
    let code = value.trim().to_ascii_uppercase();
    if code.len() < 2
        || !code.starts_with('C')
        || !code[1..].bytes().all(|byte| byte.is_ascii_digit())
    {
        return Err(CommandError::Validation("LCSC ID 格式无效".into()));
    }
    Ok(code)
}

fn normalize_input_lcsc_code(value: &str) -> Result<Option<String>, CommandError> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    normalize_lcsc_code(trimmed).map(Some)
}

fn cached_lcsc_info(db: &Database, code: &str) -> Result<Option<LcscPartInfo>, CommandError> {
    db.connection().query_row(
        "SELECT lcsc_code, name, category, package, manufacturer, mpn FROM lcsc_cache WHERE lcsc_code = ?1",
        [code],
        |row| Ok(LcscPartInfo { lcsc_code: row.get(0)?, name: row.get(1)?, category: row.get(2)?, package: row.get(3)?, manufacturer: row.get(4)?, mpn: row.get(5)? }),
    ).optional().map_err(CommandError::from)
}

fn cache_lcsc_info(db: &Database, info: &LcscPartInfo) -> Result<(), CommandError> {
    db.connection().execute(
        "INSERT INTO lcsc_cache (lcsc_code, name, category, package, manufacturer, mpn, fetched_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7) ON CONFLICT(lcsc_code) DO UPDATE SET name = excluded.name, category = excluded.category, package = excluded.package, manufacturer = excluded.manufacturer, mpn = excluded.mpn, fetched_at = excluded.fetched_at",
        params![info.lcsc_code, info.name, info.category, info.package, info.manufacturer, info.mpn, utc_now()],
    )?;
    Ok(())
}

pub fn lookup_lcsc_with_fetcher<F>(
    db: &Database,
    value: &str,
    fetcher: F,
) -> Result<LcscPartInfo, CommandError>
where
    F: FnOnce(&str) -> Result<LcscPartInfo, String>,
{
    let code = normalize_lcsc_code(value)?;
    match fetcher(&code) {
        Ok(mut info) => {
            info.lcsc_code = code;
            cache_lcsc_info(db, &info)?;
            Ok(info)
        }
        Err(network_error) => cached_lcsc_info(db, &code)?.ok_or_else(|| {
            CommandError::NotFound(format!("未找到 LCSC 元件 {code}：{network_error}"))
        }),
    }
}

fn text(value: Option<&Value>) -> String {
    value
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim()
        .to_owned()
}

fn parameter_value(parameter: &Value) -> String {
    for key in ["paramValueEn", "paramValue"] {
        let value = text(parameter.get(key));
        if !value.is_empty() && value != "-" {
            return value;
        }
    }
    String::new()
}

fn find_parameter_value<F>(parameters: &[Value], matches: F) -> Option<String>
where
    F: Fn(&str, &str) -> bool,
{
    parameters.iter().find_map(|parameter| {
        let name = text(parameter.get("paramName"));
        let name_en = text(parameter.get("paramNameEn"));
        matches(&name, &name_en)
            .then(|| parameter_value(parameter))
            .filter(|value| !value.is_empty())
    })
}

fn is_ptc_fuse(result: &Value) -> bool {
    let descriptors = [
        "catalogNameEn",
        "wmCatalogNameEn",
        "firstWmCatalogNameEn",
        "secondWmCatalogNameEn",
        "productNameEn",
        "productKeyAttributes",
        "productDescEn",
        "productIntroEn",
        "catalogName",
    ]
    .into_iter()
    .map(|key| text(result.get(key)))
    .collect::<Vec<_>>()
    .join(" ")
    .to_ascii_lowercase();
    let is_ptc = descriptors.contains("ptc") || descriptors.contains("自恢复");
    let is_fuse = descriptors.contains("fuse")
        || descriptors.contains("保险")
        || descriptors.contains("熔断");
    is_ptc && is_fuse
}

pub fn format_lcsc_display_name(raw_name: &str, package: &str) -> String {
    let raw_name = raw_name.trim();
    let resistance = raw_name
        .find('Ω')
        .and_then(|unit_start| compact_value_before_unit(raw_name, unit_start, "Ω", "kKM Rm uµ"));
    let capacitance = ["pF", "nF", "uF", "µF", "μF", "mF"]
        .iter()
        .find_map(|unit| {
            raw_name.find(unit).and_then(|unit_start| {
                compact_value_before_unit(raw_name, unit_start, unit, "pn uµμm")
            })
        });
    let (value, package_prefix) = match resistance
        .map(|value| (value, "R"))
        .or_else(|| capacitance.map(|value| (value, "C")))
    {
        Some(value) => value,
        None => return raw_name.to_owned(),
    };
    let package = package.trim();
    if package.len() == 4 && package.bytes().all(|byte| byte.is_ascii_digit()) {
        format!("{value} {package_prefix}{package}")
    } else if package.is_empty() {
        value
    } else {
        format!("{value} {package}")
    }
}

fn compact_value_before_unit(
    raw_name: &str,
    unit_start: usize,
    unit: &str,
    allowed_prefixes: &str,
) -> Option<String> {
    let numeric_prefix = &raw_name[..unit_start];
    let mut value_start = numeric_prefix.len();
    for (index, character) in numeric_prefix.char_indices().rev() {
        if character.is_ascii_digit()
            || character == '.'
            || character == ' '
            || allowed_prefixes.contains(character)
        {
            value_start = index;
            continue;
        }
        break;
    }
    let value = numeric_prefix[value_start..]
        .split_whitespace()
        .collect::<String>();
    if value.is_empty() || !value.chars().any(|character| character.is_ascii_digit()) {
        return None;
    }
    Some(format!("{}{}", value.replace('K', "k"), unit))
}

fn parse_wmsc(code: &str, payload: Value) -> Result<LcscPartInfo, String> {
    let result = payload
        .get("result")
        .ok_or_else(|| "WMSC 响应缺少 result".to_owned())?;
    let product_code = text(result.get("productCode"));
    if product_code != code {
        return Err("WMSC 响应的 LCSC ID 不匹配".into());
    }
    let package = text(result.get("encapStandard"));
    let model = text(result.get("productModel"));
    let parameters = result
        .get("paramVOList")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let name = if is_ptc_fuse(result) {
        let voltage = find_parameter_value(&parameters, |name, name_en| {
            name.contains("电压") || name_en.to_ascii_lowercase().contains("voltage")
        });
        let hold_current = find_parameter_value(&parameters, |name, name_en| {
            name.contains("保持电流")
                || name.contains("维持电流")
                || name_en.to_ascii_lowercase().contains("hold current")
        });
        match (voltage, hold_current) {
            (Some(voltage), Some(hold_current)) => {
                let mut fields = vec!["PTC".to_owned(), voltage, hold_current];
                if !package.is_empty() {
                    fields.push(package.clone());
                }
                fields.join(" ")
            }
            _ => model.clone(),
        }
    } else {
        let resistance = find_parameter_value(&parameters, |name, name_en| {
            name.contains("阻值")
                || name_en.eq_ignore_ascii_case("resistance")
                || name_en.to_ascii_lowercase().starts_with("resistance @")
        });
        let capacitance = find_parameter_value(&parameters, |name, name_en| {
            name.contains("容值")
                || name_en.eq_ignore_ascii_case("capacitance")
                || name_en.to_ascii_lowercase().starts_with("capacitance ")
        });
        let display_source = resistance
            .as_deref()
            .or(capacitance.as_deref())
            .unwrap_or(model.as_str());
        format_lcsc_display_name(display_source, &package)
    };
    Ok(LcscPartInfo {
        lcsc_code: product_code,
        name,
        category: text(result.get("parentCatalogName")),
        package,
        manufacturer: text(result.get("brandNameEn")),
        mpn: model,
    })
}

fn parse_easyeda(code: &str, payload: Value) -> Result<LcscPartInfo, String> {
    let result = payload
        .get("result")
        .ok_or_else(|| "EasyEDA 响应缺少 result".to_owned())?;
    let parameters = result
        .pointer("/dataStr/head/c_para")
        .ok_or_else(|| "EasyEDA 响应缺少元数据".to_owned())?;
    let actual_code = text(parameters.get("Supplier Part"));
    if actual_code != code {
        return Err("EasyEDA 响应的 LCSC ID 不匹配".into());
    }
    let package = text(parameters.get("package"));
    let name = text(parameters.get("name"));
    Ok(LcscPartInfo {
        lcsc_code: actual_code,
        name: format_lcsc_display_name(&name, &package),
        category: result
            .get("tags")
            .and_then(Value::as_array)
            .and_then(|tags| tags.first())
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned(),
        package,
        manufacturer: text(parameters.get("Manufacturer")),
        mpn: text(parameters.get("Manufacturer Part")),
    })
}

async fn fetch_json(client: &reqwest::Client, url: String) -> Result<Value, String> {
    let response = client
        .get(url)
        .send()
        .await
        .map_err(|error| error.to_string())?
        .error_for_status()
        .map_err(|error| error.to_string())?;
    response
        .json::<Value>()
        .await
        .map_err(|error| error.to_string())
}

fn lookup_lcsc_remote(
    code: &str,
) -> impl std::future::Future<Output = Result<LcscPartInfo, String>> + Send {
    let code = code.to_owned();
    async move {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(8))
            .user_agent("PartNest/0.1")
            .build()
            .map_err(|error| error.to_string())?;
        let wmsc = fetch_json(
            &client,
            format!("https://wmsc.lcsc.com/ftps/wm/product/detail?productCode={code}"),
        )
        .await;
        match wmsc.and_then(|payload| parse_wmsc(&code, payload)) {
            Ok(info) => Ok(info),
            Err(wmsc_error) => {
                let payload = fetch_json(
                    &client,
                    format!("https://easyeda.com/api/products/{code}/components"),
                )
                .await?;
                parse_easyeda(&code, payload).map_err(|easyeda_error| {
                    format!("WMSC: {wmsc_error}; EasyEDA: {easyeda_error}")
                })
            }
        }
    }
}

fn next_movement_sequence(transaction: &Transaction<'_>) -> Result<i64, CommandError> {
    transaction
        .query_row(
            "SELECT COALESCE(MAX(movement_sequence), 0) + 1 FROM inventory_movements",
            [],
            |row| row.get(0),
        )
        .map_err(CommandError::from)
}

fn validate_input(
    db: &Database,
    input: &PartInput,
    quantity: i64,
) -> Result<(Option<i64>, Option<String>), CommandError> {
    if quantity < 0 {
        return Err(CommandError::Validation("库存数量不能为负数".into()));
    }
    if quantity == 0 {
        return Ok((None, None));
    }
    let box_id = input
        .box_id
        .ok_or_else(|| CommandError::Validation("有库存的器件必须选择收纳盒和盒位".into()))?;
    let slot = input
        .slot
        .as_deref()
        .ok_or_else(|| CommandError::Validation("有库存的器件必须选择收纳盒和盒位".into()))?;
    let Some((rows, cols)) = db
        .connection()
        .query_row(
            "SELECT rows, cols FROM boxes WHERE id = ?1",
            [box_id],
            |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)),
        )
        .optional()?
    else {
        return Err(CommandError::NotFound("收纳盒不存在".into()));
    };
    Ok((Some(box_id), Some(normalize_slot(slot, rows, cols)?)))
}

fn read_part(db: &Database, id: &str) -> Result<PartView, CommandError> {
    db.connection()
        .query_row(
            "SELECT id, name, category, package, manufacturer, mpn, lcsc_code, quantity, box_id, slot, note, version FROM parts WHERE id = ?1 AND deleted_at IS NULL",
            [id],
            |row| Ok(PartView {
                id: row.get(0)?, name: row.get(1)?, category: row.get(2)?, package: row.get(3)?,
                manufacturer: row.get(4)?, mpn: row.get(5)?, lcsc_code: row.get(6)?, quantity: row.get(7)?,
                box_id: row.get(8)?, slot: row.get(9)?, note: row.get(10)?, version: row.get(11)?,
            }),
        )
        .optional()?
        .ok_or_else(|| CommandError::NotFound("器件不存在".into()))
}

pub fn list_parts_service(
    db: &Database,
    search: Option<&str>,
) -> Result<Vec<PartView>, CommandError> {
    let mut statement = db.connection().prepare("SELECT id, name, category, package, manufacturer, mpn, lcsc_code, quantity, box_id, slot, note, version FROM parts WHERE deleted_at IS NULL ORDER BY name COLLATE NOCASE, id")?;
    let rows = statement.query_map([], |row| {
        Ok(PartView {
            id: row.get(0)?,
            name: row.get(1)?,
            category: row.get(2)?,
            package: row.get(3)?,
            manufacturer: row.get(4)?,
            mpn: row.get(5)?,
            lcsc_code: row.get(6)?,
            quantity: row.get(7)?,
            box_id: row.get(8)?,
            slot: row.get(9)?,
            note: row.get(10)?,
            version: row.get(11)?,
        })
    })?;
    let needle = search
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_ascii_lowercase);
    Ok(rows
        .collect::<rusqlite::Result<Vec<_>>>()?
        .into_iter()
        .filter(|part| {
            needle.as_ref().is_none_or(|needle| {
                [
                    part.name.as_str(),
                    part.category.as_deref().unwrap_or(""),
                    part.mpn.as_deref().unwrap_or(""),
                    part.lcsc_code.as_deref().unwrap_or(""),
                ]
                .iter()
                .any(|value| value.to_ascii_lowercase().contains(needle))
            })
        })
        .collect())
}

pub fn create_part_service(db: &Database, input: PartInput) -> Result<PartView, CommandError> {
    let name = validate_name(&input.name, "器件")?;
    let (box_id, slot) = validate_input(db, &input, input.quantity)?;
    let lcsc_code = normalize_input_lcsc_code(&input.lcsc_code)?;
    let id = new_id();
    let transaction = db.transaction()?;
    transaction.execute(
        "INSERT INTO parts (id, name, category, package, manufacturer, mpn, lcsc_code, quantity, box_id, slot, note) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
        params![id, name, optional_text(&input.category), optional_text(&input.package), optional_text(&input.manufacturer), optional_text(&input.mpn), lcsc_code, input.quantity, box_id, slot, optional_text(&input.note)],
    )?;
    if input.quantity > 0 {
        let sequence = next_movement_sequence(&transaction)?;
        transaction.execute(
            "INSERT INTO inventory_movements (id, part_id, movement_type, quantity, reason, before_quantity, after_quantity, movement_sequence) VALUES (?1, ?2, 'in', ?3, ?4, ?5, ?6, ?7)",
            params![new_id(), id, input.quantity, "initial stock", 0_i64, input.quantity, sequence],
        )?;
    }
    transaction.commit()?;
    read_part(db, &id)
}

pub fn update_part_service(
    db: &Database,
    id: &str,
    expected_version: i64,
    input: PartInput,
) -> Result<PartView, CommandError> {
    let transaction = db.transaction()?;
    let quantity: i64 = transaction
        .query_row(
            "SELECT quantity FROM parts WHERE id = ?1 AND deleted_at IS NULL",
            [id],
            |row| row.get(0),
        )
        .optional()?
        .ok_or_else(|| CommandError::NotFound("器件不存在".into()))?;
    let name = validate_name(&input.name, "器件")?;
    // Existing stock is changed only through adjust_stock. A zero-stock item
    // may be stocked during editing because it has no live location yet.
    let target_quantity = if quantity == 0 {
        input.quantity
    } else {
        quantity
    };
    let (box_id, slot) = validate_input(db, &input, target_quantity)?;
    let lcsc_code = normalize_input_lcsc_code(&input.lcsc_code)?;
    let now = utc_now();
    let changed = transaction.execute(
        "UPDATE parts SET name = ?1, category = ?2, package = ?3, manufacturer = ?4, mpn = ?5, lcsc_code = ?6, quantity = ?7, box_id = ?8, slot = ?9, note = ?10, version = version + 1, updated_at = ?11 WHERE id = ?12 AND version = ?13 AND deleted_at IS NULL",
        params![name, optional_text(&input.category), optional_text(&input.package), optional_text(&input.manufacturer), optional_text(&input.mpn), lcsc_code, target_quantity, box_id, slot, optional_text(&input.note), now, id, expected_version],
    )?;
    if changed != 1 {
        return Err(CommandError::Conflict);
    }
    if quantity == 0 && target_quantity > 0 {
        let sequence = next_movement_sequence(&transaction)?;
        transaction.execute(
            "INSERT INTO inventory_movements (id, part_id, movement_type, quantity, reason, before_quantity, after_quantity, movement_sequence) VALUES (?1, ?2, 'in', ?3, ?4, ?5, ?6, ?7)",
            params![new_id(), id, target_quantity, "restock during edit", 0_i64, target_quantity, sequence],
        )?;
    }
    transaction.commit()?;
    read_part(db, id)
}

pub fn adjust_stock_service(
    db: &Database,
    id: &str,
    delta: i64,
    reason: &str,
) -> Result<PartView, CommandError> {
    let reason = reason.trim();
    if delta == 0 || reason.is_empty() {
        return Err(CommandError::Validation("数量调整和原因不能为空".into()));
    }
    let transaction = db.transaction()?;
    let quantity: i64 = transaction
        .query_row(
            "SELECT quantity FROM parts WHERE id = ?1 AND deleted_at IS NULL",
            [id],
            |row| row.get(0),
        )
        .optional()?
        .ok_or_else(|| CommandError::NotFound("器件不存在".into()))?;
    let new_quantity = quantity
        .checked_add(delta)
        .ok_or_else(|| CommandError::Validation("库存数量超出范围".into()))?;
    if new_quantity < 0 {
        return Err(CommandError::Validation("库存数量不能为负数".into()));
    }
    let sequence = next_movement_sequence(&transaction)?;
    transaction.execute(
        "UPDATE parts SET quantity = ?1, box_id = CASE WHEN ?1 = 0 THEN NULL ELSE box_id END, slot = CASE WHEN ?1 = 0 THEN NULL ELSE slot END, version = version + 1, updated_at = ?2 WHERE id = ?3 AND deleted_at IS NULL",
        params![new_quantity, utc_now(), id],
    )?;
    transaction.execute("INSERT INTO inventory_movements (id, part_id, movement_type, quantity, reason, before_quantity, after_quantity, movement_sequence) VALUES (?1, ?2, 'adjust', ?3, ?4, ?5, ?6, ?7)", params![new_id(), id, delta, reason, quantity, new_quantity, sequence])?;
    transaction.commit()?;
    read_part(db, id)
}

/// Archive an inventory item without losing its audited stock or identity.
/// Quantity is retained only as the baseline for legacy movement reconstruction.
pub fn delete_part_service(db: &Database, id: &str) -> Result<(), CommandError> {
    let tx = rusqlite::Transaction::new_unchecked(
        db.connection(),
        rusqlite::TransactionBehavior::Immediate,
    )?;
    let version: i64 = tx
        .query_row(
            "SELECT version FROM parts WHERE id = ?1 AND deleted_at IS NULL",
            [id],
            |row| row.get(0),
        )
        .optional()?
        .ok_or_else(|| CommandError::NotFound("器件不存在或已删除".into()))?;
    let next_version = version
        .checked_add(1)
        .ok_or_else(|| CommandError::Validation("器件版本超出范围".into()))?;
    let now = utc_now();
    tx.execute(
        "UPDATE parts SET deleted_at = ?1, updated_at = ?1, version = ?2, box_id = NULL, slot = NULL WHERE id = ?3 AND deleted_at IS NULL",
        params![now, next_version, id],
    )?;
    // Keep confirmed placements and totals, but allow replacement stock to bind
    // for fresh placements in the active session. Audit rows keep the old ID.
    tx.execute(
        "UPDATE welding_progress SET part_id = NULL, updated_at = ?1 WHERE part_id = ?2 AND session_id IN (SELECT id FROM welding_sessions WHERE status = 'active')",
        params![now, id],
    )?;
    tx.commit()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::parse_wmsc;
    use serde_json::json;

    #[test]
    fn wmsc_param_value_provides_the_compact_resistance_name() {
        let payload = json!({
            "result": {
                "productCode": "C2998176",
                "productModel": "FRC0402F2103TS",
                "encapStandard": "0402",
                "paramVOList": [
                    { "paramName": "阻值", "paramValue": "210kΩ" }
                ]
            }
        });

        let info = parse_wmsc("C2998176", payload).expect("parse WMSC response");
        assert_eq!(info.name, "210kΩ R0402");
        assert_eq!(info.mpn, "FRC0402F2103TS");
    }

    #[test]
    fn wmsc_capacitance_param_value_provides_the_compact_capacitor_name() {
        let payload = json!({
            "result": {
                "productCode": "C123456",
                "productModel": "CL05B104KO5NNNC",
                "encapStandard": "0402",
                "paramVOList": [
                    { "paramName": "容值", "paramNameEn": "Capacitance", "paramValue": "100nF" }
                ]
            }
        });

        let info = parse_wmsc("C123456", payload).expect("parse WMSC response");
        assert_eq!(info.name, "100nF C0402");
        assert_eq!(info.mpn, "CL05B104KO5NNNC");
    }

    #[test]
    fn wmsc_ptc_fuse_name_uses_voltage_hold_current_and_package() {
        let payload = json!({
            "result": {
                "productCode": "C46641020",
                "productModel": "SMD1206-075-16",
                "productNameEn": "PTC RESET FUSE 16V 750mA 1206",
                "productKeyAttributes": "PTC RESET FUSE 16V 750mA 1206",
                "parentCatalogName": "Circuit Protection",
                "catalogNameEn": "PTC Resettable Fuses",
                "encapStandard": "1206",
                "paramVOList": [
                    { "paramNameEn": "Voltage - Max", "paramValue": "16V", "paramValueEn": "16V" },
                    { "paramNameEn": "Hold Current", "paramValue": "750mA", "paramValueEn": "750mA" },
                    { "paramName": "阻值", "paramNameEn": "Resistance @ 25°C", "paramValue": "90mΩ", "paramValueEn": "90mΩ" }
                ]
            }
        });

        let info = parse_wmsc("C46641020", payload).expect("parse WMSC response");
        assert_eq!(info.name, "PTC 16V 750mA 1206");
    }
}

#[tauri::command(rename = "lookup_lcsc")]
pub async fn lookup_lcsc(
    state: State<'_, Mutex<Database>>,
    lcsc_code: String,
) -> Result<LcscPartInfo, CommandError> {
    let code = normalize_lcsc_code(&lcsc_code)?;
    // The database mutex is only taken for the short cache read/write. Holding
    // it across a two-stage network lookup would stall every other command,
    // including a confirm-take, for the full timeout window.
    match lookup_lcsc_remote(&code).await {
        Ok(mut info) => {
            info.lcsc_code = code;
            let database = state.lock().map_err(lock_error)?;
            cache_lcsc_info(&database, &info)?;
            Ok(info)
        }
        Err(network_error) => {
            let database = state.lock().map_err(lock_error)?;
            cached_lcsc_info(&database, &code)?.ok_or_else(|| {
                CommandError::NotFound(format!("未找到 LCSC 元件 {code}：{network_error}"))
            })
        }
    }
}

#[tauri::command(rename = "list_parts")]
pub fn list_parts(
    state: State<'_, Mutex<Database>>,
    search: Option<String>,
) -> Result<Vec<PartView>, CommandError> {
    let db = state.lock().map_err(lock_error)?;
    list_parts_service(&db, search.as_deref())
}

#[tauri::command(rename = "create_part")]
pub fn create_part(
    state: State<'_, Mutex<Database>>,
    input: PartInput,
) -> Result<PartView, CommandError> {
    let db = state.lock().map_err(lock_error)?;
    create_part_service(&db, input)
}

#[tauri::command(rename = "update_part")]
pub fn update_part(
    state: State<'_, Mutex<Database>>,
    id: String,
    expected_version: i64,
    input: PartInput,
) -> Result<PartView, CommandError> {
    let db = state.lock().map_err(lock_error)?;
    update_part_service(&db, &id, expected_version, input)
}

#[tauri::command(rename = "adjust_stock")]
pub fn adjust_stock(
    state: State<'_, Mutex<Database>>,
    id: String,
    delta: i64,
    reason: String,
) -> Result<PartView, CommandError> {
    let db = state.lock().map_err(lock_error)?;
    adjust_stock_service(&db, &id, delta, &reason)
}

#[tauri::command(rename = "delete_part")]
pub fn delete_part(state: State<'_, Mutex<Database>>, id: String) -> Result<(), CommandError> {
    let db = state.lock().map_err(lock_error)?;
    delete_part_service(&db, &id)
}
