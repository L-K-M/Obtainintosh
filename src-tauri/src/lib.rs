mod commands;
mod models;
mod system_colors;
mod sources;
mod storage;
mod installer;
mod updates;

use commands::AppState;
use std::sync::Arc;
use tauri::{
    menu::{Menu, MenuItem, PredefinedMenuItem, Submenu},
    Emitter, Manager,
};

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // Initialize storage
    let storage = storage::Storage::new().expect("Failed to initialize storage");
    let app_state = AppState {
        storage: Arc::new(storage),
    };

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .manage(app_state)
        .setup(|app| {
            // Create custom menu items
            let add_app = MenuItem::with_id(app, "add_app", "Add App...", true, Some("CmdOrCtrl+N"))?;
            let check_all = MenuItem::with_id(app, "check_all", "Check All", true, Some("CmdOrCtrl+R"))?;
            let settings = MenuItem::with_id(app, "settings", "Settings", true, Some("CmdOrCtrl+,"))?;

            // Create custom About menu item (instead of PredefinedMenuItem::about)
            let about = MenuItem::with_id(app, "about", "About Obtainintosh", true, None::<&str>)?;

            // Create the app submenu (Obtainintosh menu)
            let app_submenu = Submenu::with_items(
                app,
                "Obtainintosh",
                true,
                &[
                    &about,
                    &PredefinedMenuItem::separator(app)?,
                    &add_app,
                    &check_all,
                    &PredefinedMenuItem::separator(app)?,
                    &settings,
                    &PredefinedMenuItem::separator(app)?,
                    &PredefinedMenuItem::services(app, None)?,
                    &PredefinedMenuItem::separator(app)?,
                    &PredefinedMenuItem::hide(app, None)?,
                    &PredefinedMenuItem::hide_others(app, None)?,
                    &PredefinedMenuItem::show_all(app, None)?,
                    &PredefinedMenuItem::separator(app)?,
                    &PredefinedMenuItem::quit(app, None)?,
                ],
            )?;

            // Create Edit menu for standard edit operations
            let edit_submenu = Submenu::with_items(
                app,
                "Edit",
                true,
                &[
                    &PredefinedMenuItem::undo(app, None)?,
                    &PredefinedMenuItem::redo(app, None)?,
                    &PredefinedMenuItem::separator(app)?,
                    &PredefinedMenuItem::cut(app, None)?,
                    &PredefinedMenuItem::copy(app, None)?,
                    &PredefinedMenuItem::paste(app, None)?,
                    &PredefinedMenuItem::select_all(app, None)?,
                ],
            )?;

            // Create Window menu
            let window_submenu = Submenu::with_items(
                app,
                "Window",
                true,
                &[
                    &PredefinedMenuItem::minimize(app, None)?,
                    &PredefinedMenuItem::maximize(app, None)?,
                    &PredefinedMenuItem::separator(app)?,
                    &PredefinedMenuItem::close_window(app, None)?,
                ],
            )?;

            // Build the menu
            let menu = Menu::with_items(app, &[&app_submenu, &edit_submenu, &window_submenu])?;
            app.set_menu(menu)?;

            Ok(())
        })
        .on_menu_event(|app, event| {
            match event.id().as_ref() {
                "add_app" => {
                    if let Some(window) = app.get_webview_window("main") {
                        let _ = window.emit("menu-add-app", ());
                    }
                }
                "check_all" => {
                    if let Some(window) = app.get_webview_window("main") {
                        let _ = window.emit("menu-check-all", ());
                    }
                }
                "settings" => {
                    if let Some(window) = app.get_webview_window("main") {
                        let _ = window.emit("menu-settings", ());
                    }
                }
                "about" => {
                    if let Some(window) = app.get_webview_window("main") {
                        let _ = window.emit("menu-about", ());
                    }
                }
                _ => {}
            }
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_all_apps,
            commands::add_app,
            commands::update_app,
            commands::remove_app,
            commands::check_for_updates,
            commands::download_and_install,
            commands::get_settings,
            commands::update_settings,
            updates::check_self_update,
            commands::get_system_colors,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
