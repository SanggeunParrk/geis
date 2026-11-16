//! The application menu.
//!
//! The window draws its own controls, but a Mac app is also expected to answer from the
//! menu bar: Settings under the app's name with Cmd+comma, Open under File, Undo under
//! Edit. The items that are the app's own carry an id and are sent to the window as a
//! `menu` event - what each one does is decided there, where the commands live, so a
//! menu item and the button for the same thing cannot drift apart.

use tauri::menu::{AboutMetadata, Menu, MenuItem, PredefinedMenuItem, Submenu};
use tauri::{AppHandle, Emitter, Runtime};

/// The ids the window is sent. Anything else in the menu is the system's own.
const OWN: &[&str] = &[
    "settings",
    "open",
    "fetch",
    "save.image",
    "undo",
    "redo",
    "palette",
    "zoom.all",
    "mode.toggle",
    "shortcuts",
];

pub fn build<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<Menu<R>> {
    let item =
        |id: &str, text: &str, keys: Option<&str>| MenuItem::with_id(app, id, text, true, keys);
    let separator = || PredefinedMenuItem::separator(app);

    let about = AboutMetadata {
        name: Some(app.package_info().name.clone()),
        version: Some(app.package_info().version.to_string()),
        ..Default::default()
    };

    let application = Submenu::with_items(
        app,
        app.package_info().name.clone(),
        true,
        &[
            &PredefinedMenuItem::about(app, None, Some(about))?,
            &separator()?,
            &item("settings", "Settings...", Some("CmdOrCtrl+,"))?,
            &separator()?,
            &PredefinedMenuItem::services(app, None)?,
            &separator()?,
            &PredefinedMenuItem::hide(app, None)?,
            &PredefinedMenuItem::hide_others(app, None)?,
            &separator()?,
            &PredefinedMenuItem::quit(app, None)?,
        ],
    )?;

    let file = Submenu::with_items(
        app,
        "File",
        true,
        &[
            &item("open", "Open...", Some("CmdOrCtrl+O"))?,
            &item("fetch", "Fetch from the PDB...", Some("CmdOrCtrl+Shift+O"))?,
            &separator()?,
            &item("save.image", "Save Image...", Some("CmdOrCtrl+S"))?,
            &separator()?,
            &PredefinedMenuItem::close_window(app, None)?,
        ],
    )?;

    // Undo and Redo are the app's own rather than the system's: the system's act only on
    // the text field that has the keyboard, and here they also step back through what was
    // done to the scene. The window decides which is meant.
    let edit = Submenu::with_items(
        app,
        "Edit",
        true,
        &[
            &item("undo", "Undo", Some("CmdOrCtrl+Z"))?,
            &item("redo", "Redo", Some("CmdOrCtrl+Shift+Z"))?,
            &separator()?,
            &PredefinedMenuItem::cut(app, None)?,
            &PredefinedMenuItem::copy(app, None)?,
            &PredefinedMenuItem::paste(app, None)?,
            &PredefinedMenuItem::select_all(app, None)?,
        ],
    )?;

    let view = Submenu::with_items(
        app,
        "View",
        true,
        &[
            &item("palette", "Find a Command...", Some("CmdOrCtrl+K"))?,
            &separator()?,
            &item("zoom.all", "Zoom All", None)?,
            &item("mode.toggle", "Structure or Graph View", None)?,
            &separator()?,
            &PredefinedMenuItem::fullscreen(app, None)?,
        ],
    )?;

    let window = Submenu::with_items(
        app,
        "Window",
        true,
        &[
            &PredefinedMenuItem::minimize(app, None)?,
            &PredefinedMenuItem::maximize(app, None)?,
        ],
    )?;

    let help = Submenu::with_items(
        app,
        "Help",
        true,
        &[&item("shortcuts", "Keyboard Shortcuts", None)?],
    )?;

    Menu::with_items(app, &[&application, &file, &edit, &view, &window, &help])
}

/// The menu bar as text, one line per item: for checking, from outside, that the menu a
/// running app has is the one described here. Printed at launch when `GEIS_PRINT_MENU`
/// is set.
pub fn describe<R: Runtime>(menu: &Menu<R>) -> Vec<String> {
    use tauri::menu::MenuItemKind;
    let mut lines = Vec::new();
    for top in menu.items().unwrap_or_default() {
        let MenuItemKind::Submenu(submenu) = top else {
            continue;
        };
        let title = submenu.text().unwrap_or_default();
        for item in submenu.items().unwrap_or_default() {
            let line = match item {
                MenuItemKind::MenuItem(item) => format!(
                    "{title} > {} [{}]",
                    item.text().unwrap_or_default(),
                    item.id().as_ref()
                ),
                MenuItemKind::Predefined(item) => {
                    format!("{title} > {} (system)", item.text().unwrap_or_default())
                }
                _ => continue,
            };
            lines.push(line);
        }
    }
    lines
}

/// Passes one of the app's own items on to the window.
pub fn handle<R: Runtime>(app: &AppHandle<R>, id: &str) {
    if OWN.contains(&id) {
        let _ = app.emit("menu", id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_ids_sent_to_the_window_are_distinct() {
        for (index, id) in OWN.iter().enumerate() {
            assert!(!OWN[index + 1..].contains(id), "{id} is listed twice");
        }
    }
}
