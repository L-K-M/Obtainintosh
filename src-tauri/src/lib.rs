mod commands;
mod installer;
mod models;
mod sources;
mod storage;
mod system_colors;
mod updates;

use commands::AppState;
use std::sync::Arc;
use tauri::{Emitter, Manager};

// Menu construction is macOS-only, so the imports are too — on every other
// target no menu is built at all and the types would be unused, failing
// clippy's `-D warnings`.
#[cfg(target_os = "macos")]
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem, Submenu};

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.unminimize();
                let _ = window.show();
                let _ = window.set_focus();
            }
        }))
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            let storage = storage::Storage::new().expect("Failed to initialize storage");
            app.manage(AppState {
                storage: Arc::new(storage),
                active_download: Arc::new(Default::default()),
            });

            // macOS keeps the native menu bar at the top of the screen, where
            // it carries the standard app, Edit, and Window menus. Everywhere
            // else the menu only duplicates UI the window already has — GTK
            // draws it inside the window, Windows puts a native menu bar
            // under the title bar — so no menu is set there at all. The
            // actions it carried live on the toolbar (which gains an About
            // button), and the keyboard shortcuts are handled by the webview
            // frontend.
            #[cfg(target_os = "macos")]
            {
                // Create custom menu items
                let add_app =
                    MenuItem::with_id(app, "add_app", "Add App...", true, Some("CmdOrCtrl+N"))?;
                let check_all =
                    MenuItem::with_id(app, "check_all", "Check All", true, Some("CmdOrCtrl+R"))?;
                let settings =
                    MenuItem::with_id(app, "settings", "Settings", true, Some("CmdOrCtrl+,"))?;

                // Create custom About menu item (instead of
                // PredefinedMenuItem::about) so the app's own dialog opens.
                let about =
                    MenuItem::with_id(app, "about", "About Obtainintosh", true, None::<&str>)?;

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

                let menu = Menu::with_items(app, &[&app_submenu, &edit_submenu, &window_submenu])?;

                app.set_menu(menu)?;
            }

            Ok(())
        })
        .on_menu_event(|app, event| match event.id().as_ref() {
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
            updates::open_release_url,
            commands::get_system_colors,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
