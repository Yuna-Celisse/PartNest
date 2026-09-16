use super::{lock_error, CommandError};
use crate::{backup, bom::cache::InteractiveBomRuntime, db::Database};
use std::path::PathBuf;
use std::sync::Mutex;
use tauri::{AppHandle, Manager, State};

/// Back up first, then clear business records atomically, preserving migrations.
pub fn reset_data_service(
    database: &Database,
    runtime: &InteractiveBomRuntime,
    backup_directory: &std::path::Path,
) -> Result<String, CommandError> {
    let path = backup::create_backup(database, backup_directory)?;
    let tx = database.transaction()?;
    tx.execute_batch(
        "PRAGMA defer_foreign_keys = ON;
         DELETE FROM inventory_movements;
         DELETE FROM welding_progress;
         DELETE FROM welding_sessions;
         DELETE FROM bom_files;
         DELETE FROM parts;
         DELETE FROM boxes;
         DELETE FROM lcsc_cache;",
    )?;
    runtime
        .invalidate_active_session()
        .map_err(|error| CommandError::Database(error.to_string()))?;
    tx.commit()?;
    Ok(path.to_string_lossy().into_owned())
}

#[tauri::command(rename = "reset_data")]
pub fn reset_data(
    app: AppHandle,
    state: State<'_, Mutex<Database>>,
    runtime: State<'_, Mutex<InteractiveBomRuntime>>,
) -> Result<String, CommandError> {
    let directory = app_data_directory(&app)?;
    let database = state.lock().map_err(lock_error)?;
    let runtime = runtime.lock().map_err(lock_error)?;
    reset_data_service(&database, &runtime, &backup::backup_dir(directory))
}

fn app_data_directory(app: &AppHandle) -> Result<PathBuf, CommandError> {
    app.path()
        .app_data_dir()
        .map_err(|error| CommandError::Database(format!("无法定位应用数据目录: {error}")))
}

#[tauri::command(rename = "create_backup")]
pub fn create_backup(
    app: AppHandle,
    state: State<'_, Mutex<Database>>,
) -> Result<String, CommandError> {
    let app_data_directory = app_data_directory(&app)?;
    let database = state.lock().map_err(lock_error)?;
    let path = backup::create_backup(&database, backup::backup_dir(app_data_directory))?;
    Ok(path.to_string_lossy().into_owned())
}

#[tauri::command(rename = "restore_backup")]
pub fn restore_backup(
    app: AppHandle,
    backup_path: String,
    state: State<'_, Mutex<Database>>,
    runtime: State<'_, Mutex<InteractiveBomRuntime>>,
) -> Result<(), CommandError> {
    if backup_path.trim().is_empty() {
        return Err(CommandError::Validation("请选择备份文件".into()));
    }
    let app_data_directory = app_data_directory(&app)?;
    let selected = PathBuf::from(backup_path);
    let live_path = app_data_directory.join("partnest.db");
    if selected == live_path {
        return Err(CommandError::Validation("不能从当前数据库文件恢复".into()));
    }
    let mut database = state.lock().map_err(lock_error)?;
    backup::restore_database_file(&mut database, selected)?;
    let runtime = runtime.lock().map_err(lock_error)?;
    runtime
        .invalidate_active_session()
        .map_err(|error| CommandError::Database(error.to_string()))?;
    Ok(())
}
