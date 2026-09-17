//! Project commands: a project is one imported BOM, either an interactive HTML
//! export or a CSV/XLSX table, and the welding workspace opens from it.

use crate::{
    bom::{
        bridge::{validate_selection_message, BomSelectionMessage, BridgeError},
        cache::{
            preview_interactive_bom, CachedBomSession, ImportedProject, InteractiveBomRuntime,
            ResolvedSelection,
        },
        projects::{load_snapshot, remove_project, rename_project, ProjectKind, ProjectRecord},
        tabular::inspect_tabular_bom as inspect_tabular_bom_file,
        types::{FieldMapping, ImportPreview},
    },
    db::Database,
};
use serde::Serialize;
use std::sync::Mutex;
use tauri::State;

#[derive(Debug, Clone, Serialize)]
pub struct ProjectSummary {
    pub id: String,
    pub name: String,
    pub original_name: String,
    pub kind: ProjectKind,
    /// Whether a parts table backs the project, as its source or its companion.
    pub has_table: bool,
    pub created_at: String,
    /// Whether this project currently owns the welding session.
    pub active: bool,
}

impl ProjectSummary {
    fn from_record(record: &ProjectRecord, created_at: String, active: bool) -> Self {
        Self {
            id: record.id.clone(),
            name: record.name.clone(),
            original_name: record.original_name.clone(),
            kind: record.kind,
            has_table: record.has_table,
            created_at,
            active,
        }
    }
}

/// Import arguments. `mapping` only applies to tabular BOMs whose headers could
/// not be resolved on their own, and `companion_csv_path` only to interactive
/// ones, whose export may ship without the parts columns the viewer expects.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct ImportProjectInput {
    pub source_path: String,
    pub name: Option<String>,
    pub mapping: Option<FieldMapping>,
    pub companion_csv_path: Option<String>,
}

#[tauri::command(rename = "list_projects")]
pub fn list_projects(database: State<'_, Mutex<Database>>) -> Result<Vec<ProjectSummary>, String> {
    let database = database.lock().map_err(|_| "数据库锁不可用".to_owned())?;
    let mut stmt = database.connection().prepare("SELECT p.id, p.name, p.original_name, p.kind, p.has_table, p.created_at, EXISTS(SELECT 1 FROM welding_sessions s WHERE s.project_id = p.id AND s.status = 'active') FROM projects p ORDER BY p.created_at DESC").map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |row| {
            let kind: String = row.get(3)?;
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                kind,
                row.get::<_, bool>(4)?,
                row.get::<_, String>(5)?,
                row.get::<_, bool>(6)?,
            ))
        })
        .map_err(|e| e.to_string())?;
    rows.map(|row| {
        let (id, name, original_name, kind, has_table, created_at, active) =
            row.map_err(|e| e.to_string())?;
        Ok(ProjectSummary {
            id,
            name,
            original_name,
            kind: ProjectKind::from_db(&kind).map_err(|e| e.user_message())?,
            has_table,
            created_at,
            active,
        })
    })
    .collect()
}

/// Import a BOM file as a project. Analysis alone never changes the welding
/// workspace; opening a project is the explicit step that does.
#[tauri::command(rename = "import_project")]
pub fn import_project(
    input: ImportProjectInput,
    database: State<'_, Mutex<Database>>,
    runtime: State<'_, Mutex<InteractiveBomRuntime>>,
) -> Result<ImportedProject, String> {
    let database = database.lock().map_err(|_| "数据库锁不可用".to_owned())?;
    let runtime = runtime
        .lock()
        .map_err(|_| "BOM 缓存状态锁不可用".to_owned())?;
    runtime
        .import_project(
            &database,
            std::path::Path::new(&input.source_path),
            input.name.as_deref(),
            input.mapping.as_ref(),
            input
                .companion_csv_path
                .as_deref()
                .map(std::path::Path::new),
        )
        .map_err(|error| error.user_message())
}

/// Reopen a project's stored analysis without touching the welding session.
#[tauri::command(rename = "analyze_project")]
pub fn analyze_project(
    id: String,
    database: State<'_, Mutex<Database>>,
) -> Result<ImportPreview, String> {
    let database = database.lock().map_err(|_| "数据库锁不可用".to_owned())?;
    load_snapshot(&database, &id)
        .map(ImportPreview::Ready)
        .map_err(|error| error.user_message())
}

#[tauri::command(rename = "rename_project")]
pub fn rename_project_command(
    id: String,
    name: String,
    database: State<'_, Mutex<Database>>,
) -> Result<ProjectSummary, String> {
    let database = database.lock().map_err(|_| "数据库锁不可用".to_owned())?;
    let record = rename_project(&database, &id, &name).map_err(|error| error.user_message())?;
    summary(&database, &record).map_err(|error| error.to_string())
}

