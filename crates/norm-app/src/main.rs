#![cfg_attr(
    all(not(debug_assertions), target_os = "windows"),
    windows_subsystem = "windows"
)]

use serde::{Deserialize, Serialize};
use std::fs::File;
use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::Mutex;
use tauri::{Emitter, Manager, State};

use norm_core::doc::DocId;
use norm_core::oplog::{DeviceId, VaultKey};
use norm_core::workspace::Workspace;

#[derive(Serialize, Deserialize, Default)]
struct AppConfig {
    vault_path: Option<String>,
}

fn get_config_path() -> PathBuf {
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home).join(".normnote_config.json")
}

fn read_config() -> AppConfig {
    if let Ok(content) = std::fs::read_to_string(get_config_path()) {
        if let Ok(config) = serde_json::from_str(&content) {
            return config;
        }
    }
    AppConfig::default()
}

fn save_config(config: &AppConfig) {
    if let Ok(content) = serde_json::to_string_pretty(config) {
        let _ = std::fs::write(get_config_path(), content);
    }
}

fn get_vault_path() -> PathBuf {
    let config = read_config();
    if let Some(path) = config.vault_path {
        PathBuf::from(path)
    } else {
        let home = std::env::var("HOME")
            .or_else(|_| std::env::var("USERPROFILE"))
            .unwrap_or_else(|_| ".".to_string());
        PathBuf::from(home).join("Documents").join("NormNoteVault")
    }
}
#[derive(Serialize)]
struct NoteItem {
    id: String,
    title: String,
    preview: String,
    updated_at: u64,
    tags: Vec<String>,
}

struct AppState {
    workspace: Mutex<Workspace>,
}

fn init_workspace() -> Workspace {
    // For the UI prototype, we use a fixed vault in the Documents directory.
    let root = get_vault_path();
    if !root.exists() {
        std::fs::create_dir_all(&root).unwrap();
    }

    // We just use a fixed key and device ID for now.
    let key = VaultKey::new([42u8; 32]);
    let device = DeviceId::from_hex("00000000000000000000000000000001").unwrap();

    let mut ws = Workspace::open(device, key, &root).expect("Failed to open workspace");
    // Ensure any external files are loaded
    ws.pull_from_disk().unwrap();
    ws
}

#[tauri::command]
fn get_notes(state: State<AppState>) -> Result<Vec<NoteItem>, String> {
    let mut ws = state.workspace.lock().unwrap();
    ws.pull_from_disk().map_err(|e| e.to_string())?;

    let mut items = Vec::new();
    let live_notes = ws.replica().live_notes().map_err(|e| e.to_string())?;

    for id in live_notes {
        if let Ok(Some(text)) = ws.replica().text(&id) {
            let lines: Vec<&str> = text.lines().collect();
            let title = lines.first().unwrap_or(&"").to_string();
            let preview = if lines.len() > 1 {
                lines[1].to_string()
            } else {
                "".to_string()
            };

            let mut tags = Vec::new();
            for word in text.split_whitespace() {
                if word.starts_with('#') && word.len() > 1 {
                    let tag = word.trim_matches(|c: char| !c.is_alphanumeric() && c != '_');
                    if !tag.is_empty() {
                        tags.push(tag.to_string());
                    }
                }
            }
            tags.sort();
            tags.dedup();

            items.push(NoteItem {
                id: id.to_string(),
                title: title.replace("# ", "").trim().to_string(),
                preview: preview.trim().to_string(),
                updated_at: 0,
                tags,
            });
        }
    }

    // Sort by id for now (in a real app, sort by updated_at)
    items.sort_by(|a, b| b.id.cmp(&a.id));
    Ok(items)
}

#[tauri::command]
fn read_note(id: String, state: State<AppState>) -> Result<String, String> {
    let ws = state.workspace.lock().unwrap();
    let doc_id = DocId::from_relative_path(std::path::Path::new(&id));
    let text = ws.replica().text(&doc_id).map_err(|e| e.to_string())?;
    Ok(text.unwrap_or_default())
}

#[tauri::command]
fn save_note(id: String, content: String, state: State<AppState>) -> Result<(), String> {
    let mut ws = state.workspace.lock().unwrap();
    let doc_id = DocId::from_relative_path(std::path::Path::new(&id));

    ws.replica_mut()
        .write(&doc_id, &content)
        .map_err(|e| e.to_string())?;
    ws.push_to_disk().map_err(|e| e.to_string())?;

    Ok(())
}

