//! Transactional persistence for the welding workspace.

use crate::{
    bom::types::BomSide,
    commands::CommandError,
    db::{new_id, utc_now, Database},
};
use rusqlite::{params, Connection, OptionalExtension, Transaction};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ConfirmTakeInput {
    pub session_id: String,
    pub component_key: String,
    pub side: BomSide,
    pub designators: Vec<String>,
    pub bom_quantity: i64,
    pub take_quantity: i64,
    pub part_id: String,
    pub expected_part_version: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TakeResult {
    pub movement_id: String,
    pub session_id: String,
    pub component_key: String,
    pub side: BomSide,
    pub part_id: String,
    pub take_quantity: i64,
    pub required_quantity: i64,
    pub consumed_quantity: i64,
    pub taken_quantity: i64,
    pub status: String,
    pub part_version: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WeldingProgress {
    pub session_id: String,
    pub component_key: String,
    pub side: BomSide,
    pub part_id: Option<String>,
    pub required_quantity: i64,
    pub consumed_quantity: i64,
    pub taken_quantity: i64,
    /// Designators already consumed for this side. The welding workspace uses it
    /// to show what is done and to avoid re-charging the same designator.
    pub confirmed_designators: Vec<String>,
    pub status: String,
}

fn side_text(side: &BomSide) -> &'static str {
    match side {
        BomSide::Top => "top",
        BomSide::Bottom => "bottom",
    }
}

fn side_from_text(value: &str) -> Result<BomSide, CommandError> {
    match value {
        "top" => Ok(BomSide::Top),
        "bottom" => Ok(BomSide::Bottom),
        _ => Err(CommandError::Database("焊接板面数据无效".into())),
    }
}

fn status(consumed: i64, required: i64) -> &'static str {
    if consumed == 0 {
        "pending"
    } else if consumed >= required {
        "taken"
    } else {
        "partial"
    }
}

fn parse_designators(raw: &str) -> Vec<String> {
    serde_json::from_str(raw).unwrap_or_default()
}

fn designators_json(designators: &[String]) -> Result<String, CommandError> {
    serde_json::to_string(designators)
        .map_err(|error| CommandError::Validation(format!("位号无法记录: {error}")))
}

/// Union of two designator sets, sorted so the stored JSON is deterministic.
fn merged_designators(mut kept: Vec<String>, added: &[String]) -> Vec<String> {
    kept.extend(added.iter().cloned());
    kept.sort();
    kept.dedup();
    kept
}

fn without_designators(mut kept: Vec<String>, removed: &[String]) -> Vec<String> {
    for value in removed {
        kept.retain(|existing| existing != value);
    }
    kept.sort();
    kept.dedup();
    kept
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

fn validate_input(input: &ConfirmTakeInput) -> Result<(), CommandError> {
    if input.session_id.trim().is_empty() || input.component_key.trim().is_empty() {
        return Err(CommandError::Validation(
            "焊接会话和器件分组不能为空".into(),
        ));
    }
    if input.part_id.trim().is_empty() {
        return Err(CommandError::Validation("器件不能为空".into()));
    }
    if input.take_quantity <= 0 {
        return Err(CommandError::Validation("取用数量必须为正整数".into()));
    }
    if input.bom_quantity <= 0 {
        return Err(CommandError::Validation("BOM数量必须为正整数".into()));
    }
    if input.expected_part_version <= 0 {
        return Err(CommandError::Validation("器件版本无效".into()));
    }
    if input.designators.is_empty()
        || input
            .designators
            .iter()
            .any(|designator| designator.is_empty() || designator.trim() != designator)
    {
        return Err(CommandError::Validation(
            "位号不能为空或包含首尾空格".into(),
        ));
    }
    let unique = input
        .designators
        .iter()
        .collect::<std::collections::HashSet<_>>();
    if unique.len() != input.designators.len() {
        return Err(CommandError::Validation("位号不能重复".into()));
    }
    Ok(())
}

fn session_active(connection: &Connection, session_id: &str) -> Result<(), CommandError> {
    let active = connection
        .query_row(
            "SELECT status = 'active' FROM welding_sessions WHERE id = ?1",
            [session_id],
            |row| row.get::<_, bool>(0),
        )
        .optional()?;
    if active != Some(true) {
        return Err(CommandError::NotFound("焊接会话不存在或未激活".into()));
    }
    Ok(())
}

fn session_exists(connection: &Connection, session_id: &str) -> Result<(), CommandError> {
    let known = connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM welding_sessions WHERE id = ?1)",
            [session_id],
            |row| row.get::<_, bool>(0),
        )
        .optional()?
        .unwrap_or(false);
    if !known {
        return Err(CommandError::NotFound("焊接会话不存在".into()));
    }
    Ok(())
}

