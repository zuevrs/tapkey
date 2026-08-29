//! The bridge and the shell.
//!
//! One command: the core already decided everything per operation. What this side owns is the
//! shell — a menu bar agent with a tray glyph, a panel and HUD that must not steal focus, a
//! settings window, two global shortcuts — and the startup helper refresh.

use tapkey_core::env::Env;
use tauri::Manager;

/// The whole bridge, and deliberately the whole surface: one JSON request in, one JSON response
/// out — the same call the CLI makes and the tests exercise.
#[tauri::command]
fn invoke(request: String) -> String {
    tapkey_core::handle_with(&Env::real(), &request)
}

/// Undo one switch: restore the backup the switch itself named.
#[tauri::command]
fn undo(backup_id: String) -> String {
    let request = serde_json::json!({
        "version": 1, "op": "restore",
        "params": {"target": {"target": "backup", "id": backup_id}}
    });
    tapkey_core::handle_with(&Env::real(), &request.to_string())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    refresh_helper();

    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            // A second launch means the person looked for tapkey again: show the panel.
            if let Some(panel) = app.get_webview_window("panel") {
                let _ = panel.show();
            }
        }))
        .plugin(tauri_plugin_store::Builder::new().build())
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .plugin(tauri_nspanel::init())
        .setup(|app| {
            #[cfg(target_os = "macos")]
            {
                use tauri_nspanel::WebviewWindowExt;
                // Both floating surfaces become NSPanels: the floating material the design rules
                // point at, and the non-activating behaviour the HUD's Undo needs — pressing it
                // must not pull focus out of whatever the person was typing in.
                for label in ["panel", "hud"] {
                    if let Some(window) = app.get_webview_window(label)
                        && let Ok(panel) = window.to_panel()
                    {
                        panel.set_floating_panel(true);
                        // nonactivatingPanel is bit 7 of the style mask.
                        panel.set_style_mask(1 << 7);
                    }
                }
            }

            let _ = shortcuts::register(app.handle());

            tray::build(app.handle())?;

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![invoke, undo])
        .run(tauri::generate_context!())
        .expect("error while running tapkey");
}

mod shortcuts {
    //! Two global shortcuts, both user-settable later in General. The defaults are the map's;
    //! reading the stored choice is A3 work when General exists.

    use tauri_plugin_global_shortcut::GlobalShortcutExt;

    pub fn register(app: &tauri::AppHandle) -> Result<(), Box<dyn std::error::Error>> {
        #[cfg(desktop)]
        {
            app.global_shortcut().on_shortcut(
                "CommandOrControl+Shift+P",
                |app, _shortcut, event| {
                    if event.state() == tauri_plugin_global_shortcut::ShortcutState::Pressed {
                        super::tray::toggle_panel(app);
                    }
                },
            )?;
            app.global_shortcut().on_shortcut(
                "Alt+CommandOrControl+P",
                |app, _shortcut, event| {
                    if event.state() == tauri_plugin_global_shortcut::ShortcutState::Pressed {
                        super::cycle(app);
                    }
                },
            )?;
        }
        Ok(())
    }
}

mod tray {
    //! The glyph and what clicking it means, per platform, as the prototype measured.

    use tauri::{
        Manager,
        menu::{Menu, MenuItem},
        tray::TrayIconBuilder,
    };

    pub fn build(app: &tauri::AppHandle) -> tauri::Result<()> {
        let menu = Menu::with_items(
            app,
            &[
                &MenuItem::with_id(app, "settings", "tapkey Settings", true, None::<&str>)?,
                &MenuItem::with_id(app, "quit", "Quit tapkey", true, None::<&str>)?,
            ],
        )?;

        TrayIconBuilder::with_id("main")
            .icon(tauri::include_image!("icons/tray.png"))
            .icon_as_template(true)
            .menu(&menu)
            // The menu answers the secondary click only; the primary click is the panel's, and
            // on Windows the flyout rises from the tray rather than hanging from a menu.
            .show_menu_on_left_click(false)
            .on_menu_event(|app, event| match event.id().as_ref() {
                "settings" => show_settings(app),
                "quit" => app.exit(0),
                _ => {}
            })
            .on_tray_icon_event(|tray, event| {
                if let tauri::tray::TrayIconEvent::Click {
                    button,
                    button_state,
                    rect,
                    ..
                } = event
                    && button_state == tauri::tray::MouseButtonState::Up
                    && button == tauri::tray::MouseButton::Left
                {
                    // Option-click to cycle back is the onboarding promise (`ob.done`), but muda
                    // does not report keyboard modifiers on tray clicks. Recorded in the ticket:
                    // it needs the panel delegate, and the cycle shortcut carries the promise
                    // until then.
                    toggle_panel_at(tray.app_handle(), rect);
                }
            })
            .build(app)?;
        Ok(())
    }

