use std::path::Path;
use std::process::Command;

/// Detect if an app is installed in /Applications or ~/Applications and get its version
pub fn detect_installed_app(app_name: &str) -> Option<(String, String)> {
    log::debug!("Searching for app: {}", app_name);

    let mut app_dirs = vec!["/Applications".to_string()];
    if let Some(home) = dirs::home_dir() {
        app_dirs.push(home.join("Applications").to_string_lossy().to_string());
    }

    for dir in &app_dirs {
        // Try exact match first
        let exact_path = format!("{}/{}.app", dir, app_name);
        log::debug!("Trying exact path: {}", exact_path);

        if let Some(version) = get_app_version(&exact_path) {
            log::debug!("Found via exact match! Version: {}", version);
            return Some((exact_path, version));
        }

        // Try case-insensitive search
        log::debug!("Exact match failed, trying case-insensitive search in {}", dir);
        if let Ok(entries) = std::fs::read_dir(dir) {
            for entry in entries.filter_map(|e| e.ok()) {
                let path = entry.path();
                if path.extension().and_then(|s| s.to_str()) == Some("app") {
                    if let Some(file_name) = path.file_stem().and_then(|s| s.to_str()) {
                        // Case-insensitive comparison
                        if file_name.to_lowercase() == app_name.to_lowercase() {
                            log::debug!("Match found: {}", file_name);
                            if let Some(version) = get_app_version(&path.to_string_lossy()) {
                                log::debug!("Version found: {}", version);
                                return Some((path.to_string_lossy().to_string(), version));
                            }
                        }
                    }
                }
            }
        }
    }

    log::debug!("No match found for {}", app_name);
    None
}

/// Get version from an installed .app bundle
fn get_app_version(app_path: &str) -> Option<String> {
    let plist_path = format!("{}/Contents/Info.plist", app_path);
    
    if !Path::new(&plist_path).exists() {
        return None;
    }
    
    // Use PlistBuddy to read version
    let output = Command::new("/usr/libexec/PlistBuddy")
        .args(&["-c", "Print :CFBundleShortVersionString", &plist_path])
        .output()
        .ok()?;
    
    if output.status.success() {
        let version = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if !version.is_empty() {
            return Some(version);
        }
    }
    
    // Fallback to CFBundleVersion
    let output = Command::new("/usr/libexec/PlistBuddy")
        .args(&["-c", "Print :CFBundleVersion", &plist_path])
        .output()
        .ok()?;
    
    if output.status.success() {
        let version = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if !version.is_empty() {
            return Some(version);
        }
    }
    
    None
}
