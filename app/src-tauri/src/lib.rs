//! The bridge.
//!
//! One command. The core already decided everything per operation; a command layer that
//! re-validated per operation would be a second opinion growing where none is wanted. What the
//! app owns at startup lives here too: the store arrives from the core's own per-platform
//! resolution, and the helper is refreshed from this bundle into the store by content hash.

use tapkey_core::env::Env;

/// The whole bridge, and deliberately the whole surface: one JSON request in, one JSON response
/// out — the same call the CLI makes and the tests exercise.
#[tauri::command]
fn invoke(request: String) -> String {
    tapkey_core::handle_with(&Env::real(), &request)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    refresh_helper();
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![invoke])
        .run(tauri::generate_context!())
        .expect("error while running tapkey");
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
