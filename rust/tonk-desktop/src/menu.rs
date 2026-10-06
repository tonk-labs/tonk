//! The macOS menu bar.
//!
//! A macOS app is expected to have one, and the webview depends on it: the
//! system sends copy, paste and the other editing commands through the Edit
//! menu's items, so a window without them gets no clipboard at all.

use muda::accelerator::Accelerator;
use muda::{AboutMetadata, Menu, MenuId, MenuItem, PredefinedMenuItem, Submenu};

/// The installed menu bar. It must live as long as the app.
pub struct MenuBar {
    _menu: Menu,
    reload: MenuItem,
    inspect: MenuItem,
}

/// A menu command the app handles itself.
pub enum Command {
    /// Load the page again.
    Reload,
    /// Open the Web Inspector.
    Inspect,
}

impl MenuBar {
    /// The command `id` names, if it is one of ours. The predefined items
    /// (copy, quit, …) are handled by the system and never reach here.
    pub fn command(&self, id: &MenuId) -> Option<Command> {
        if id == self.reload.id() {
            Some(Command::Reload)
        } else if id == self.inspect.id() {
            Some(Command::Inspect)
        } else {
            None
        }
    }
}

/// Build the menu bar and install it as the app's.
///
/// Call once the app has started, on the main thread.
pub fn install() -> muda::Result<MenuBar> {
    let shortcut = |keys: &str| keys.parse::<Accelerator>().ok();
    let reload = MenuItem::new("Reload", true, shortcut("CmdOrCtrl+R"));
    let inspect = MenuItem::new("Web Inspector", true, shortcut("CmdOrCtrl+Alt+I"));
    let separator = PredefinedMenuItem::separator();
    let about = AboutMetadata {
        name: Some("Tonk".to_owned()),
        version: Some(env!("CARGO_PKG_VERSION").to_owned()),
        ..AboutMetadata::default()
    };

    let app = Submenu::with_items(
        "Tonk",
        true,
        &[
            &PredefinedMenuItem::about(None, Some(about)),
            &separator,
            &PredefinedMenuItem::services(None),
            &separator,
            &PredefinedMenuItem::hide(None),
            &PredefinedMenuItem::hide_others(None),
            &PredefinedMenuItem::show_all(None),
            &separator,
            &PredefinedMenuItem::quit(None),
        ],
    )?;
    let edit = Submenu::with_items(
        "Edit",
        true,
        &[
            &PredefinedMenuItem::undo(None),
            &PredefinedMenuItem::redo(None),
            &separator,
            &PredefinedMenuItem::cut(None),
            &PredefinedMenuItem::copy(None),
            &PredefinedMenuItem::paste(None),
            &PredefinedMenuItem::select_all(None),
        ],
    )?;
    let view = Submenu::with_items(
        "View",
        true,
        &[
            &reload,
            &inspect,
            &separator,
            &PredefinedMenuItem::fullscreen(None),
        ],
    )?;
    let window = Submenu::with_items(
        "Window",
        true,
        &[
            &PredefinedMenuItem::minimize(None),
            &PredefinedMenuItem::maximize(None),
            &separator,
            &PredefinedMenuItem::close_window(None),
        ],
    )?;
    let menu = Menu::with_items(&[&app, &edit, &view, &window])?;
    menu.init_for_nsapp();
    Ok(MenuBar {
        _menu: menu,
        reload,
        inspect,
    })
}