#[tauri::command]
fn delete_note(id: String, state: State<AppState>) -> Result<(), String> {
    let mut ws = state.workspace.lock().unwrap();
    let doc_id = DocId::from_relative_path(std::path::Path::new(&id));

    // Save to trash first
    if let Ok(Some(text)) = ws.replica().text(&doc_id) {
        if !text.is_empty() {
            let _ = norm_core::trash::save(ws.vault().root(), &doc_id, &text);
        }
    }

    ws.replica_mut()
        .delete(&doc_id)
        .map_err(|e| e.to_string())?;
    ws.push_to_disk().map_err(|e| e.to_string())?;

    // Remove the file from disk explicitly since push_to_disk handles tombstoned notes
    let file_path = ws.vault().path_of(&doc_id);
    if file_path.exists() {
        let _ = std::fs::remove_file(file_path);
    }

    Ok(())
}

#[tauri::command]
fn export_note_dialog(
    format: String,
    title: String,
    md_content: String,
    txt_content: String,
    html_content: String,
) -> Result<(), String> {
    let mut dialog = rfd::FileDialog::new().set_file_name(&title);

    if format == "html" {
        dialog = dialog.add_filter("HTML (*.html)", &["html"]);
    } else if format == "txt" {
        dialog = dialog.add_filter("Text (*.txt)", &["txt"]);
    } else {
        dialog = dialog.add_filter("Markdown (*.md)", &["md"]);
    }

    if let Some(path) = dialog.save_file() {
        let ext = format.as_str();

        // Convert relative .assets paths to absolute file URIs so images work externally
        let absolute_assets = get_vault_path().join(".assets");
        let uri = format!("file://{}/", absolute_assets.display());
        let updated_md = md_content.replace("](.assets/", &format!("]({}", uri));
        let updated_html = html_content.replace("src=\".assets/", &format!("src=\"{}", uri));

        let content = match ext {
            "txt" => txt_content,
            "html" => updated_html,
            _ => updated_md,
        };
        std::fs::write(path, content).map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[tauri::command]
fn export_pdf_dialog(title: String, base64_data: String) -> Result<(), String> {
    let dialog = rfd::FileDialog::new()
        .set_file_name(&title)
        .add_filter("PDF (*.pdf)", &["pdf"]);

    if let Some(path) = dialog.save_file() {
        // Strip the data URI prefix if it exists (e.g. "data:application/pdf;base64,")
        let b64 = if let Some(idx) = base64_data.find("base64,") {
            &base64_data[idx + 7..]
        } else {
            &base64_data
        };

        use base64::{engine::general_purpose::STANDARD, Engine as _};
        let bytes = STANDARD.decode(b64).map_err(|e| e.to_string())?;
        std::fs::write(path, bytes).map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[tauri::command]
fn import_files_dialog(
    folder: String,
    filter_type: String,
    state: State<AppState>,
) -> Result<Vec<String>, String> {
    let mut dialog = rfd::FileDialog::new();

    if filter_type == "md" {
        dialog = dialog.add_filter("Markdown Files", &["md"]);
    } else if filter_type == "img" {
        dialog = dialog.add_filter("Images", &["png", "jpg", "jpeg", "gif", "webp", "svg"]);
    } else if filter_type == "txt" {
        dialog = dialog.add_filter("Text Files", &["txt", "csv", "json", "log"]);
    } else {
        dialog = dialog.add_filter("All Files", &["*"]);
    }

    if let Some(paths) = dialog.pick_files() {
        let mut ws = state.workspace.lock().unwrap();
        let mut imported = Vec::new();

        for path in paths {
            let timestamp = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_millis();

            let is_image = filter_type == "img"
                || matches!(
                    path.extension().and_then(|s| s.to_str()).unwrap_or(""),
                    "png" | "jpg" | "jpeg" | "gif" | "webp" | "svg"
                );

            if is_image {
                let ext = path.extension().and_then(|s| s.to_str()).unwrap_or("png");
                let asset_filename = format!("{}.{}", timestamp, ext);
                let ws_path = get_vault_path();
                let assets_dir = ws_path.join(".assets");
                std::fs::create_dir_all(&assets_dir).unwrap_or_default();
                let dest = assets_dir.join(&asset_filename);
                if std::fs::copy(&path, &dest).is_ok() {
                    let stem = path
                        .file_stem()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .to_string();
                    let content = format!(
                        "# {}\n\n![{}]({})\n",
                        stem,
                        stem,
                        format!("assets://localhost/{}", asset_filename)
                    );

                    let new_filename = format!("{}.md", timestamp);
                    let id = if folder.is_empty() {
                        new_filename
                    } else {
                        format!("{}/{}", folder, new_filename)
                    };
                    let doc_id = DocId::from_relative_path(std::path::Path::new(&id));

                    if ws.replica_mut().write(&doc_id, &content).is_ok() {
                        imported.push(id);
                    }
                }
            } else {
                if let Ok(content) = std::fs::read_to_string(&path) {
                    let stem = path
                        .file_stem()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .to_string();
                    let is_markdown = path.extension().map(|e| e == "md").unwrap_or(false);

                    let mut final_content = content;
                    if !is_markdown && !final_content.starts_with('#') {
                        final_content = format!("# {}\n\n{}", stem, final_content);
                    }

                    let new_filename = format!("{}.md", timestamp);
                    let id = if folder.is_empty() {
                        new_filename
                    } else {
                        format!("{}/{}", folder, new_filename)
                    };

                    let doc_id = DocId::from_relative_path(std::path::Path::new(&id));
                    if ws.replica_mut().write(&doc_id, &final_content).is_ok() {
                        imported.push(id);
                    }
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(2));
        }

        ws.push_to_disk().map_err(|e| e.to_string())?;
        return Ok(imported);
    }

    Ok(Vec::new())
}

#[tauri::command]
fn import_image_dialog(_state: State<AppState>) -> Result<String, String> {
    if let Some(path) = rfd::FileDialog::new()
        .add_filter("Images", &["png", "jpg", "jpeg", "gif", "webp", "svg"])
        .pick_file()
    {
        let root = get_vault_path();
        let assets_dir = root.join(".assets");
        if !assets_dir.exists() {
            std::fs::create_dir_all(&assets_dir).map_err(|e| e.to_string())?;
        }

        let extension = path.extension().and_then(|e| e.to_str()).unwrap_or("png");
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis();

        let new_filename = format!("image_{}.{}", timestamp, extension);
        let target_path = assets_dir.join(&new_filename);

        std::fs::copy(&path, &target_path).map_err(|e| e.to_string())?;

        return Ok(format!(".assets/{}", new_filename));
    }
    Err("No file selected".to_string())
}

#[tauri::command]
fn import_image_asset(file_path: String, _state: State<AppState>) -> Result<String, String> {
    let source_path = PathBuf::from(&file_path);
    if !source_path.exists() || !source_path.is_file() {
        return Err("Invalid file path".to_string());
    }

    // We hardcode the vault root for now as Documents/NormNoteVault
    let root = get_vault_path();
    let assets_dir = root.join(".assets");
    if !assets_dir.exists() {
        std::fs::create_dir_all(&assets_dir).map_err(|e| e.to_string())?;
    }

    let extension = source_path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("png");

    // Use timestamp for uniqueness
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis();

    let new_filename = format!("image_{}.{}", timestamp, extension);
    let target_path = assets_dir.join(&new_filename);

    std::fs::copy(&source_path, &target_path).map_err(|e| e.to_string())?;

    // Return relative path for markdown
    Ok(format!(".assets/{}", new_filename))
}

#[tauri::command]
fn import_image_bytes(
    bytes: Vec<u8>,
    ext: String,
    _state: State<AppState>,
) -> Result<String, String> {
    let root = get_vault_path();
    let assets_dir = root.join(".assets");
    if !assets_dir.exists() {
        std::fs::create_dir_all(&assets_dir).map_err(|e| e.to_string())?;
    }

    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis();

    let new_filename = format!("image_{}.{}", timestamp, ext);
    let target_path = assets_dir.join(&new_filename);

    std::fs::write(&target_path, bytes).map_err(|e| e.to_string())?;

    Ok(format!(".assets/{}", new_filename))
}

#[tauri::command]
fn read_image_bytes(path: String) -> Result<Vec<u8>, String> {
    let root = get_vault_path();
    // Only allow reading from .assets to prevent directory traversal
    if !path.starts_with(".assets/") {
        return Err("Invalid path".to_string());
    }
    let target = root.join(&path);
    std::fs::read(&target).map_err(|e| e.to_string())
}

#[tauri::command]
fn backup_vault() -> Result<String, String> {
    if let Some(path) = rfd::FileDialog::new()
        .set_file_name("norm_vault_backup.zip")
        .add_filter("Zip Archive", &["zip"])
        .save_file()
    {
        let file = File::create(&path).map_err(|e| e.to_string())?;
        let mut zip = zip::ZipWriter::new(file);

        #[allow(deprecated)]
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);

        let vault_dir = get_vault_path();
        if !vault_dir.exists() {
            return Err("Vault not found".into());
        }

        for entry in walkdir::WalkDir::new(&vault_dir) {
            let entry = entry.map_err(|e| e.to_string())?;
            let path = entry.path();
            let name = path.strip_prefix(&vault_dir).unwrap().to_str().unwrap();

            if name.is_empty() {
                continue;
            }

            if path.is_file() {
                #[allow(deprecated)]
                zip.start_file(name, options).map_err(|e| e.to_string())?;
                let mut f = File::open(path).map_err(|e| e.to_string())?;
                let mut buffer = Vec::new();
                f.read_to_end(&mut buffer).map_err(|e| e.to_string())?;
                zip.write_all(&buffer).map_err(|e| e.to_string())?;
            } else {
                #[allow(deprecated)]
                zip.add_directory(name, options)
                    .map_err(|e| e.to_string())?;
            }
        }

        zip.finish().map_err(|e| e.to_string())?;
        return Ok(path.to_string_lossy().into_owned());
    }
    Err("Cancelled".into())
}

#[tauri::command]
fn restore_vault(state: State<AppState>) -> Result<(), String> {
    if let Some(path) = rfd::FileDialog::new()
        .add_filter("Zip Archive", &["zip"])
        .pick_file()
    {
        let file = File::open(&path).map_err(|e| e.to_string())?;
        let mut archive = zip::ZipArchive::new(file).map_err(|e| e.to_string())?;

        let vault_dir = get_vault_path();
        if !vault_dir.exists() {
            std::fs::create_dir_all(&vault_dir).map_err(|e| e.to_string())?;
        }

        for i in 0..archive.len() {
            let mut file = archive.by_index(i).map_err(|e| e.to_string())?;
            let outpath = match file.enclosed_name() {
                Some(path) => vault_dir.join(path),
                None => continue,
            };

            if (*file.name()).ends_with('/') {
                std::fs::create_dir_all(&outpath).map_err(|e| e.to_string())?;
            } else {
                if let Some(p) = outpath.parent() {
                    if !p.exists() {
                        std::fs::create_dir_all(p).map_err(|e| e.to_string())?;
                    }
                }
                let mut outfile = File::create(&outpath).map_err(|e| e.to_string())?;
                std::io::copy(&mut file, &mut outfile).map_err(|e| e.to_string())?;
            }
        }

        // Refresh workspace
        let mut ws = state.workspace.lock().unwrap();
        ws.pull_from_disk().map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[cfg(target_os = "macos")]
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem, Submenu};
#[cfg(target_os = "macos")]
use tauri::AppHandle;

#[cfg(target_os = "macos")]
fn build_menu(
    app: &AppHandle,
    recent_notes: &[(String, String)],
) -> tauri::Result<Menu<tauri::Wry>> {
    let new_note = MenuItem::with_id(app, "new-note", "New Note", true, Some("CmdOrCtrl+N"))?;
    let new_folder = MenuItem::with_id(
        app,
        "new-folder",
        "New Folder",
        true,
        Some("CmdOrCtrl+Shift+N"),
    )?;
    let rename_note = MenuItem::with_id(app, "rename-note", "Rename Note...", true, None::<&str>)?;
    let duplicate_note = MenuItem::with_id(
        app,
        "duplicate-note",
        "Duplicate Note",
        true,
        Some("CmdOrCtrl+D"),
    )?;
    let delete_note = MenuItem::with_id(
        app,
        "delete-note",
        "Delete Note",
        true,
        Some("CmdOrCtrl+Backspace"),
    )?;
    let import_md =
        MenuItem::with_id(app, "import-files:md", "Markdown (.md)", true, None::<&str>)?;
    let import_img = MenuItem::with_id(
        app,
        "import-files:img",
        "Images (.png, .jpg)",
        true,
        None::<&str>,
    )?;
    let import_txt = MenuItem::with_id(
        app,
        "import-files:txt",
        "Text (.txt, .csv)",
        true,
        None::<&str>,
    )?;
    let import_all = MenuItem::with_id(app, "import-files:all", "All Files", true, None::<&str>)?;
    let import_submenu = Submenu::with_items(
        app,
        "Import Files...",
        true,
        &[&import_md, &import_img, &import_txt, &import_all],
    )?;

    let export_md = MenuItem::with_id(app, "export-note:md", "Markdown (.md)", true, None::<&str>)?;
    let export_pdf = MenuItem::with_id(app, "export-note:pdf", "PDF (.pdf)", true, None::<&str>)?;
    let export_html =
        MenuItem::with_id(app, "export-note:html", "HTML (.html)", true, None::<&str>)?;
    let export_txt = MenuItem::with_id(app, "export-note:txt", "Text (.txt)", true, None::<&str>)?;
    let export_submenu = Submenu::with_items(
        app,
        "Export Note...",
        true,
        &[&export_md, &export_pdf, &export_html, &export_txt],
    )?;
    let backup_vault =
        MenuItem::with_id(app, "backup-vault", "Backup Vault...", true, None::<&str>)?;
    let restore_vault =
        MenuItem::with_id(app, "restore-vault", "Restore Vault...", true, None::<&str>)?;
    let settings = MenuItem::with_id(app, "settings", "Settings...", true, Some("CmdOrCtrl+,"))?;

    let app_menu = Submenu::with_items(
        app,
        "NormNote",
        true,
        &[
            &PredefinedMenuItem::about(app, None, None)?,
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

    // Build the "Open Recent" submenu dynamically
    let recent_submenu = Submenu::new(app, "Open Recent", true)?;
    if recent_notes.is_empty() {
        let no_recent =
            MenuItem::with_id(app, "no-recent", "No Recent Notes", false, None::<&str>)?;
        let _ = recent_submenu.append(&no_recent);
    } else {
        for (id, name) in recent_notes {
            let note_item =
                MenuItem::with_id(app, format!("open-recent:{}", id), name, true, None::<&str>)?;
            let _ = recent_submenu.append(&note_item);
        }
    }

    let file_menu = Submenu::with_items(
        app,
        "File",
        true,
        &[
            &new_note,
            &new_folder,
            &recent_submenu,
            &PredefinedMenuItem::separator(app)?,
            &rename_note,
            &duplicate_note,
            &delete_note,
            &PredefinedMenuItem::separator(app)?,
            &import_submenu,
            &export_submenu,
            &PredefinedMenuItem::separator(app)?,
            &backup_vault,
            &restore_vault,
        ],
    )?;

    let edit_menu = Submenu::with_items(
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

    let toggle_sidebar = MenuItem::with_id(
        app,
        "toggle-sidebar",
        "Toggle Sidebar",
        true,
        Some("CmdOrCtrl+Option+S"),
    )?;
    let zoom_in = MenuItem::with_id(app, "zoom-in", "Zoom In", true, Some("CmdOrCtrl+="))?;
    let zoom_out = MenuItem::with_id(app, "zoom-out", "Zoom Out", true, Some("CmdOrCtrl+-"))?;
    let actual_size =
        MenuItem::with_id(app, "actual-size", "Actual Size", true, Some("CmdOrCtrl+0"))?;
    let toggle_fullscreen = MenuItem::with_id(
        app,
        "toggle-fullscreen",
        "Toggle Full Screen",
        true,
        Some("CmdOrCtrl+Ctrl+F"),
    )?;

    let view_menu = Submenu::with_items(
        app,
        "View",
        true,
        &[
            &toggle_sidebar,
            &PredefinedMenuItem::separator(app)?,
            &zoom_in,
            &zoom_out,
            &actual_size,
            &PredefinedMenuItem::separator(app)?,
            &toggle_fullscreen,
        ],
    )?;

    let window_zoom = MenuItem::with_id(app, "window-zoom", "Zoom", true, None::<&str>)?;
    let show_notes_window =
        MenuItem::with_id(app, "show-notes-window", "Notes", true, Some("CmdOrCtrl+1"))?;
    let window_menu = Submenu::with_items(
        app,
        "Window",
        true,
        &[
            &PredefinedMenuItem::minimize(app, None)?,
            &window_zoom,
            &PredefinedMenuItem::separator(app)?,
            &show_notes_window,
            &PredefinedMenuItem::separator(app)?,
            &PredefinedMenuItem::close_window(app, None)?,
        ],
    )?;

    let help_item = MenuItem::with_id(app, "help-doc", "NormNote Help", true, None::<&str>)?;
    let help_menu = Submenu::with_items(app, "Help", true, &[&help_item])?;
    let _ = help_menu.set_as_help_menu_for_nsapp();

    Menu::with_items(
        app,
        &[
            &app_menu,
            &file_menu,
            &edit_menu,
            &view_menu,
            &window_menu,
            &help_menu,
        ],
    )
}

#[tauri::command]
fn update_recent_menu(app: tauri::AppHandle, recent_notes: Vec<(String, String)>) {
    #[cfg(target_os = "macos")]
    {
        if let Ok(menu) = build_menu(&app, &recent_notes) {
            let _ = app.set_menu(menu);
        }
    }
}

#[tauri::command]
fn get_current_vault_path() -> Result<String, String> {
    Ok(get_vault_path().to_string_lossy().into_owned())
}

#[tauri::command]
fn choose_vault_location_dialog() -> Result<String, String> {
    if let Some(path) = rfd::FileDialog::new().pick_folder() {
        return Ok(path.to_string_lossy().into_owned());
    }
    Err("Cancelled".into())
}

#[tauri::command]
fn set_vault_location(path: String, state: State<AppState>) -> Result<(), String> {
    let mut config = read_config();
    config.vault_path = Some(path);
    save_config(&config);

    // Reinitialize workspace with new path
    let workspace = init_workspace();
    let mut ws = state.workspace.lock().unwrap();
    *ws = workspace;

    Ok(())
}

fn main() {
    let workspace = init_workspace();

    tauri::Builder::default()
        .manage(AppState {
            workspace: Mutex::new(workspace),
        })
        .invoke_handler(tauri::generate_handler![
            get_notes,
            save_note,
            read_note,
            delete_note,
            export_note_dialog,
            export_pdf_dialog,
            import_files_dialog,
            import_image_dialog,
            import_image_asset,
            import_image_bytes,
            read_image_bytes,
            backup_vault,
            restore_vault,
            update_recent_menu,
            get_current_vault_path,
            choose_vault_location_dialog,
            set_vault_location
        ])
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(|app, _shortcut, event| {
                    if event.state == tauri_plugin_global_shortcut::ShortcutState::Pressed {
                        let _ = app.emit("quick-capture", ());
                        if let Some(window) = app.get_webview_window("main") {
                            let _ = window.unminimize();
                            let _ = window.show();
                            let _ = window.set_focus();
                        }
                    }
                })
                .build(),
        )
        .plugin(tauri_plugin_updater::Builder::new().build())
        .setup(|app| {
            #[cfg(target_os = "macos")]
            {

                let initial_recent: Vec<(String, String)> = vec![];
                if let Ok(menu) = build_menu(app.handle(), &initial_recent) {
                    let _ = app.set_menu(menu);
                }
            }
            #[cfg(desktop)]
            {
                use tauri_plugin_global_shortcut::{Code, GlobalShortcutExt, Modifiers, Shortcut};
                let shortcut = Shortcut::new(Some(Modifiers::CONTROL | Modifiers::SHIFT), Code::KeyN);
                let _ = app.global_shortcut().register(shortcut);
            }
            Ok(())
        })
        .on_menu_event(|app, event| {
            let id = event.id().as_ref();
            std::fs::write("/Users/adchanapong/Desktop/menu_rust.log", format!("Menu clicked: {}", id)).ok();
            if id == "toggle-fullscreen" {
                if let Some(window) = app.get_webview_window("main") {
                    let is_fs = window.is_fullscreen().unwrap_or(false);
                    let _ = window.set_fullscreen(!is_fs);
                }
            } else if id == "window-zoom" {
                if let Some(window) = app.get_webview_window("main") {
                    let is_max = window.is_maximized().unwrap_or(false);
                    if is_max {
                        let _ = window.unmaximize();
                    } else {
                        let _ = window.maximize();
                    }
                }
            } else if id == "show-notes-window" {
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.show();
                    let _ = window.unminimize();
                    let _ = window.set_focus();
                }
            } else {
                if let Some(window) = app.get_webview_window("main") {
                    let script = match id {
                        "new-note" => "document.getElementById('new-note-btn')?.click();",
                        "new-folder" => "document.getElementById('new-folder-btn')?.click();",
                        "import-files:md" => "document.querySelector('#import-dropdown .dropdown-item[data-format=\"md\"]')?.click();",
                        "import-files:img" => "document.querySelector('#import-dropdown .dropdown-item[data-format=\"img\"]')?.click();",
                        "import-files:txt" => "document.querySelector('#import-dropdown .dropdown-item[data-format=\"txt\"]')?.click();",
                        "import-files:all" => "document.querySelector('#import-dropdown .dropdown-item[data-format=\"all\"]')?.click();",
                        "export-note:md" => "document.querySelector('#export-dropdown .dropdown-item[data-format=\"md\"]')?.click();",
                        "export-note:pdf" => "document.querySelector('#export-dropdown .dropdown-item[data-format=\"pdf\"]')?.click();",
                        "export-note:html" => "document.querySelector('#export-dropdown .dropdown-item[data-format=\"html\"]')?.click();",
                        "export-note:txt" => "document.querySelector('#export-dropdown .dropdown-item[data-format=\"txt\"]')?.click();",
                        "settings" => "document.getElementById('open-settings-btn')?.click();",
                        "export-note" => "document.getElementById('export-btn')?.click();",
                        "delete-note" => "document.querySelector('.note-item.active .del-note-btn')?.click();",
                        "rename-note" => "document.querySelector('.note-item.active .rename-btn')?.click();",
                        "duplicate-note" => "document.querySelector('.note-item.active .dup-note-btn')?.click();",
                        "toggle-sidebar" => "const s = document.querySelector('.sidebar'); if(s) s.classList.toggle('collapsed');",
                        "backup-vault" => "document.getElementById('backup-vault-btn')?.click();",
                        "restore-vault" => "document.getElementById('restore-vault-btn')?.click();",
                        "zoom-in" => "document.body.style.zoom = parseFloat(document.body.style.zoom || 1) + 0.1;",
                        "zoom-out" => "document.body.style.zoom = Math.max(0.5, parseFloat(document.body.style.zoom || 1) - 0.1);",
                        "actual-size" => "document.body.style.zoom = 1;",
                        "help-doc" => "if (window.showHelpModal) { window.showHelpModal(); }",
                        _ if id.starts_with("open-recent:") => {
                            // Note: we can't easily format! here if we want a static str in match arms. Let's do it dynamically.
                            "/* handled dynamically */"
                        },
                        _ => ""
                    };
                    let mut final_script = script.to_string();
                    if id.starts_with("open-recent:") {
                        let note_id = id.trim_start_matches("open-recent:");
                        final_script = format!("if (window.selectNote) window.selectNote('{}');", note_id);
                    }
                    if !final_script.is_empty() {
                        let res = window.eval(&final_script);
                        std::fs::write(format!("/Users/adchanapong/Desktop/menu_eval_{}.log", id.replace(":", "_")), format!("script: {}\nres: {:?}", final_script, res)).ok();
                    }
                }
            }
        })
        .on_window_event(|window, event| match event {
            tauri::WindowEvent::CloseRequested { api, .. } => {
                #[cfg(target_os = "macos")]
                {
                    let _ = window.hide();
                    api.prevent_close();
                }
            }
            _ => {}
        })
        .build(tauri::generate_context!())
        .expect("error while running tauri application")
        .run(|app_handle, event| match event {
            #[cfg(target_os = "macos")]
            tauri::RunEvent::Reopen { .. } => {
                if let Some(window) = app_handle.get_webview_window("main") {
                    let _ = window.show();
                    let _ = window.set_focus();
                }
            }
            _ => {}
        });
}
