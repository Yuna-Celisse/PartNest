use super::{lock_error, CommandError};
use crate::db::Database;
use serde::Serialize;
use std::collections::HashMap;
use std::sync::Mutex;
use tauri::State;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct MovementView {
    pub id: String,
    pub part_id: Option<String>,
    pub part_name: Option<String>,
    pub component: Option<String>,
    pub component_key: Option<String>,
    pub side: Option<String>,
    pub movement_type: String,
    pub delta: i64,
    pub quantity: i64,
    pub before_quantity: Option<i64>,
    pub after_quantity: Option<i64>,
    pub reason: String,
    pub bom_display_name: Option<String>,
    pub session_id: Option<String>,
    pub created_at: String,
    pub reverses_movement_id: Option<String>,
    pub reversible: bool,
}

struct RawMovement {
    id: String,
    part_id: Option<String>,
    part_name: Option<String>,
    movement_type: String,
    quantity: i64,
    before_quantity: Option<i64>,
    after_quantity: Option<i64>,
    reason: String,
    bom_display_name: Option<String>,
    session_id: Option<String>,
    component_key: Option<String>,
    side: Option<String>,
    session_exists: bool,
    has_progress: bool,
    created_at: String,
    reverses_movement_id: Option<String>,
    has_reversal: bool,
    part_active: bool,
}

pub fn list_movements_service(db: &Database) -> Result<Vec<MovementView>, CommandError> {
    let mut stock = db
        .connection()
        .prepare("SELECT id, quantity FROM parts")?
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
        })?
        .collect::<rusqlite::Result<HashMap<_, _>>>()?;

    let mut statement = db.connection().prepare(
        "SELECT m.id, m.part_id, p.name, m.movement_type, m.quantity, m.reason,
                m.before_quantity, m.after_quantity,
                bf.display_name, m.session_id, m.component_key, m.side,
                EXISTS(SELECT 1 FROM welding_sessions session
                       WHERE session.id = m.session_id),
                EXISTS(SELECT 1 FROM welding_progress progress
                       WHERE progress.session_id = m.session_id
                         AND progress.component_key = m.component_key
                         AND progress.side = m.side),
                m.created_at,
                m.reverses_movement_id,
                EXISTS(SELECT 1 FROM inventory_movements reversal
                       WHERE reversal.reverses_movement_id = m.id),
                p.id IS NOT NULL AND p.deleted_at IS NULL
           FROM inventory_movements m
           LEFT JOIN parts p ON p.id = m.part_id
           LEFT JOIN welding_sessions ws ON ws.id = m.session_id
           LEFT JOIN bom_files bf ON bf.id = ws.bom_file_id
           ORDER BY m.created_at DESC, COALESCE(m.movement_sequence, 0) DESC, m.id DESC",
    )?;
    let rows = statement.query_map([], |row| {
        Ok(RawMovement {
            id: row.get(0)?,
            part_id: row.get(1)?,
            part_name: row.get(2)?,
            movement_type: row.get(3)?,
            quantity: row.get(4)?,
            reason: row.get(5)?,
            before_quantity: row.get(6)?,
            after_quantity: row.get(7)?,
            bom_display_name: row.get(8)?,
            session_id: row.get(9)?,
            component_key: row.get(10)?,
            side: row.get(11)?,
            session_exists: row.get(12)?,
            has_progress: row.get(13)?,
            created_at: row.get(14)?,
            reverses_movement_id: row.get(15)?,
            has_reversal: row.get(16)?,
            part_active: row.get(17)?,
        })
    })?;

    let mut result = Vec::new();
    for row in rows {
        let movement = row?;
        let (before_quantity, after_quantity) = if movement.before_quantity.is_some()
            && movement.after_quantity.is_some()
        {
            // The list is newest-first. A modern audited row still has to
            // rewind the cursor to its `before` value so older nullable
            // rows are reconstructed against the correct historical stock.
            if let (Some(part_id), Some(before)) = (&movement.part_id, movement.before_quantity) {
                stock.insert(part_id.clone(), before);
            }
            (movement.before_quantity, movement.after_quantity)
        } else if let Some(part_id) = &movement.part_id {
            let after = stock
                .get(part_id)
                .copied()
                .ok_or_else(|| CommandError::Database("流水关联的器件不存在".into()))?;
            let before = after
                .checked_sub(movement.quantity)
                .ok_or_else(|| CommandError::Database("流水库存数量超出范围".into()))?;
            stock.insert(part_id.clone(), before);
            (Some(before), Some(after))
        } else {
            (None, None)
        };
        let reversible = movement.movement_type == "consume"
            && movement.quantity < 0
            && movement.reverses_movement_id.is_none()
            && !movement.has_reversal
            && movement.part_active
            && movement.part_id.is_some()
            && movement.part_name.is_some()
            && movement.session_exists
            && movement
                .component_key
                .as_deref()
                .is_some_and(|component_key| !component_key.trim().is_empty())
            && movement
                .side
                .as_deref()
                .is_some_and(|side| matches!(side, "top" | "bottom"))
            && movement.has_progress;
        result.push(MovementView {
            id: movement.id,
            part_id: movement.part_id,
            component: movement
                .part_name
                .clone()
                .or_else(|| movement.component_key.clone()),
            part_name: movement.part_name,
            component_key: movement.component_key,
            movement_type: movement.movement_type,
            delta: movement.quantity,
            quantity: movement.quantity,
            before_quantity,
            after_quantity,
            reason: movement.reason,
            bom_display_name: movement.bom_display_name,
            session_id: movement.session_id,
            side: movement.side,
            created_at: movement.created_at,
            reverses_movement_id: movement.reverses_movement_id,
            reversible,
        });
    }
    Ok(result)
}

#[tauri::command(rename = "list_movements")]
pub fn list_movements(
    state: State<'_, Mutex<Database>>,
) -> Result<Vec<MovementView>, CommandError> {
    let database = state.lock().map_err(lock_error)?;
    list_movements_service(&database)
}
