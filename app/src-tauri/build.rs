fn main() {
    // Ship the credential helper beside the app binary: externalBin resolves at bundle time, and
    // the file it wants is the core's own build output. Copying here keeps the wiring in one
    // place and fails the build loudly if the helper did not build.
    let profile = std::env::var("PROFILE").unwrap_or_else(|_| "debug".into());
    let src = std::path::Path::new("../../target")
        .join(&profile)
        .join(format!("tapkey-helper{}", std::env::consts::EXE_SUFFIX));
    let dst_dir = std::path::Path::new("binaries");
    let _ = std::fs::create_dir_all(dst_dir);
    // externalBin resolves the per-target triple, not the bare name.
    let target = std::env::var("TARGET").unwrap_or_default();
    let dst = dst_dir.join(format!("tapkey-helper-{target}"));
    std::fs::copy(&src, &dst).unwrap_or_else(|_| {
        panic!(
            "the credential helper is not built for the {profile} profile yet — run \
             `cargo build --{profile} -p tapkey-core --bin tapkey-helper` first; the app bundles it \
             beside the executable, so the app does not build without it"
        )
    });
    println!("cargo:rerun-if-changed={}", src.display());
    tauri_build::build()
}
