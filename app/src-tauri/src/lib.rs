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

/// Route one switch result to the HUD window: it drives itself from the query parameters, and
/// the panel never touches another window's content.
#[tauri::command]
fn onboarding_done(app: tauri::AppHandle) {
    crate::onboarding::mark_done(&app);
}

#[tauri::command]
fn show_sheet(app: tauri::AppHandle, sheet: String) -> tauri::Result<()> {
    // Sheets are windows the shell owns; the panel never touches another window's content, it
    // asks the shell to show one.
    let label = match sheet.as_str() {
        "effective" => "effective",
        "history" => "history",
        _ => return Ok(()),
    };
    if let Some(window) = app.get_webview_window(label) {
        window.show()?;
        window.set_focus()?;
    }
    Ok(())
}

#[tauri::command]
fn show_hud(app: tauri::AppHandle, response_json: String, backup_id: String) -> tauri::Result<()> {
    let hud = app
        .get_webview_window("hud")
        .expect("the hud window exists");
    // Built on the window's own absolute URL: `Url::parse` refuses a relative path, and the
    // live pass watched every switch kill the app on exactly that line — the command had
    // never once run outside the gate harness.
    let mut url = hud.url()?;
    let query = url::form_urlencoded::Serializer::new(String::new())
        .append_pair("response", &response_json)
        .append_pair("backup", &backup_id)
        .finish();
    url.set_query(Some(&query));
    // `navigate`, not `eval`: an eval'd `location.replace` never fired on a webview that had
    // not been shown, and the live pass watched the HUD stay empty through a whole switch.
    hud.navigate(url)?;
    hud.show()
}

/// Open at login, through the autostart plugin — the one General toggle that needs the OS.
#[tauri::command]
fn set_autostart(app: tauri::AppHandle, enabled: bool) -> Result<bool, String> {
    use tauri_plugin_autostart::ManagerExt as _;
    let autostart = app.autolaunch();
    if enabled {
        autostart.enable().map_err(|e| e.to_string())?;
    } else {
        autostart.disable().map_err(|e| e.to_string())?;
    }
    autostart.is_enabled().map_err(|e| e.to_string())
}

#[tauri::command]
fn get_autostart(app: tauri::AppHandle) -> Result<bool, String> {
    use tauri_plugin_autostart::ManagerExt as _;
    app.autolaunch().is_enabled().map_err(|e| e.to_string())
}

/// Run setup again: the flag goes, the onboarding window returns — the same flow the first
/// run took, nothing special about it.
#[tauri::command]
fn run_setup_again(app: tauri::AppHandle) {
    use tauri_plugin_store::StoreExt as _;
    if let Ok(store) = app.store("prefs.json") {
        store.delete("onboarded");
    }
    if let Some(window) = app.get_webview_window("onboarding") {
        let _ = window.show();
        let _ = window.set_focus();
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    refresh_helper();

    let builder = tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            // A second launch means the person looked for tapkey again: show the panel.
            if let Some(panel) = app.get_webview_window("panel") {
                let _ = panel.show();
            }
        }))
        .plugin(tauri_plugin_store::Builder::new().build())
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            None,
        ))
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .setup(|app| {
            // An accessory app: menu bar, no dock tile — the design-rules Surfaces section in
            // one line, at runtime, because the v2 bundler carries no plist-extension config.
            #[cfg(target_os = "macos")]
            app.set_activation_policy(tauri::ActivationPolicy::Accessory);

            // The floating surfaces are ordinary always-on-top windows. The live pass measured
            // the alternative — NSPanel conversion through tauri-nspanel — as **display-only**:
            // the class swap takes the floating material and takes every input event with it,
            // no click and no keystroke ever reaching the webview, while a plain window takes
            // both the moment it appears. tapkey paints its own panel background from the token
            // layer instead of the OS material, which the design rules called "no glass of our
            // own" and reality overruled; the record lives in A11.

            // The floating material, done as a view rather than a window class: NSVisualEffectView
            // inserted **below** the webview of each floating surface. The live pass measured the
            // other route — converting the window to an NSPanel — as display-only, its class swap
            // taking every input event with it; a background view leaves the window and its input
            // alone, and the CSS tint then sits over the system's own blur.
            #[cfg(target_os = "macos")]
            for label in ["panel", "hud"] {
                if let Some(window) = app.get_webview_window(label) {
                    // Popover material, active: the menubar-flyout glass macOS itself draws,
                    // behind the webview — no window-class swap (measured display-only).
                    let _ = window_vibrancy::apply_vibrancy(
                        &window,
                        window_vibrancy::NSVisualEffectMaterial::Popover,
                        Some(window_vibrancy::NSVisualEffectState::Active),
                        None,
                    );
                }
            }

            let _ = shortcuts::register(app.handle());

            tray::build(app.handle())?;

            onboarding::maybe_show(app.handle());

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            invoke,
            show_hud,
            show_sheet,
            onboarding_done,
            set_autostart,
            get_autostart,
            run_setup_again
        ]);

    builder
        .run(tauri::generate_context!())
        .expect("error while running tapkey");
}

mod onboarding {
    //! First run only: the flag lives in the app's own preferences — never in the core's store,
    //! which ADR-0019 gave to the engine. "Set up later" sets the same flag with the catalogue's
    //! honest consequence.

    use tauri::Manager;
    use tauri_plugin_store::StoreExt;

    pub fn maybe_show(app: &tauri::AppHandle) {
        let Ok(store) = app.store("prefs.json") else {
            return;
        };
        if store.get("onboarded").is_some() {
            return;
        }
        if let Some(window) = app.get_webview_window("onboarding") {
            let _ = window.show();
            let _ = window.set_focus();
        }
    }

