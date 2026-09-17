pub mod backup;
pub mod bom;
pub mod commands;
pub mod db;

use bom::cache::InteractiveBomRuntime;
use db::Database;
use std::sync::Mutex;
use tauri::Manager;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let app_data_dir = app.path().app_data_dir()?;
            let database = Database::open_app_data_dir(&app_data_dir)?;
            if let Err(error) = backup::maybe_create_startup_backup(&database, &app_data_dir) {
                eprintln!("PartNest startup backup skipped: {error}");
            }
            app.manage(Mutex::new(database));
            app.manage(Mutex::new(InteractiveBomRuntime::new(
                app_data_dir.join("interactive-bom-cache"),
            )));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::projects::inspect_tabular_bom,
            commands::projects::preview_interactive_bom_command,
            commands::projects::list_projects,
            commands::projects::import_project,
            commands::projects::supplement_project,
            commands::projects::analyze_project,
            commands::projects::rename_project_command,
            commands::projects::remove_project_command,
            commands::projects::open_project_welding,
            commands::projects::resolve_bom_selection,
            commands::projects::restore_active_welding_session,
            commands::boxes::list_boxes,
            commands::boxes::create_box,
            commands::boxes::resize_box,
            commands::boxes::update_box,
            commands::boxes::delete_box,
            commands::parts::list_parts,
            commands::parts::create_part,
            commands::parts::update_part,
            commands::parts::adjust_stock,
            commands::parts::delete_part,
            commands::parts::lookup_lcsc,
            commands::movements::list_movements,
            commands::settings::create_backup,
            commands::settings::restore_backup,
            commands::settings::reset_data,
            commands::welding::confirm_take,
            commands::welding::end_welding_session,
            commands::welding::reverse_take,
            commands::welding::get_welding_progress,
        ])
        .run(tauri::generate_context!())
        .expect("error while running PartNest");
}
