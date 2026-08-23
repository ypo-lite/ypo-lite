//! Helper for the applications' `build.rs`, behind the `build` feature.
//!
//! ```no_run
//! // y-snap/build.rs
//! fn main() {
//!     y_core::build::embed_resources("rsc/y-snap.rc");
//! }
//! ```
//!
//! `embed-resource` picks whichever resource compiler the active toolchain
//! provides (`rc.exe` from the Windows SDK, `llvm-rc`, or `windres`), so the
//! same resource script works for every Windows host.
//!
//! A hand-written `.rc` is preferred over the `winres`/`winresource` crates,
//! which generate the script for you: the moment an application adds a menu, a
//! dialog or a string table, it wants to own that file.

use std::path::Path;

/// Compiles `script` into the executable and keeps its version in sync with
/// `Cargo.toml`.
///
/// Also watches the files the script includes — `embed-resource` watches the
/// `.rc` itself, but not the icon or manifest it names, so editing an icon
/// would otherwise not trigger a rebuild.
pub fn embed_resources(script: &str) {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed={script}");
    for included in included_files(script) {
        println!("cargo:rerun-if-changed={included}");
    }

    // Keep `cargo check` usable when cross-compiling away from Windows.
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }

    assert_version_in_sync(script);

    // `manifest_required` turns a missing resource compiler into a hard error
    // instead of silently producing a binary without icon or manifest.
    embed_resource::compile(script, embed_resource::NONE)
        .manifest_required()
        .expect("failed to compile Windows resources");
}

/// The file names quoted in the script, resolved next to it: the icon, the
/// manifest, anything else it pulls in.
fn included_files(script: &str) -> Vec<String> {
    let Ok(text) = std::fs::read_to_string(script) else {
        return Vec::new();
    };
    let directory = Path::new(script).parent().unwrap_or(Path::new("."));

    text.lines()
        .filter_map(|line| {
            let quoted = line.split('"').nth(1)?;
            let path = directory.join(quoted);
            path.is_file().then(|| path.display().to_string())
        })
        .collect()
}

/// Fails the build when `FILEVERSION` in the resource script drifts away from
/// the version in `Cargo.toml` — the two are edited in different files and
/// otherwise diverge silently.
fn assert_version_in_sync(script: &str) {
    let text = std::fs::read_to_string(script)
        .unwrap_or_else(|error| panic!("cannot read {script}: {error}"));

    let declared = text
        .lines()
        .find_map(|line| line.trim().strip_prefix("FILEVERSION"))
        .map(|value| {
            value
                .split(',')
                .take(3)
                .map(|part| part.trim().to_owned())
                .collect::<Vec<_>>()
                .join(".")
        })
        .unwrap_or_else(|| panic!("no FILEVERSION found in {script}"));

    // The build script's own crate version is `y-core`'s; the application's
    // arrives through the environment Cargo sets for the build script.
    let expected = std::env::var("CARGO_PKG_VERSION").expect("CARGO_PKG_VERSION");
    let expected = expected.split('.').take(3).collect::<Vec<_>>().join(".");

    assert_eq!(
        declared, expected,
        "version mismatch: {script} declares {declared}, Cargo.toml declares {expected} \
         (update FILEVERSION, PRODUCTVERSION and the version strings in the .rc)"
    );
}