    pub fn mark_done(app: &tauri::AppHandle) {
        let Ok(store) = app.store("prefs.json") else {
            return;
        };
        store.set("onboarded", true);
        if let Some(window) = app.get_webview_window("onboarding") {
            let _ = window.close();
        }
    }
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
                |_app, _shortcut, event| {
                    if event.state() == tauri_plugin_global_shortcut::ShortcutState::Pressed {
                        super::cycle();
                    }
                },
            )?;
            // The footer's two openings are the prototype's shortcuts, and a shortcut is a
            // promise the footer visibly makes — ⌘Y and ⌘, are the macOS conventions they ride.
            app.global_shortcut()
                .on_shortcut("CommandOrControl+Y", |app, _shortcut, event| {
                    if event.state() == tauri_plugin_global_shortcut::ShortcutState::Pressed {
                        let for_main = app.clone();
                        let _ = app.run_on_main_thread(move || {
                            super::show_sheet(for_main, "history".into()).ok();
                        });
                    }
                })?;
            app.global_shortcut()
                .on_shortcut("CommandOrControl+,", |app, _shortcut, event| {
                    if event.state() == tauri_plugin_global_shortcut::ShortcutState::Pressed {
                        let for_main = app.clone();
                        let _ = app.run_on_main_thread(move || {
                            super::tray::show_settings(&for_main);
                        });
                    }
                })?;
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

        // The glyph, per platform, as the design rules draw it: Apple's SF Symbol as a
        // black+alpha template on macOS, where the system tints it; the copper mark itself
        // on Windows, where templates do not exist and a black glyph would vanish on a dark
        // taskbar.
        #[cfg(target_os = "macos")]
        let glyph = tauri::include_image!("icons/tray.png");
        #[cfg(not(target_os = "macos"))]
        let glyph = tauri::include_image!("icons/32x32.png");

        TrayIconBuilder::with_id("main")
            .icon(glyph)
            .icon_as_template(cfg!(target_os = "macos"))
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

    /// Show the panel and make it the thing keystrokes go to. `set_focus` alone sets the first
    /// responder without making the window key or activating this accessory application, so
    /// system keystrokes keep going to whatever was frontmost — the live pass watched "glm"
    /// and Enter do exactly nothing until this existed.
    /// Show the panel and make it the thing keystrokes go to. The AppKit half runs on the
    /// main thread — a shortcut handler may arrive elsewhere, and AppKit calls from off the
    /// main thread are silently dropped, which is exactly how a panel that looks focused can
    /// keep sending keystrokes to whatever was frontmost. The crate's `show` is the whole
    /// sequence: first responder to the webview, front regardless, and **key window** — the
    /// one call that was missing, without which a nonactivating panel floats but never types.
    fn show_and_key(app: &tauri::AppHandle) {
        let Some(panel) = app.get_webview_window("panel") else {
            return;
        };
        let _ = panel.show();
        let _ = panel.set_focus();
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
        show_and_key(app);
    }

    pub fn toggle_panel(app: &tauri::AppHandle) {
        let Some(panel) = app.get_webview_window("panel") else {
            return;
        };
        if panel.is_visible().unwrap_or(false) {
            let _ = panel.hide();
        } else {
            show_and_key(app);
        }
    }

    pub fn show_settings(app: &tauri::AppHandle) {
        if let Some(settings) = app.get_webview_window("settings") {
            let _ = settings.show();
            let _ = settings.set_focus();
        }
    }
}

/// Cycle profiles: the next profile in the store's order. The list is core state, so cycling is
/// two bridge calls — ask, then switch — and never a read of the store by the app. What is
/// currently in effect decides where "next" starts; the response's chains name it.
fn cycle() {
    let env = Env::real();
    let list = tapkey_core::handle_with(&env, r#"{"version":1,"op":"list_profiles","params":{}}"#);
    let state =
        tapkey_core::handle_with(&env, r#"{"version":1,"op":"effective_state","params":{}}"#);
    let Ok(list) = serde_json::from_str::<serde_json::Value>(&list) else {
        return;
    };
    let Ok(state) = serde_json::from_str::<serde_json::Value>(&state) else {
        return;
    };
    let Some(rows) = list["profiles"].as_array() else {
        return;
    };
    if rows.len() < 2 {
        return;
    }
    // The first owned slot value per tool is what the tool will use; the id whose row it matches
    // is the current selection. Nothing matches — start at the top.
    let now = state["tools"].as_array().and_then(|tools| {
        tools.iter().find_map(|tool| {
            tool["slots"].as_array().and_then(|slots| {
                slots
                    .iter()
                    .find(|s| s["owned"] == true)
                    .and_then(|s| s["resolved"]["effective"].as_str().map(str::to_owned))
            })
        })
    });
    let at = now.and_then(|value| {
        rows.iter().position(|row| {
            row["name"].as_str() == Some(value.as_str())
                || value.contains(row["name"].as_str().unwrap_or_default())
        })
    });
    let next = rows[(at.unwrap_or(rows.len().saturating_sub(1)) + 1) % rows.len()]["id"]
        .as_str()
        .unwrap_or_default()
        .to_string();
    let request = format!(r#"{{"version":1,"op":"switch","params":{{"profile_id":"{next}"}}}}"#);
    let _ = tapkey_core::handle_with(&env, &request);
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
    // externalBin lands the helper beside the app executable in a bundle; in a dev build it sits
    // in the same target dir as this binary.
    let dir = std::env::current_exe().ok()?.parent()?.to_path_buf();
    let beside = dir.join(&exe);
    if beside.exists() {
        return Some(beside);
    }
    dir.parent()
        .map(|target| target.join(&exe))
        .filter(|p| p.exists())
}