/// Delete a project record and its cached canvas.
#[tauri::command(rename = "remove_project")]
pub fn remove_project_command(
    id: String,
    database: State<'_, Mutex<Database>>,
    runtime: State<'_, Mutex<InteractiveBomRuntime>>,
) -> Result<(), String> {
    let database = database.lock().map_err(|_| "数据库锁不可用".to_owned())?;
    let runtime = runtime
        .lock()
        .map_err(|_| "BOM 缓存状态锁不可用".to_owned())?;
    remove_project(&database, runtime.cache_dir(), &id).map_err(|error| error.user_message())
}

/// Open the welding workspace for one project, making it the active session.
#[tauri::command(rename = "open_project_welding")]
pub fn open_project_welding(
    id: String,
    database: State<'_, Mutex<Database>>,
    runtime: State<'_, Mutex<InteractiveBomRuntime>>,
) -> Result<CachedBomSession, String> {
    let database = database.lock().map_err(|_| "数据库锁不可用".to_owned())?;
    let runtime = runtime
        .lock()
        .map_err(|_| "BOM 缓存状态锁不可用".to_owned())?;
    runtime
        .open_project_welding(&database, &id)
        .map_err(|error| error.user_message())
}

/// Preview a tabular BOM. Nothing is stored: the operator confirms the mapping
/// first and imports the project afterwards.
#[tauri::command(rename = "inspect_tabular_bom")]
pub fn inspect_tabular_bom(
    source_path: String,
    mapping: Option<FieldMapping>,
) -> Result<ImportPreview, String> {
    inspect_tabular_bom_file(&source_path, mapping.as_ref()).map_err(|error| error.to_string())
}

/// Preview an interactive BOM without caching it or switching the session.
#[tauri::command(rename = "preview_interactive_bom")]
pub fn preview_interactive_bom_command(
    source_path: String,
    companion_csv_path: Option<String>,
) -> Result<ImportPreview, String> {
    let companion_path = companion_csv_path.as_deref().map(std::path::Path::new);
    preview_interactive_bom(std::path::Path::new(&source_path), companion_path)
        .map(ImportPreview::Ready)
        .map_err(|error| error.user_message())
}

#[tauri::command(rename = "resolve_bom_selection")]
pub fn resolve_bom_selection(
    message: BomSelectionMessage,
    database: State<'_, Mutex<Database>>,
    runtime: State<'_, Mutex<InteractiveBomRuntime>>,
) -> Result<ResolvedSelection, String> {
    // The bridge contract is enforced here rather than trusting the webview
    // pre-validation: an unknown `type`, oversized payload, or duplicate
    // designator never reaches the session table.
    validate_selection_message(&message).map_err(|error| error.user_message())?;
    let database = database.lock().map_err(|_| "数据库锁不可用".to_owned())?;
    let runtime = runtime
        .lock()
        .map_err(|_| "BOM 缓存状态锁不可用".to_owned())?;
    runtime
        .resolve_bom_selection(&database, &message.token, &message.designators)
        .map_err(|error: BridgeError| error.user_message())
}

#[tauri::command(rename = "restore_active_welding_session")]
pub fn restore_active_welding_session(
    database: State<'_, Mutex<Database>>,
    runtime: State<'_, Mutex<InteractiveBomRuntime>>,
) -> Result<Option<CachedBomSession>, String> {
    let database = database.lock().map_err(|_| "数据库锁不可用".to_owned())?;
    let runtime = runtime
        .lock()
        .map_err(|_| "BOM 缓存状态锁不可用".to_owned())?;
    runtime
        .restore_active_session(&database)
        .map_err(|error| error.user_message())
}

/// Read the columns the summary shows but the record does not carry.
fn summary(database: &Database, record: &ProjectRecord) -> Result<ProjectSummary, rusqlite::Error> {
    let (created_at, active): (String, bool) = database.connection().query_row(
        "SELECT p.created_at, EXISTS(SELECT 1 FROM welding_sessions s WHERE s.project_id = p.id AND s.status = 'active') FROM projects p WHERE p.id = ?1",
        [&record.id],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    Ok(ProjectSummary::from_record(record, created_at, active))
}

/// Arguments for adding the source a project still lacks.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct SupplementProjectInput {
    pub project_id: String,
    pub source_path: String,
    pub mapping: Option<FieldMapping>,
}

/// Complete a project with the source it was imported without: the parts table
/// of an interactive project, or the interactive export of a table project.
#[tauri::command(rename = "supplement_project")]
pub fn supplement_project(
    input: SupplementProjectInput,
    database: State<'_, Mutex<Database>>,
    runtime: State<'_, Mutex<InteractiveBomRuntime>>,
) -> Result<ImportedProject, String> {
    let database = database.lock().map_err(|_| "数据库锁不可用".to_owned())?;
    let runtime = runtime
        .lock()
        .map_err(|_| "BOM 缓存状态锁不可用".to_owned())?;
    runtime
        .supplement_project(
            &database,
            &input.project_id,
            std::path::Path::new(&input.source_path),
            input.mapping.as_ref(),
        )
        .map_err(|error| error.user_message())
}