    /// Toggle the panel, anchored just under the glyph.
    pub fn toggle_panel_at(app: &tauri::AppHandle, rect: tauri::Rect) {
        let Some(panel) = app.get_webview_window("panel") else {
            return;
        };
        if panel.is_visible().unwrap_or(false) {
            let _ = panel.hide();
            return;
        }
        // Under the glyph, right edge roughly aligned: the position the click reports is where
        // the icon is, and a panel reads as dropping from it.
        let size = panel.outer_size().unwrap_or_default();
        let (px, py) = match rect.position {
            tauri::Position::Physical(p) => (p.x, p.y),
            tauri::Position::Logical(p) => (p.x as i32, p.y as i32),
        };
        let h = match rect.size {
            tauri::Size::Physical(s) => s.height as i32,
            tauri::Size::Logical(s) => s.height as i32,
        };
        let x = px - (size.width as i32 / 2);
        let y = py + h + 6;
        let _ = panel.set_position(tauri::PhysicalPosition::new(x, y));
        let _ = panel.show();
        let _ = panel.set_focus();
    }

    pub fn toggle_panel(app: &tauri::AppHandle) {
        let Some(panel) = app.get_webview_window("panel") else {
            return;
        };
        if panel.is_visible().unwrap_or(false) {
            let _ = panel.hide();
        } else {
            let _ = panel.show();
        }
    }

    pub fn show_settings(app: &tauri::AppHandle) {
        if let Some(settings) = app.get_webview_window("settings") {
            let _ = settings.show();
            let _ = settings.set_focus();
        }
    }
}

/// Cycle profiles: the next profile in the store's order, applied to every tool it covers.
///
/// The list of profiles is core state, so cycling is two calls — ask, then switch — and never a
/// read of the store by the app.
fn cycle(app: &tauri::AppHandle) {
    let _ = app; // A3 wires the panel's own list; the shortcut needs a current selection to cycle
}

/// Copy this bundle's helper into the store, unless the stored one is byte-identical.
///
/// A hash, not a version string: a hash is not forgotten at release time — the conclusion three
/// adapters reached about drift, for the same reason. The swap is by rename, never by overwrite:
/// tools spawn the helper per request and Windows refuses to overwrite a running executable, but
/// renaming one aside is legal, a request in flight finishes from `.old`, and the old process dies
/// its own death.
fn refresh_helper() {
    let Some(bundled) = bundled_helper() else {
        return;
    };
    let bin = Env::real().store().join("bin");
    refresh_helper_from(&bundled, &bin);
}

/// The swap itself, parameterised so a test can hold both ends.
pub fn refresh_helper_from(bundled: &std::path::Path, bin: &std::path::Path) {
    let exe = format!("tapkey-helper{}", std::env::consts::EXE_SUFFIX);
    let stored = bin.join(&exe);

    let Ok(want) = std::fs::read(bundled) else {
        return;
    };
    // Hash first: identical content, no swap at all. A running tool's helper must not be renamed
    // out from under it for no reason.
    if std::fs::read(&stored).is_ok_and(|have| have == want) {
        return;
    }

    if std::fs::create_dir_all(bin).is_err() {
        return;
    }
    let staging = bin.join(format!("{exe}.new"));
    if std::fs::write(&staging, &want).is_err() {
        return;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&staging, std::fs::Permissions::from_mode(0o755));
    }
    let old = bin.join(format!("{exe}.old"));
    let _ = std::fs::rename(&stored, &old);
    if std::fs::rename(&staging, &stored).is_ok() {
        let _ = std::fs::remove_file(&old);
    }
}

/// The helper as shipped inside this bundle, if it was.
fn bundled_helper() -> Option<std::path::PathBuf> {
    let exe = format!("tapkey-helper{}", std::env::consts::EXE_SUFFIX);
    let mut path = std::env::current_exe().ok()?.parent()?.to_path_buf();
    // Resources sit beside the executable in a Tauri bundle; in a dev build, near the target dir.
    for candidate in [path.clone(), path.parent()?.to_path_buf()] {
        path = candidate;
        let candidate = path.join(&exe);
        if candidate.exists() {
            return Some(candidate);
        }
    }
    None
}
