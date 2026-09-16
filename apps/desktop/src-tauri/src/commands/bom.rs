use crate::{
    bom::{
        bridge::{validate_selection_message, BomSelectionMessage, BridgeError},
        cache::{
            preview_interactive_bom, CachedBomSession, InteractiveBomRuntime, ResolvedSelection,
        },
        history::{load_import, record_import, remove_import, BomImportKind},
        tabular::inspect_tabular_bom as inspect_tabular_bom_file,
        types::{FieldMapping, ImportPreview, NormalizedBomDto},
    },
    db::Database,
};
use serde::Serialize;
use std::sync::Mutex;
use tauri::State;

#[derive(Debug, Clone, Serialize)]
pub struct BomFileSummary {
    pub id: String,
    pub display_name: String,
    pub original_name: String,
    pub created_at: String,
    pub active: bool,
}

#[tauri::command(rename = "list_bom_files")]
pub fn list_bom_files(database: State<'_, Mutex<Database>>) -> Result<Vec<BomFileSummary>, String> {
    let database = database.lock().map_err(|_| "数据库锁不可用".to_owned())?;
    let mut stmt = database.connection().prepare("SELECT f.id, f.display_name, f.original_name, f.created_at, EXISTS(SELECT 1 FROM welding_sessions s WHERE s.bom_file_id = f.id AND s.status = 'active') FROM bom_files f ORDER BY f.created_at DESC").map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |row| {
            Ok(BomFileSummary {
                id: row.get(0)?,
                display_name: row.get(1)?,
                original_name: row.get(2)?,
                created_at: row.get(3)?,
                active: row.get(4)?,
            })
        })
        .map_err(|e| e.to_string())?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())
}

#[tauri::command(rename = "inspect_tabular_bom")]
pub fn inspect_tabular_bom(
    source_path: String,
    mapping: Option<FieldMapping>,
    display_name: Option<String>,
    database: State<'_, Mutex<Database>>,
) -> Result<ImportPreview, String> {
    let preview = inspect_tabular_bom_file(&source_path, mapping.as_ref())
        .map_err(|error| error.to_string())?;
    if let ImportPreview::Ready(normalized) = &preview {
        record_history(
            &database,
            &source_path,
            display_name.as_deref(),
            BomImportKind::Tabular,
            normalized,
        );
    }
    Ok(preview)
}

/// Analyze an interactive BOM without caching it as an active session. The
/// analysis still records the file in the imported-BOM history.
#[tauri::command(rename = "preview_interactive_bom")]
pub fn preview_interactive_bom_command(
    source_path: String,
    companion_csv_path: Option<String>,
    display_name: Option<String>,
    database: State<'_, Mutex<Database>>,
) -> Result<ImportPreview, String> {
    let companion_path = companion_csv_path.as_deref().map(std::path::Path::new);
    let normalized = preview_interactive_bom(std::path::Path::new(&source_path), companion_path)
        .map_err(|error| error.user_message())?;
    record_history(
        &database,
        &source_path,
        display_name.as_deref(),
        BomImportKind::Interactive,
        &normalized,
    );
    Ok(ImportPreview::Ready(normalized))
}

/// Reopen an imported BOM from its stored analysis snapshot.
#[tauri::command(rename = "analyze_bom_file")]
pub fn analyze_bom_file(
    id: String,
    database: State<'_, Mutex<Database>>,
) -> Result<ImportPreview, String> {
    let database = database.lock().map_err(|_| "数据库锁不可用".to_owned())?;
    load_import(&database, &id)
        .map(ImportPreview::Ready)
        .map_err(|error| error.user_message())
}

/// Activate an analysed BOM as the welding session. Interactive BOMs are
/// re-cached from their source file; tabular ones open a snapshot-backed
/// session without any cache copy.
#[tauri::command(rename = "activate_imported_bom")]
pub fn activate_imported_bom(
    id: Option<String>,
    source_path: Option<String>,
    display_name: Option<String>,
    database: State<'_, Mutex<Database>>,
    runtime: State<'_, Mutex<InteractiveBomRuntime>>,
) -> Result<CachedBomSession, String> {
    let database = database.lock().map_err(|_| "数据库锁不可用".to_owned())?;
    let runtime = runtime
        .lock()
        .map_err(|_| "BOM 缓存状态锁不可用".to_owned())?;
    runtime
        .activate_imported_bom(
            &database,
            id.as_deref(),
            source_path.as_deref().map(std::path::Path::new),
            display_name.as_deref(),
        )
        .map_err(|error| error.user_message())
}

/// Remove an imported BOM record from the history.
#[tauri::command(rename = "remove_bom_file")]
pub fn remove_bom_file(
    id: String,
    database: State<'_, Mutex<Database>>,
    runtime: State<'_, Mutex<InteractiveBomRuntime>>,
) -> Result<(), String> {
    let database = database.lock().map_err(|_| "数据库锁不可用".to_owned())?;
    let runtime = runtime
        .lock()
        .map_err(|_| "BOM 缓存状态锁不可用".to_owned())?;
    remove_import(&database, runtime.cache_dir(), &id).map_err(|error| error.user_message())
}

/// History is a convenience, not a precondition: a failed registration must
/// never block the analysis result from reaching the UI.
fn record_history(
    database: &State<'_, Mutex<Database>>,
    source_path: &str,
    display_name: Option<&str>,
    kind: BomImportKind,
    normalized: &NormalizedBomDto,
) {
    let stored = database
        .lock()
        .map_err(|_| "数据库锁不可用".to_owned())
        .and_then(|database| {
            record_import(
                &database,
                std::path::Path::new(source_path),
                display_name,
                kind,
                normalized,
            )
            .map_err(|error| error.user_message())
        });
    if let Err(error) = stored {
        eprintln!("PartNest BOM import history failed: {error}");
    }
}

#[tauri::command(rename = "cache_interactive_bom")]
pub fn cache_interactive_bom(
    source_path: String,
    display_name: String,
    companion_csv_path: Option<String>,
    database: State<'_, Mutex<Database>>,
    runtime: State<'_, Mutex<InteractiveBomRuntime>>,
) -> Result<CachedBomSession, String> {
    let database = database.lock().map_err(|_| "数据库锁不可用".to_owned())?;
    let runtime = runtime
        .lock()
        .map_err(|_| "BOM 缓存状态锁不可用".to_owned())?;
    let companion_path = companion_csv_path.as_deref().map(std::path::Path::new);
    runtime
        .cache_interactive_bom_with_companion(&database, source_path, display_name, companion_path)
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

#[tauri::command(rename = "restore_active_interactive_bom")]
pub fn restore_active_interactive_bom(
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