/// End the workspace's active welding session. Its takes stay reversible: the
/// board simply stops being the current session.
pub fn end_welding_session(db: &Database, session_id: &str) -> Result<(), CommandError> {
    if session_id.trim().is_empty() {
        return Err(CommandError::Validation("焊接会话编号不能为空".into()));
    }
    let ended = db.connection().execute(
        "UPDATE welding_sessions SET status = 'completed', updated_at = ?1 WHERE id = ?2 AND status = 'active'",
        params![utc_now(), session_id],
    )?;
    if ended == 0 {
        return Err(CommandError::NotFound("焊接会话不存在或已结束".into()));
    }
    Ok(())
}

fn result_from_progress(
    movement_id: String,
    input: &ConfirmTakeInput,
    part_id: String,
    part_version: i64,
    required: i64,
    consumed: i64,
) -> TakeResult {
    TakeResult {
        movement_id,
        session_id: input.session_id.clone(),
        component_key: input.component_key.clone(),
        side: input.side.clone(),
        part_id,
        take_quantity: input.take_quantity,
        required_quantity: required,
        consumed_quantity: consumed,
        taken_quantity: consumed,
        status: status(consumed, required).into(),
        part_version,
    }
}

/// Atomically consumes inventory, audits it, and updates one side's progress.
pub fn confirm_take(db: &Database, input: ConfirmTakeInput) -> Result<TakeResult, CommandError> {
    validate_input(&input)?;
    let side = side_text(&input.side);
    let tx = db.transaction()?;
    session_active(&tx, &input.session_id)?;

    let (quantity, version): (i64, i64) = tx
        .query_row(
            "SELECT quantity, version FROM parts WHERE id = ?1 AND deleted_at IS NULL",
            [&input.part_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?
        .ok_or_else(|| CommandError::NotFound("器件不存在".into()))?;
    if version != input.expected_part_version {
        return Err(CommandError::Conflict);
    }
    let remaining = quantity
        .checked_sub(input.take_quantity)
        .ok_or_else(|| CommandError::Validation("库存数量超出范围".into()))?;
    if remaining < 0 {
        return Err(CommandError::Validation("库存不足".into()));
    }
    let next_version = version
        .checked_add(1)
        .ok_or_else(|| CommandError::Validation("器件版本超出范围".into()))?;
    let sequence = next_movement_sequence(&tx)?;

    let existing = tx
        .query_row(
            "SELECT part_id, required_quantity, taken_quantity, confirmed_designators FROM welding_progress WHERE session_id = ?1 AND component_key = ?2 AND side = ?3",
            params![input.session_id, input.component_key, side],
            |row| Ok((row.get::<_, Option<String>>(0)?, row.get::<_, i64>(1)?, row.get::<_, i64>(2)?, row.get::<_, String>(3)?)),
        )
        .optional()?;
    if let Some((Some(existing_part), _, _, _)) = &existing {
        if existing_part != &input.part_id {
            return Err(CommandError::Conflict);
        }
    }
    let (existing_required, previous_consumed, confirmed) = existing
        .map(|(_, required, consumed, confirmed)| {
            (required, consumed, parse_designators(&confirmed))
        })
        .unwrap_or((input.bom_quantity, 0, Vec::new()));
    // A designator identifies exactly one board position, so it may be charged
    // once per session even when the operator switches the working side. Table
    // BOMs without a side column would otherwise deduct the same placement twice.
    let charged: Vec<String> = {
        let mut statement = tx.prepare(
            "SELECT confirmed_designators FROM welding_progress WHERE session_id = ?1 AND component_key = ?2",
        )?;
        let rows = statement
            .query_map(params![input.session_id, input.component_key], |row| {
                row.get::<_, String>(0)
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        rows.iter().flat_map(|raw| parse_designators(raw)).collect()
    };
    // A designator may only be charged once per session, otherwise the audit
    // trail claims the same placement was consumed twice. Re-selecting an
    // already taken placement is allowed as long as something new is charged.
    let fresh: Vec<String> = input
        .designators
        .iter()
        .filter(|designator| !charged.contains(*designator))
        .cloned()
        .collect();
    if fresh.is_empty() {
        return Err(CommandError::Validation(
            "所选位号均已取用，如需追加请重新选择位号，如需纠正请撤销取用流水".into(),
        ));
    }
    let confirmation_designators = designators_json(&fresh)?;
    // Selecting a wider subset later must raise the requirement; the first
    // selection can never permanently understate what the board side needs.
    let required = existing_required.max(input.bom_quantity);
    let confirmed = merged_designators(confirmed, &fresh);
    let consumed = previous_consumed
        .checked_add(input.take_quantity)
        .ok_or_else(|| CommandError::Validation("焊接数量超出范围".into()))?;

    tx.execute(
        "UPDATE parts SET quantity = ?1, version = ?2, updated_at = ?3 WHERE id = ?4 AND version = ?5 AND deleted_at IS NULL",
        params![remaining, next_version, utc_now(), input.part_id, version],
    )?;
    let movement_id = new_id();
    tx.execute(
        "INSERT INTO inventory_movements (id, part_id, session_id, movement_type, quantity, reason, component_key, side, before_quantity, after_quantity, movement_sequence, bom_quantity, confirmation_designators) VALUES (?1, ?2, ?3, 'consume', ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
        params![movement_id, input.part_id, input.session_id, -input.take_quantity, "welding take", input.component_key, side, quantity, remaining, sequence, input.bom_quantity, confirmation_designators],
    )?;
    let confirmed_json = designators_json(&confirmed)?;
    tx.execute(
        "INSERT INTO welding_progress (id, session_id, component_key, side, part_id, required_quantity, taken_quantity, confirmed_designators, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9) ON CONFLICT(session_id, component_key, side) DO UPDATE SET part_id = excluded.part_id, required_quantity = excluded.required_quantity, taken_quantity = excluded.taken_quantity, confirmed_designators = excluded.confirmed_designators, updated_at = excluded.updated_at",
        params![new_id(), input.session_id, input.component_key, side, input.part_id, required, consumed, confirmed_json, utc_now()],
    )?;
    tx.execute(
        "UPDATE welding_sessions SET updated_at = ?1 WHERE id = ?2",
        params![utc_now(), input.session_id],
    )?;
    tx.commit()?;
    Ok(result_from_progress(
        movement_id,
        &input,
        input.part_id.clone(),
        next_version,
        required,
        consumed,
    ))
}

/// Restores a consume movement exactly once and recomputes its side.
pub fn reverse_take(db: &Database, movement_id: &str) -> Result<TakeResult, CommandError> {
    if movement_id.trim().is_empty() {
        return Err(CommandError::Validation("流水编号不能为空".into()));
    }
    let tx = db.transaction()?;
    let original = tx
        .query_row(
            "SELECT part_id, session_id, component_key, side, movement_type, quantity, bom_quantity, confirmation_designators FROM inventory_movements WHERE id = ?1",
            [movement_id],
            |row| Ok((row.get::<_, Option<String>>(0)?, row.get::<_, Option<String>>(1)?, row.get::<_, Option<String>>(2)?, row.get::<_, Option<String>>(3)?, row.get::<_, String>(4)?, row.get::<_, i64>(5)?, row.get::<_, Option<i64>>(6)?, row.get::<_, Option<String>>(7)?)),
        )
        .optional()?
        .ok_or_else(|| CommandError::NotFound("库存流水不存在".into()))?;
    let (
        part_id,
        session_id,
        component_key,
        side,
        movement_type,
        quantity,
        bom_quantity,
        confirmation_designators,
    ) = original;
    if movement_type != "consume" || quantity >= 0 {
        return Err(CommandError::Validation("只有取用流水可以撤销".into()));
    }
    let part_id = part_id.ok_or_else(|| CommandError::Validation("取用流水缺少器件".into()))?;
    let session_id =
        session_id.ok_or_else(|| CommandError::Validation("取用流水缺少焊接会话".into()))?;
    let component_key =
        component_key.ok_or_else(|| CommandError::Validation("取用流水缺少器件分组".into()))?;
    let side = side.ok_or_else(|| CommandError::Validation("取用流水缺少板面".into()))?;
    // Reversing corrects inventory history rather than spending a live bridge
    // authorization, so an ended session's takes stay reversible.
    session_exists(&tx, &session_id)?;
    if tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM inventory_movements WHERE reverses_movement_id = ?1)",
        [movement_id],
        |row| row.get::<_, bool>(0),
    )? {
        return Err(CommandError::Conflict);
    }
    let restore = quantity
        .checked_neg()
        .ok_or_else(|| CommandError::Validation("库存数量超出范围".into()))?;
    let (stock, version): (i64, i64) = tx
        .query_row(
            "SELECT quantity, version FROM parts WHERE id = ?1 AND deleted_at IS NULL",
            [&part_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?
        .ok_or_else(|| CommandError::NotFound("器件不存在".into()))?;
    let next_stock = stock
        .checked_add(restore)
        .ok_or_else(|| CommandError::Validation("库存数量超出范围".into()))?;
    let next_version = version
        .checked_add(1)
        .ok_or_else(|| CommandError::Validation("器件版本超出范围".into()))?;
    let sequence = next_movement_sequence(&tx)?;
    tx.execute(
        "UPDATE parts SET quantity = ?1, version = ?2, updated_at = ?3 WHERE id = ?4 AND version = ?5 AND deleted_at IS NULL",
        params![next_stock, next_version, utc_now(), part_id, version],
    )?;
    let reversal_id = new_id();
    tx.execute(
        "INSERT INTO inventory_movements (id, part_id, session_id, movement_type, quantity, reason, reverses_movement_id, component_key, side, before_quantity, after_quantity, movement_sequence, bom_quantity, confirmation_designators) VALUES (?1, ?2, ?3, 'reverse', ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
        params![reversal_id, part_id, session_id, restore, "welding take reversal", movement_id, component_key, side, stock, next_stock, sequence, bom_quantity, confirmation_designators],
    )?;
    let (required, part, confirmed): (i64, Option<String>, String) = tx
        .query_row(
            "SELECT required_quantity, part_id, confirmed_designators FROM welding_progress WHERE session_id = ?1 AND component_key = ?2 AND side = ?3",
            params![session_id, component_key, side],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()?
        .ok_or_else(|| CommandError::NotFound("焊接进度不存在".into()))?;
    let net: i64 = tx.query_row(
        "SELECT COALESCE(-SUM(quantity), 0) FROM inventory_movements WHERE session_id = ?1 AND component_key = ?2 AND side = ?3",
        params![session_id, component_key, side], |row| row.get(0))?;
    let consumed = net.max(0);
    // Releasing the take must also release its designators, otherwise the
    // restored stock could never be charged for those placements again.
    let released: Vec<String> = confirmation_designators
        .as_deref()
        .map(parse_designators)
        .unwrap_or_default();
    let confirmed = designators_json(&without_designators(
        parse_designators(&confirmed),
        &released,
    ))?;
    tx.execute(
        "UPDATE welding_progress SET taken_quantity = ?1, confirmed_designators = ?2, updated_at = ?3 WHERE session_id = ?4 AND component_key = ?5 AND side = ?6",
        params![consumed, confirmed, utc_now(), session_id, component_key, side],
    )?;
    tx.execute(
        "UPDATE welding_sessions SET updated_at = ?1 WHERE id = ?2",
        params![utc_now(), session_id],
    )?;
    tx.commit()?;
    let side = side_from_text(&side)?;
    Ok(TakeResult {
        movement_id: reversal_id,
        session_id,
        component_key,
        side,
        part_id: part.unwrap_or(part_id),
        take_quantity: restore,
        required_quantity: required,
        consumed_quantity: consumed,
        taken_quantity: consumed,
        status: status(consumed, required).into(),
        part_version: next_version,
    })
}

pub fn get_welding_progress(
    db: &Database,
    session_id: &str,
) -> Result<Vec<WeldingProgress>, CommandError> {
    if session_id.trim().is_empty() {
        return Err(CommandError::Validation("焊接会话不能为空".into()));
    }
    let mut statement = db.connection().prepare(
        "SELECT session_id, component_key, side, part_id, required_quantity, taken_quantity, confirmed_designators FROM welding_progress WHERE session_id = ?1 ORDER BY component_key, side",
    )?;
    let rows = statement.query_map([session_id], |row| {
        let side: String = row.get(2)?;
        let required: i64 = row.get(4)?;
        let consumed: i64 = row.get(5)?;
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            side,
            row.get::<_, Option<String>>(3)?,
            required,
            consumed,
            row.get::<_, String>(6)?,
        ))
    })?;
    let mut result = Vec::new();
    for row in rows {
        let (session_id, component_key, side, part_id, required, consumed, confirmed) = row?;
        result.push(WeldingProgress {
            session_id,
            component_key,
            side: side_from_text(&side)?,
            part_id,
            required_quantity: required,
            consumed_quantity: consumed,
            taken_quantity: consumed,
            confirmed_designators: parse_designators(&confirmed),
            status: status(consumed, required).into(),
        });
    }
    Ok(result)
}
